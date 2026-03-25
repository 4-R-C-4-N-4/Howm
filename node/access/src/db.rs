use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::schema::{create_tables, seed_built_in_groups};
use crate::types::*;

/// Handle to the access control database.
///
/// Uses `Mutex<Connection>` so the struct is `Send + Sync` and can live
/// inside `Arc<AccessDb>` in an async runtime (tokio AppState).
pub struct AccessDb {
    conn: Mutex<Connection>,
}

impl AccessDb {
    /// Open (or create) the access database at `path`. Initialises schema and
    /// seeds built-in groups if this is a fresh database.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        create_tables(&conn)?;
        seed_built_in_groups(&conn)?;
        debug!("access.db opened at {}", path.display());
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open a read-only handle (for capability processes).
    pub fn open_readonly(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ── Permission resolution (hot path) ─────────────────────────────────

    /// Resolve whether `peer_id` is allowed to access `capability_name`.
    ///
    /// Logic (BRD §4.4):
    /// 1. Collect all groups the peer belongs to (explicit memberships).
    /// 2. Always include howm.default (implicit fallback).
    /// 3. If ANY group has allow=true for the capability → Allow.
    /// 4. Otherwise → Deny.
    pub fn resolve_permission(&self, peer_id: &[u8], capability_name: &str) -> PermissionResult {
        let conn = self.conn.lock();
        // Walk the parent_group_id chain so inherited capabilities resolve.
        // The CTE collects all direct membership groups + howm.default, then
        // recursively adds each group's parent.
        let result = conn.query_row(
            "WITH RECURSIVE member_groups(gid) AS (
                 SELECT group_id FROM peer_group_memberships WHERE peer_id = ?2
                 UNION
                 SELECT ?3
                 UNION
                 SELECT g.parent_group_id FROM groups g
                   JOIN member_groups mg ON g.group_id = mg.gid
                  WHERE g.parent_group_id IS NOT NULL
             )
             SELECT MAX(cr.allow), cr.rate_limit, cr.ttl
             FROM capability_rules cr
             WHERE cr.capability_name = ?1
               AND cr.group_id IN (SELECT gid FROM member_groups)",
            params![capability_name, peer_id, GROUP_DEFAULT.to_string()],
            |row| {
                let allow: Option<i32> = row.get(0)?;
                let rate_limit: Option<u64> = row.get(1)?;
                let ttl: Option<u64> = row.get(2)?;
                Ok((allow, rate_limit, ttl))
            },
        );

        match result {
            Ok((Some(allow), rate_limit, ttl)) if allow >= 1 => {
                PermissionResult::Allow { rate_limit, ttl }
            }
            _ => PermissionResult::Deny,
        }
    }

    /// Resolve permissions for multiple capabilities in one pass.
    pub fn resolve_all_permissions(
        &self,
        peer_id: &[u8],
        capabilities: &[&str],
    ) -> HashMap<String, PermissionResult> {
        capabilities
            .iter()
            .map(|cap| (cap.to_string(), self.resolve_permission(peer_id, cap)))
            .collect()
    }

    /// Resolve effective permissions across ALL known capabilities for a peer.
    pub fn get_peer_effective_permissions(
        &self,
        peer_id: &[u8],
    ) -> rusqlite::Result<HashMap<String, PermissionResult>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT capability_name FROM capability_rules")?;
        let cap_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        let refs: Vec<&str> = cap_names.iter().map(|s| s.as_str()).collect();
        Ok(self.resolve_all_permissions(peer_id, &refs))
    }

    // ── Group CRUD ───────────────────────────────────────────────────────

    /// List all groups (built-in and custom).
    pub fn list_groups(&self) -> rusqlite::Result<Vec<Group>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT group_id, name, built_in, created_at, description, parent_group_id FROM groups ORDER BY built_in DESC, name",
        )?;
        let groups: Vec<Group> = stmt
            .query_map([], |row| {
                let group_id_str: String = row.get(0)?;
                Ok(Group {
                    group_id: Uuid::parse_str(&group_id_str).unwrap_or(Uuid::nil()),
                    name: row.get(1)?,
                    built_in: row.get::<_, i32>(2)? != 0,
                    capabilities: Vec::new(),
                    created_at: row.get(3)?,
                    description: row.get(4)?,
                    parent_group_id: {
                        let s: Option<String> = row.get(5)?;
                        s.and_then(|v| Uuid::parse_str(&v).ok())
                    },
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        let mut result = Vec::with_capacity(groups.len());
        for mut group in groups {
            group.capabilities = self.get_capability_rules(&group.group_id)?;
            result.push(group);
        }
        Ok(result)
    }

    /// Get a single group by ID, or None if not found.
    pub fn get_group(&self, group_id: &Uuid) -> rusqlite::Result<Option<Group>> {
        let group: Option<Group> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT group_id, name, built_in, created_at, description, parent_group_id FROM groups WHERE group_id = ?1",
                params![group_id.to_string()],
                |row| {
                    let group_id_str: String = row.get(0)?;
                    Ok(Group {
                        group_id: Uuid::parse_str(&group_id_str).unwrap_or(Uuid::nil()),
                        name: row.get(1)?,
                        built_in: row.get::<_, i32>(2)? != 0,
                        capabilities: Vec::new(),
                        created_at: row.get(3)?,
                        description: row.get(4)?,
                        parent_group_id: {
                            let s: Option<String> = row.get(5)?;
                            s.and_then(|v| Uuid::parse_str(&v).ok())
                        },
                    })
                },
            )
            .optional()?
        };

        match group {
            Some(mut g) => {
                g.capabilities = self.get_capability_rules(&g.group_id)?;
                Ok(Some(g))
            }
            None => Ok(None),
        }
    }

    /// Create a custom group.
    pub fn create_group(
        &self,
        name: &str,
        description: Option<&str>,
        capability_rules: &[CapabilityRule],
    ) -> rusqlite::Result<Group> {
        let group_id = Uuid::new_v4();
        let now = epoch_secs();

        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO groups (group_id, name, built_in, created_at, description)
                 VALUES (?1, ?2, 0, ?3, ?4)",
                params![group_id.to_string(), name, now, description],
            )?;

            for rule in capability_rules {
                insert_capability_rule(&conn, &group_id, rule)?;
            }
        }

        debug!("created custom group '{}' ({})", name, group_id);
        self.get_group(&group_id).map(|g| g.unwrap())
    }

    /// Update a custom group's name, description, or capability rules.
    pub fn update_group(
        &self,
        group_id: &Uuid,
        name: Option<&str>,
        description: Option<Option<&str>>,
        capability_rules: Option<&[CapabilityRule]>,
    ) -> rusqlite::Result<Option<Group>> {
        let group = match self.get_group(group_id)? {
            Some(g) => g,
            None => return Ok(None),
        };

        if group.built_in && capability_rules.is_some() {
            warn!(
                "attempted to modify capability rules of built-in group '{}'",
                group.name
            );
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        {
            let conn = self.conn.lock();

            if let Some(new_name) = name {
                conn.execute(
                    "UPDATE groups SET name = ?1 WHERE group_id = ?2",
                    params![new_name, group_id.to_string()],
                )?;
            }

            if let Some(new_desc) = description {
                conn.execute(
                    "UPDATE groups SET description = ?1 WHERE group_id = ?2",
                    params![new_desc, group_id.to_string()],
                )?;
            }

            if let Some(rules) = capability_rules {
                conn.execute(
                    "DELETE FROM capability_rules WHERE group_id = ?1",
                    params![group_id.to_string()],
                )?;
                for rule in rules {
                    insert_capability_rule(&conn, group_id, rule)?;
                }
            }
        }

        self.get_group(group_id)
    }

    /// Delete a custom group. Returns error if built-in.
    pub fn delete_group(&self, group_id: &Uuid) -> rusqlite::Result<bool> {
        let group = match self.get_group(group_id)? {
            Some(g) => g,
            None => return Ok(false),
        };

        if group.built_in {
            warn!("attempted to delete built-in group '{}'", group.name);
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM groups WHERE group_id = ?1",
            params![group_id.to_string()],
        )?;

        debug!("deleted custom group '{}' ({})", group.name, group_id);
        Ok(true)
    }

    // ── Membership CRUD ──────────────────────────────────────────────────

    /// List all groups a peer belongs to (explicit memberships only).
    pub fn list_peer_groups(&self, peer_id: &[u8]) -> rusqlite::Result<Vec<Group>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT g.group_id, g.name, g.built_in, g.created_at, g.description, g.parent_group_id
             FROM groups g
             JOIN peer_group_memberships pgm ON g.group_id = pgm.group_id
             WHERE pgm.peer_id = ?1
             ORDER BY g.built_in DESC, g.name",
        )?;
        let groups: Vec<Group> = stmt
            .query_map(params![peer_id], |row| {
                let group_id_str: String = row.get(0)?;
                Ok(Group {
                    group_id: Uuid::parse_str(&group_id_str).unwrap_or(Uuid::nil()),
                    name: row.get(1)?,
                    built_in: row.get::<_, i32>(2)? != 0,
                    capabilities: Vec::new(),
                    created_at: row.get(3)?,
                    description: row.get(4)?,
                    parent_group_id: {
                        let s: Option<String> = row.get(5)?;
                        s.and_then(|v| Uuid::parse_str(&v).ok())
                    },
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        let mut result = Vec::with_capacity(groups.len());
        for mut group in groups {
            group.capabilities = self.get_capability_rules(&group.group_id)?;
            result.push(group);
        }
        Ok(result)
    }

    /// List all peer IDs that are members of a specific group.
    pub fn list_group_member_ids(&self, group_id: &Uuid) -> rusqlite::Result<Vec<Vec<u8>>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT peer_id FROM peer_group_memberships WHERE group_id = ?1 ORDER BY assigned_at",
        )?;
        let members: Vec<Vec<u8>> = stmt
            .query_map(params![group_id.to_string()], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(members)
    }

    /// Assign a peer to a group. No-op if already assigned.
    pub fn assign_peer_to_group(
        &self,
        peer_id: &[u8],
        group_id: &Uuid,
    ) -> rusqlite::Result<PeerGroupMembership> {
        let now = epoch_secs();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO peer_group_memberships (peer_id, group_id, assigned_at, assigned_by)
             VALUES (?1, ?2, ?3, 'local')",
            params![peer_id, group_id.to_string(), now],
        )?;

        debug!(
            "assigned peer {} to group {}",
            hex::encode(peer_id),
            group_id
        );
        Ok(PeerGroupMembership {
            peer_id: peer_id.to_vec(),
            group_id: *group_id,
            assigned_at: now,
            assigned_by: "local".to_string(),
        })
    }

    /// Remove a peer from a group. Returns true if a row was deleted.
    pub fn remove_peer_from_group(
        &self,
        peer_id: &[u8],
        group_id: &Uuid,
    ) -> rusqlite::Result<bool> {
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "DELETE FROM peer_group_memberships WHERE peer_id = ?1 AND group_id = ?2",
            params![peer_id, group_id.to_string()],
        )?;
        if deleted > 0 {
            debug!(
                "removed peer {} from group {}",
                hex::encode(peer_id),
                group_id
            );
        }
        Ok(deleted > 0)
    }

    /// Remove a peer from ALL groups (used by deny flow).
    pub fn remove_peer_from_all_groups(&self, peer_id: &[u8]) -> rusqlite::Result<usize> {
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "DELETE FROM peer_group_memberships WHERE peer_id = ?1",
            params![peer_id],
        )?;
        if deleted > 0 {
            debug!(
                "removed peer {} from all groups ({} memberships)",
                hex::encode(peer_id),
                deleted
            );
        }
        Ok(deleted)
    }

    /// Check if a peer has any explicit group memberships.
    pub fn peer_has_memberships(&self, peer_id: &[u8]) -> rusqlite::Result<bool> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM peer_group_memberships WHERE peer_id = ?1",
            params![peer_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    // ── Internal helpers ─────────────────────────────────────────────────

    fn get_capability_rules(&self, group_id: &Uuid) -> rusqlite::Result<Vec<CapabilityRule>> {
        let conn = self.conn.lock();
        let group_id_str = group_id.to_string();
        let mut stmt = conn.prepare(
            "SELECT capability_name, allow, rate_limit, ttl
             FROM capability_rules
             WHERE group_id = ?1
             ORDER BY capability_name",
        )?;
        let rules: Vec<CapabilityRule> = stmt
            .query_map(params![group_id_str], |row| {
                Ok(CapabilityRule {
                    capability_name: row.get(0)?,
                    allow: row.get::<_, i32>(1)? != 0,
                    rate_limit: row.get(2)?,
                    ttl: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rules)
    }
}

fn insert_capability_rule(
    conn: &Connection,
    group_id: &Uuid,
    rule: &CapabilityRule,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO capability_rules (group_id, capability_name, allow, rate_limit, ttl)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            group_id.to_string(),
            rule.capability_name,
            rule.allow as i32,
            rule.rate_limit,
            rule.ttl,
        ],
    )?;
    Ok(())
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

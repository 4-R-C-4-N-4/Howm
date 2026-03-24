use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// Well-known group UUIDs (mirrors node/access/src/types.rs)
const GROUP_FRIENDS: Uuid = Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
]);

const GROUP_TRUSTED: Uuid = Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
]);

// ── Download ────────────────────────────────────────────────────────────────

/// A tracked download from a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub blob_id: String,
    pub offering_id: String,
    pub peer_id: String,
    pub transfer_id: i64,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub status: String, // pending, transferring, complete, failed
    pub started_at: i64,
    pub completed_at: Option<i64>,
}

// ── Offering ────────────────────────────────────────────────────────────────

/// A file offering in the catalogue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offering {
    pub offering_id: String,
    pub blob_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub mime_type: String,
    pub size: i64,
    pub created_at: i64,
    pub access: String,
    /// JSON array of base64 peer_ids, used when access='peer'.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<String>,
}

/// Partial update for an existing offering.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OfferingUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub access: Option<String>,
    pub allowlist: Option<String>,
}

/// Peer group info (cached from daemon access API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerGroup {
    pub group_id: String,
    pub name: String,
    pub built_in: bool,
}

// ── Database ────────────────────────────────────────────────────────────────

/// Handle to the files capability database.
///
/// Stores offering catalogue and download tracking.
/// Uses `Arc<Mutex<Connection>>` so the struct is `Clone + Send + Sync`
/// and can live inside Axum state.
#[derive(Clone)]
pub struct FilesDb {
    conn: Arc<Mutex<Connection>>,
}

impl FilesDb {
    /// Open (or create) the files database at `data_dir/files.db`.
    pub fn open(data_dir: &Path) -> rusqlite::Result<Self> {
        let db_path = data_dir.join("files.db");
        let conn = Connection::open(&db_path)?;
        create_tables(&conn)?;
        tracing::debug!("files.db opened at {}", db_path.display());
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory database (for testing).
    #[cfg(test)]
    pub fn open_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        create_tables(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Offerings CRUD ────────────────────────────────────────────────────────

    /// Insert a new offering. Returns Err if name is not unique.
    pub fn insert_offering(&self, offering: &Offering) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO offerings (offering_id, blob_id, name, description, mime_type, size, created_at, access, allowlist)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                offering.offering_id,
                offering.blob_id,
                offering.name,
                offering.description,
                offering.mime_type,
                offering.size,
                offering.created_at,
                offering.access,
                offering.allowlist,
            ],
        )
        .map_err(|e| {
            if let rusqlite::Error::SqliteFailure(ref err, _) = e {
                if err.extended_code == 2067 {
                    // SQLITE_CONSTRAINT_UNIQUE
                    return anyhow::anyhow!("name_conflict");
                }
            }
            anyhow::anyhow!("insert failed: {}", e)
        })?;
        Ok(())
    }

    /// List all offerings (operator view).
    pub fn list_offerings(&self) -> anyhow::Result<Vec<Offering>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT offering_id, blob_id, name, description, mime_type, size, created_at, access, allowlist
             FROM offerings ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_offering)?;
        let mut offerings = Vec::new();
        for row in rows {
            offerings.push(row?);
        }
        Ok(offerings)
    }

    /// Get a single offering by ID.
    pub fn get_offering(&self, offering_id: &str) -> anyhow::Result<Option<Offering>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT offering_id, blob_id, name, description, mime_type, size, created_at, access, allowlist
                 FROM offerings WHERE offering_id = ?1",
                params![offering_id],
                row_to_offering,
            )
            .optional()?;
        Ok(result)
    }

    /// Update an offering (partial update). Returns false if offering not found.
    pub fn update_offering(
        &self,
        offering_id: &str,
        update: &OfferingUpdate,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();

        let mut sets = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref name) = update.name {
            sets.push("name = ?");
            values.push(Box::new(name.clone()));
        }
        if let Some(ref desc) = update.description {
            sets.push("description = ?");
            values.push(Box::new(desc.clone()));
        }
        if let Some(ref access) = update.access {
            sets.push("access = ?");
            values.push(Box::new(access.clone()));
        }
        if let Some(ref allowlist) = update.allowlist {
            sets.push("allowlist = ?");
            values.push(Box::new(allowlist.clone()));
        }

        if sets.is_empty() {
            return Ok(false);
        }

        // Re-number placeholders
        let numbered_sets: Vec<String> = sets
            .iter()
            .enumerate()
            .map(|(i, s)| s.replace('?', &format!("?{}", i + 1)))
            .collect();

        let sql = format!(
            "UPDATE offerings SET {} WHERE offering_id = ?{}",
            numbered_sets.join(", "),
            values.len() + 1
        );
        values.push(Box::new(offering_id.to_string()));

        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let updated = conn.execute(&sql, params.as_slice()).map_err(|e| {
            if let rusqlite::Error::SqliteFailure(ref err, _) = e {
                if err.extended_code == 2067 {
                    return anyhow::anyhow!("name_conflict");
                }
            }
            anyhow::anyhow!("update failed: {}", e)
        })?;
        Ok(updated > 0)
    }

    /// Delete an offering. Returns the blob_id so the caller can optionally
    /// delete the blob, or None if the offering wasn't found.
    pub fn delete_offering(&self, offering_id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let blob_id: Option<String> = conn
            .query_row(
                "SELECT blob_id FROM offerings WHERE offering_id = ?1",
                params![offering_id],
                |row| row.get(0),
            )
            .optional()?;

        if blob_id.is_some() {
            conn.execute(
                "DELETE FROM offerings WHERE offering_id = ?1",
                params![offering_id],
            )?;
        }
        Ok(blob_id)
    }

    /// List offerings visible to a specific peer, filtered by access policy.
    /// `peer_id_b64` is the base64-encoded 32-byte peer public key.
    /// `peer_groups` are the cached group memberships for this peer.
    pub fn list_offerings_for_peer(
        &self,
        peer_id_b64: &str,
        peer_groups: &[PeerGroup],
    ) -> anyhow::Result<Vec<Offering>> {
        let all = self.list_offerings()?;
        let visible: Vec<Offering> = all
            .into_iter()
            .filter(|o| peer_can_see_offering(o, peer_id_b64, peer_groups))
            .collect();
        Ok(visible)
    }

    // ── Downloads CRUD ─────────────────────────────────────────────────────────

    /// Insert a new download record.
    pub fn insert_download(&self, dl: &Download) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO downloads (blob_id, offering_id, peer_id, transfer_id, name, mime_type, size, status, started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                dl.blob_id,
                dl.offering_id,
                dl.peer_id,
                dl.transfer_id,
                dl.name,
                dl.mime_type,
                dl.size,
                dl.status,
                dl.started_at,
                dl.completed_at,
            ],
        )?;
        Ok(())
    }

    /// List all downloads.
    pub fn list_downloads(&self) -> anyhow::Result<Vec<Download>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT blob_id, offering_id, peer_id, transfer_id, name, mime_type, size, status, started_at, completed_at
             FROM downloads ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_download)?;
        let mut downloads = Vec::new();
        for row in rows {
            downloads.push(row?);
        }
        Ok(downloads)
    }

    /// Get a single download by blob_id.
    pub fn get_download(&self, blob_id: &str) -> anyhow::Result<Option<Download>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT blob_id, offering_id, peer_id, transfer_id, name, mime_type, size, status, started_at, completed_at
                 FROM downloads WHERE blob_id = ?1",
                params![blob_id],
                row_to_download,
            )
            .optional()?;
        Ok(result)
    }

    /// Update a download's status and optionally completed_at. Returns true if a row was updated.
    pub fn update_download_status(
        &self,
        blob_id: &str,
        status: &str,
        completed_at: Option<i64>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE downloads SET status = ?1, completed_at = ?2 WHERE blob_id = ?3",
            params![status, completed_at, blob_id],
        )?;
        Ok(updated > 0)
    }

    /// Delete a download by blob_id. Returns true if a row was deleted.
    #[allow(dead_code)]
    pub fn delete_download(&self, blob_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute("DELETE FROM downloads WHERE blob_id = ?1", params![blob_id])?;
        Ok(deleted > 0)
    }

    /// List offerings visible to a peer, with pagination.
    /// Returns (offerings, total_visible_count).
    pub fn list_offerings_for_peer_paginated(
        &self,
        peer_id_b64: &str,
        peer_groups: &[PeerGroup],
        cursor: usize,
        limit: usize,
    ) -> anyhow::Result<(Vec<Offering>, usize)> {
        let all = self.list_offerings_for_peer(peer_id_b64, peer_groups)?;
        let total = all.len();
        let page: Vec<Offering> = all.into_iter().skip(cursor).take(limit).collect();
        Ok((page, total))
    }
}

fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS offerings (
            offering_id TEXT PRIMARY KEY,
            blob_id     TEXT NOT NULL,
            name        TEXT NOT NULL UNIQUE,
            description TEXT,
            mime_type   TEXT NOT NULL,
            size        INTEGER NOT NULL,
            created_at  INTEGER NOT NULL,
            access      TEXT NOT NULL DEFAULT 'public',
            allowlist   TEXT
        );

        CREATE TABLE IF NOT EXISTS downloads (
            blob_id      TEXT PRIMARY KEY,
            offering_id  TEXT NOT NULL,
            peer_id      TEXT NOT NULL,
            transfer_id  INTEGER NOT NULL,
            name         TEXT NOT NULL,
            mime_type    TEXT NOT NULL,
            size         INTEGER NOT NULL,
            status       TEXT NOT NULL DEFAULT 'pending',
            started_at   INTEGER NOT NULL,
            completed_at INTEGER
        );
        ",
    )
}

fn row_to_offering(row: &rusqlite::Row) -> rusqlite::Result<Offering> {
    Ok(Offering {
        offering_id: row.get(0)?,
        blob_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        mime_type: row.get(4)?,
        size: row.get(5)?,
        created_at: row.get(6)?,
        access: row.get(7)?,
        allowlist: row.get(8)?,
    })
}

fn row_to_download(row: &rusqlite::Row) -> rusqlite::Result<Download> {
    Ok(Download {
        blob_id: row.get(0)?,
        offering_id: row.get(1)?,
        peer_id: row.get(2)?,
        transfer_id: row.get(3)?,
        name: row.get(4)?,
        mime_type: row.get(5)?,
        size: row.get(6)?,
        status: row.get(7)?,
        started_at: row.get(8)?,
        completed_at: row.get(9)?,
    })
}

// ── Access filtering ────────────────────────────────────────────────────────

/// Check whether a peer should see a given offering based on its access policy.
pub fn peer_can_see_offering(
    offering: &Offering,
    peer_id_b64: &str,
    peer_groups: &[PeerGroup],
) -> bool {
    match offering.access.as_str() {
        "public" => true,
        "friends" => peer_groups.iter().any(|g| {
            parse_group_uuid(&g.group_id)
                .is_some_and(|uuid| uuid == GROUP_FRIENDS || uuid == GROUP_TRUSTED)
        }),
        "trusted" => peer_groups
            .iter()
            .any(|g| parse_group_uuid(&g.group_id) == Some(GROUP_TRUSTED)),
        "peer" => allowlist_contains(&offering.allowlist, peer_id_b64),
        access if access.starts_with("group:") => {
            let gid_str = &access[6..];
            match Uuid::parse_str(gid_str) {
                Ok(target) => peer_groups
                    .iter()
                    .any(|g| parse_group_uuid(&g.group_id) == Some(target)),
                Err(_) => false,
            }
        }
        access if access.starts_with("groups:") => {
            let gids: Vec<Uuid> = access[7..]
                .split(',')
                .filter_map(|s| Uuid::parse_str(s.trim()).ok())
                .collect();
            peer_groups
                .iter()
                .any(|g| parse_group_uuid(&g.group_id).is_some_and(|uuid| gids.contains(&uuid)))
        }
        _ => false, // unknown policy = deny
    }
}

/// Check if a peer_id is in the JSON allowlist.
fn allowlist_contains(allowlist: &Option<String>, peer_id_b64: &str) -> bool {
    match allowlist {
        Some(json_str) => {
            let ids: Vec<String> = serde_json::from_str(json_str).unwrap_or_default();
            ids.iter().any(|id| id == peer_id_b64)
        }
        None => false,
    }
}

/// Parse a group_id string as UUID. The daemon returns UUIDs as hyphenated strings.
fn parse_group_uuid(group_id: &str) -> Option<Uuid> {
    Uuid::parse_str(group_id).ok()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_offering(name: &str, access: &str) -> Offering {
        Offering {
            offering_id: Uuid::new_v4().to_string(),
            blob_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string(),
            name: name.to_string(),
            description: Some("test file".to_string()),
            mime_type: "application/octet-stream".to_string(),
            size: 1024,
            created_at: 1700000000,
            access: access.to_string(),
            allowlist: None,
        }
    }

    fn friends_group() -> PeerGroup {
        PeerGroup {
            group_id: GROUP_FRIENDS.to_string(),
            name: "howm.friends".to_string(),
            built_in: true,
        }
    }

    fn trusted_group() -> PeerGroup {
        PeerGroup {
            group_id: GROUP_TRUSTED.to_string(),
            name: "howm.trusted".to_string(),
            built_in: true,
        }
    }

    fn custom_group(uuid: &str) -> PeerGroup {
        PeerGroup {
            group_id: uuid.to_string(),
            name: "custom-group".to_string(),
            built_in: false,
        }
    }

    // ── CRUD tests ──────────────────────────────────────────────────────────

    #[test]
    fn insert_and_list_offering() {
        let db = FilesDb::open_memory().unwrap();
        let o = make_offering("test.txt", "public");
        db.insert_offering(&o).unwrap();

        let list = db.list_offerings().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "test.txt");
        assert_eq!(list[0].access, "public");
    }

    #[test]
    fn insert_duplicate_name_fails() {
        let db = FilesDb::open_memory().unwrap();
        let o1 = make_offering("same-name.txt", "public");
        db.insert_offering(&o1).unwrap();

        let mut o2 = make_offering("same-name.txt", "friends");
        o2.offering_id = Uuid::new_v4().to_string();
        let err = db.insert_offering(&o2).unwrap_err();
        assert!(err.to_string().contains("name_conflict"));
    }

    #[test]
    fn get_offering_by_id() {
        let db = FilesDb::open_memory().unwrap();
        let o = make_offering("get-test.bin", "public");
        let id = o.offering_id.clone();
        db.insert_offering(&o).unwrap();

        let found = db.get_offering(&id).unwrap().unwrap();
        assert_eq!(found.name, "get-test.bin");

        let missing = db.get_offering("nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn update_offering_fields() {
        let db = FilesDb::open_memory().unwrap();
        let o = make_offering("update-me.txt", "public");
        let id = o.offering_id.clone();
        db.insert_offering(&o).unwrap();

        let update = OfferingUpdate {
            name: Some("renamed.txt".to_string()),
            description: Some("updated description".to_string()),
            access: Some("friends".to_string()),
            allowlist: None,
        };
        let updated = db.update_offering(&id, &update).unwrap();
        assert!(updated);

        let found = db.get_offering(&id).unwrap().unwrap();
        assert_eq!(found.name, "renamed.txt");
        assert_eq!(found.description.as_deref(), Some("updated description"));
        assert_eq!(found.access, "friends");
    }

    #[test]
    fn update_nonexistent_returns_false() {
        let db = FilesDb::open_memory().unwrap();
        let update = OfferingUpdate {
            name: Some("nope".to_string()),
            ..Default::default()
        };
        let updated = db.update_offering("nonexistent", &update).unwrap();
        assert!(!updated);
    }

    #[test]
    fn update_name_conflict() {
        let db = FilesDb::open_memory().unwrap();
        let o1 = make_offering("first.txt", "public");
        let o2 = make_offering("second.txt", "public");
        db.insert_offering(&o1).unwrap();
        db.insert_offering(&o2).unwrap();

        let update = OfferingUpdate {
            name: Some("first.txt".to_string()),
            ..Default::default()
        };
        let err = db.update_offering(&o2.offering_id, &update).unwrap_err();
        assert!(err.to_string().contains("name_conflict"));
    }

    #[test]
    fn delete_offering_returns_blob_id() {
        let db = FilesDb::open_memory().unwrap();
        let o = make_offering("delete-me.txt", "public");
        let id = o.offering_id.clone();
        let expected_blob = o.blob_id.clone();
        db.insert_offering(&o).unwrap();

        let blob_id = db.delete_offering(&id).unwrap();
        assert_eq!(blob_id, Some(expected_blob));

        // Verify it's gone
        let found = db.get_offering(&id).unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn delete_nonexistent_returns_none() {
        let db = FilesDb::open_memory().unwrap();
        let blob_id = db.delete_offering("nonexistent").unwrap();
        assert!(blob_id.is_none());
    }

    // ── Access filtering tests ──────────────────────────────────────────────

    #[test]
    fn public_visible_to_all() {
        let o = make_offering("public.txt", "public");
        assert!(peer_can_see_offering(&o, "somepeer", &[]));
    }

    #[test]
    fn friends_visible_to_friends_and_trusted() {
        let o = make_offering("friends.txt", "friends");

        // No groups = denied
        assert!(!peer_can_see_offering(&o, "peer1", &[]));

        // Friends group = allowed
        assert!(peer_can_see_offering(&o, "peer1", &[friends_group()]));

        // Trusted group = also allowed (trusted ⊃ friends)
        assert!(peer_can_see_offering(&o, "peer1", &[trusted_group()]));
    }

    #[test]
    fn trusted_visible_only_to_trusted() {
        let o = make_offering("trusted.txt", "trusted");

        assert!(!peer_can_see_offering(&o, "peer1", &[]));
        assert!(!peer_can_see_offering(&o, "peer1", &[friends_group()]));
        assert!(peer_can_see_offering(&o, "peer1", &[trusted_group()]));
    }

    #[test]
    fn peer_allowlist_filtering() {
        let mut o = make_offering("secret.txt", "peer");
        o.allowlist = Some(serde_json::to_string(&vec!["alice_b64", "bob_b64"]).unwrap());

        assert!(peer_can_see_offering(&o, "alice_b64", &[]));
        assert!(peer_can_see_offering(&o, "bob_b64", &[]));
        assert!(!peer_can_see_offering(&o, "charlie_b64", &[]));
    }

    #[test]
    fn peer_allowlist_empty_or_missing() {
        let o = make_offering("secret.txt", "peer");
        // No allowlist = nobody can see it
        assert!(!peer_can_see_offering(&o, "alice_b64", &[]));

        let mut o2 = make_offering("secret2.txt", "peer");
        o2.allowlist = Some("[]".to_string());
        assert!(!peer_can_see_offering(&o2, "alice_b64", &[]));
    }

    #[test]
    fn single_group_access() {
        let custom_uuid = "a1b2c3d4-e5f6-7890-abcd-ef0123456789";
        let o = make_offering("group-file.txt", &format!("group:{}", custom_uuid));

        // Not in group
        assert!(!peer_can_see_offering(&o, "peer1", &[]));
        assert!(!peer_can_see_offering(&o, "peer1", &[friends_group()]));

        // In the matching custom group
        assert!(peer_can_see_offering(
            &o,
            "peer1",
            &[custom_group(custom_uuid)]
        ));
    }

    #[test]
    fn multi_group_access() {
        let uuid1 = "a1b2c3d4-e5f6-7890-abcd-ef0123456789";
        let uuid2 = "b2c3d4e5-f6a7-8901-bcde-f01234567890";
        let o = make_offering("multi-group.txt", &format!("groups:{},{}", uuid1, uuid2));

        // Not in any group
        assert!(!peer_can_see_offering(&o, "peer1", &[]));

        // In first group
        assert!(peer_can_see_offering(&o, "peer1", &[custom_group(uuid1)]));

        // In second group
        assert!(peer_can_see_offering(&o, "peer1", &[custom_group(uuid2)]));

        // In neither
        assert!(!peer_can_see_offering(
            &o,
            "peer1",
            &[custom_group("00000000-0000-0000-0000-000000000099")]
        ));
    }

    #[test]
    fn invalid_group_uuid_denies() {
        let o = make_offering("bad-group.txt", "group:not-a-uuid");
        assert!(!peer_can_see_offering(&o, "peer1", &[friends_group()]));
    }

    #[test]
    fn unknown_access_policy_denies() {
        let o = make_offering("unknown.txt", "some_new_policy");
        assert!(!peer_can_see_offering(
            &o,
            "peer1",
            &[friends_group(), trusted_group()]
        ));
    }

    // ── list_offerings_for_peer integration ──────────────────────────────────

    #[test]
    fn list_for_peer_filters_correctly() {
        let db = FilesDb::open_memory().unwrap();

        let mut o1 = make_offering("public.txt", "public");
        o1.offering_id = "o1".to_string();
        db.insert_offering(&o1).unwrap();

        let mut o2 = make_offering("friends-only.txt", "friends");
        o2.offering_id = "o2".to_string();
        db.insert_offering(&o2).unwrap();

        let mut o3 = make_offering("trusted-only.txt", "trusted");
        o3.offering_id = "o3".to_string();
        db.insert_offering(&o3).unwrap();

        let mut o4 = make_offering("peer-specific.txt", "peer");
        o4.offering_id = "o4".to_string();
        o4.allowlist = Some(serde_json::to_string(&vec!["alice"]).unwrap());
        db.insert_offering(&o4).unwrap();

        // No groups, not in allowlist
        let visible = db.list_offerings_for_peer("bob", &[]).unwrap();
        assert_eq!(visible.len(), 1); // only public
        assert_eq!(visible[0].name, "public.txt");

        // Friends group
        let visible = db
            .list_offerings_for_peer("bob", &[friends_group()])
            .unwrap();
        assert_eq!(visible.len(), 2); // public + friends

        // Trusted group
        let visible = db
            .list_offerings_for_peer("bob", &[trusted_group()])
            .unwrap();
        assert_eq!(visible.len(), 3); // public + friends + trusted

        // Alice with friends
        let visible = db
            .list_offerings_for_peer("alice", &[friends_group()])
            .unwrap();
        assert_eq!(visible.len(), 3); // public + friends + peer-specific
    }

    // ── Pagination ──────────────────────────────────────────────────────────

    #[test]
    fn paginated_listing() {
        let db = FilesDb::open_memory().unwrap();

        for i in 0..5 {
            let mut o = make_offering(&format!("file-{}.txt", i), "public");
            o.offering_id = format!("o{}", i);
            o.created_at = 1700000000 + i;
            db.insert_offering(&o).unwrap();
        }

        let (page, total) = db
            .list_offerings_for_peer_paginated("peer", &[], 0, 2)
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(page.len(), 2);

        let (page2, _) = db
            .list_offerings_for_peer_paginated("peer", &[], 2, 2)
            .unwrap();
        assert_eq!(page2.len(), 2);

        let (page3, _) = db
            .list_offerings_for_peer_paginated("peer", &[], 4, 2)
            .unwrap();
        assert_eq!(page3.len(), 1);

        let (page4, _) = db
            .list_offerings_for_peer_paginated("peer", &[], 10, 2)
            .unwrap();
        assert_eq!(page4.len(), 0);
    }

    // ── Download CRUD tests ──────────────────────────────────────────────

    fn make_download(blob_id: &str, status: &str) -> Download {
        Download {
            blob_id: blob_id.to_string(),
            offering_id: "off-1".to_string(),
            peer_id: "peer-abc".to_string(),
            transfer_id: 123456,
            name: "test-file.bin".to_string(),
            mime_type: "application/octet-stream".to_string(),
            size: 2048,
            status: status.to_string(),
            started_at: 1700000000,
            completed_at: None,
        }
    }

    #[test]
    fn insert_and_list_downloads() {
        let db = FilesDb::open_memory().unwrap();
        let dl = make_download("blob1", "transferring");
        db.insert_download(&dl).unwrap();

        let list = db.list_downloads().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].blob_id, "blob1");
        assert_eq!(list[0].status, "transferring");
    }

    #[test]
    fn get_download_by_blob_id() {
        let db = FilesDb::open_memory().unwrap();
        let dl = make_download("blob2", "pending");
        db.insert_download(&dl).unwrap();

        let found = db.get_download("blob2").unwrap().unwrap();
        assert_eq!(found.name, "test-file.bin");
        assert_eq!(found.transfer_id, 123456);

        let missing = db.get_download("nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn update_download_status_works() {
        let db = FilesDb::open_memory().unwrap();
        let dl = make_download("blob3", "transferring");
        db.insert_download(&dl).unwrap();

        let updated = db
            .update_download_status("blob3", "complete", Some(1700001000))
            .unwrap();
        assert!(updated);

        let found = db.get_download("blob3").unwrap().unwrap();
        assert_eq!(found.status, "complete");
        assert_eq!(found.completed_at, Some(1700001000));
    }

    #[test]
    fn update_download_status_nonexistent() {
        let db = FilesDb::open_memory().unwrap();
        let updated = db.update_download_status("nope", "complete", None).unwrap();
        assert!(!updated);
    }

    #[test]
    fn delete_download_works() {
        let db = FilesDb::open_memory().unwrap();
        let dl = make_download("blob4", "complete");
        db.insert_download(&dl).unwrap();

        let deleted = db.delete_download("blob4").unwrap();
        assert!(deleted);
        assert!(db.get_download("blob4").unwrap().is_none());
    }

    #[test]
    fn delete_download_nonexistent() {
        let db = FilesDb::open_memory().unwrap();
        let deleted = db.delete_download("nope").unwrap();
        assert!(!deleted);
    }
}

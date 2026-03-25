use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED};

/// Create all tables and indexes. Idempotent (IF NOT EXISTS).
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
PRAGMA journal_mode = WAL;
    PRAGMA busy_timeout = 5000;
    PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS groups (
            group_id        TEXT PRIMARY KEY,
            name            TEXT NOT NULL,
            built_in        INTEGER NOT NULL DEFAULT 0,
            created_at      INTEGER NOT NULL,
            description     TEXT,
            parent_group_id TEXT REFERENCES groups(group_id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS capability_rules (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id        TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
            capability_name TEXT NOT NULL,
            allow           INTEGER NOT NULL DEFAULT 1,
            rate_limit      INTEGER,
            ttl             INTEGER,
            UNIQUE(group_id, capability_name)
        );

        CREATE TABLE IF NOT EXISTS peer_group_memberships (
            peer_id     BLOB NOT NULL,
            group_id    TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
            assigned_at INTEGER NOT NULL,
            assigned_by TEXT NOT NULL DEFAULT 'local',
            PRIMARY KEY (peer_id, group_id)
        );

        CREATE INDEX IF NOT EXISTS idx_pgm_peer  ON peer_group_memberships(peer_id);
        CREATE INDEX IF NOT EXISTS idx_pgm_group ON peer_group_memberships(group_id);
        CREATE INDEX IF NOT EXISTS idx_cr_group  ON capability_rules(group_id);
        ",
    )
}

/// Seed the three built-in groups with their fixed capability rules.
/// Idempotent — skips if groups already exist.
pub fn seed_built_in_groups(conn: &Connection) -> rusqlite::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // ── howm.default (no parent — base tier) ─────────────────────────────
    upsert_group(
        conn,
        &GROUP_DEFAULT.to_string(),
        "howm.default",
        "Session health + endpoint reflection only",
        now,
        None,
    )?;
    let default_caps = &[
        "core.session.heartbeat.1",
        "core.session.attest.1",
        "core.session.latency.1",
        "core.network.endpoint.1",
        "core.session.timesync.1",
    ];
    seed_capability_rules(conn, &GROUP_DEFAULT.to_string(), default_caps)?;

    // ── howm.friends (inherits default) ─────────────────────────────────
    upsert_group(
        conn,
        &GROUP_FRIENDS.to_string(),
        "howm.friends",
        "Inherits default + social, room access, and peer exchange",
        now,
        Some(&GROUP_DEFAULT.to_string()),
    )?;
    // Only capabilities added at this tier (default caps inherited via parent)
    let friends_caps = &[
        "howm.social.feed.1",
        "howm.social.messaging.1",
        "howm.social.files.1",
        "howm.world.room.1",
        "core.network.peerexchange.1",
    ];
    seed_capability_rules(conn, &GROUP_FRIENDS.to_string(), friends_caps)?;

    // ── howm.trusted (inherits friends → default) ───────────────────────
    upsert_group(
        conn,
        &GROUP_TRUSTED.to_string(),
        "howm.trusted",
        "Full access — inherits friends + relay",
        now,
        Some(&GROUP_FRIENDS.to_string()),
    )?;
    // Only capabilities added at this tier (friends + default inherited)
    let trusted_caps = &["core.network.relay.1"];
    seed_capability_rules(conn, &GROUP_TRUSTED.to_string(), trusted_caps)?;

    Ok(())
}

fn upsert_group(
    conn: &Connection,
    group_id: &str,
    name: &str,
    description: &str,
    now: u64,
    parent_group_id: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO groups (group_id, name, built_in, created_at, description, parent_group_id)
         VALUES (?1, ?2, 1, ?3, ?4, ?5)",
        rusqlite::params![group_id, name, now, description, parent_group_id],
    )?;
    // Update parent if group already existed (migration path)
    if let Some(pid) = parent_group_id {
        conn.execute(
            "UPDATE groups SET parent_group_id = ?1 WHERE group_id = ?2 AND parent_group_id IS NULL",
            rusqlite::params![pid, group_id],
        )?;
    }
    Ok(())
}

fn seed_capability_rules(
    conn: &Connection,
    group_id: &str,
    capabilities: &[&str],
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO capability_rules (group_id, capability_name, allow)
         VALUES (?1, ?2, 1)",
    )?;
    for cap in capabilities {
        stmt.execute(rusqlite::params![group_id, cap])?;
    }
    Ok(())
}

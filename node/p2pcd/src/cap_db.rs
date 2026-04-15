//! SQLite bootstrap helpers for capabilities.
//!
//! Every Howm capability that persists state (feed, files, messaging) used to
//! paste the same ~10 lines to open a rusqlite connection:
//!
//! ```no_run
//! # use std::path::Path;
//! # fn main() -> anyhow::Result<()> {
//! let path = Path::new("./data.db");
//! std::fs::create_dir_all(path.parent().unwrap())?;
//! let conn = rusqlite::Connection::open(path)?;
//! conn.execute_batch(
//!     "PRAGMA journal_mode = WAL;
//!      PRAGMA busy_timeout = 5000;
//!      PRAGMA foreign_keys = ON;",
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! This module collapses that into a single [`open_sqlite`] call. Caps still
//! own their schema migrations — this helper only handles opening the file
//! and setting the standard pragmas.
//!
//! Behind the `cap-db` feature so caps without a datastore (presence, voice)
//! don't have to compile rusqlite.

use std::path::Path;

use rusqlite::Connection;

/// Open a SQLite database at `path` with the Howm standard pragmas.
///
/// Creates the parent directory if it does not already exist, opens the file
/// (creating it if missing), and applies:
///
/// - `journal_mode = WAL` — enables concurrent reads during writes
/// - `busy_timeout = 5000` — waits up to 5s on lock contention instead of
///   returning `SQLITE_BUSY`
/// - `foreign_keys = ON` — enforces FK constraints (off by default in SQLite)
///
/// Caps should run their schema / migration code on the returned connection
/// themselves.
///
/// # Errors
///
/// Returns an error if the parent directory cannot be created, the file
/// cannot be opened, or any of the pragmas fail to apply.
pub fn open_sqlite(path: &Path) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!("cap_db: failed to create parent dir {:?}: {}", parent, e)
            })?;
        }
    }

    let conn = Connection::open(path)
        .map_err(|e| anyhow::anyhow!("cap_db: failed to open {:?}: {}", path, e))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )
    .map_err(|e| anyhow::anyhow!("cap_db: failed to apply pragmas: {}", e))?;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_sqlite_creates_file_and_applies_pragmas() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("nested").join("test.db");
        let conn = open_sqlite(&db_path).unwrap();

        assert!(db_path.exists(), "db file should be created");

        // Verify pragmas are set
        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "wal");

        let busy: i64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5000);

        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn open_sqlite_reuses_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");

        {
            let conn = open_sqlite(&db_path).unwrap();
            conn.execute("CREATE TABLE foo (id INTEGER)", []).unwrap();
            conn.execute("INSERT INTO foo VALUES (42)", []).unwrap();
        }

        let conn = open_sqlite(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM foo", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}

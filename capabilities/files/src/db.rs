use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_memory_creates_tables() {
        let db = FilesDb::open_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        // Verify offerings table exists
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM offerings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        // Verify downloads table exists
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM downloads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

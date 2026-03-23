// SQLite storage for social feed posts and attachments.
//
// Replaces the JSON flat-file storage (posts.json, peer_posts.json) with a
// single SQLite database at $DATA_DIR/social_feed.db. WAL mode for concurrent
// reads from SSE/status endpoints while writes happen on the ingest path.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use serde::Serialize;

use crate::posts::{Attachment, Post};

/// A blob transfer record — tracks download progress for inbound post attachments.
#[derive(Debug, Clone, Serialize)]
pub struct BlobTransfer {
    pub post_id: String,
    pub blob_id: String,
    /// One of: pending, fetching, complete, failed.
    pub status: String,
    pub bytes_received: u64,
    pub updated_at: u64,
    /// MIME type from the attachment record.
    pub mime_type: String,
    /// Total expected size from the attachment record.
    pub total_size: u64,
}

// ── Database ─────────────────────────────────────────────────────────────────

/// Thread-safe SQLite handle for social feed storage.
#[derive(Clone)]
pub struct FeedDb {
    conn: Arc<Mutex<Connection>>,
}

impl FeedDb {
    /// Open (or create) the social feed database in the given directory.
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let db_path = data_dir.join("social_feed.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory database (for tests).
    #[cfg(test)]
    pub fn open_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn migrate(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS posts (
                id            TEXT PRIMARY KEY,
                author_id     TEXT NOT NULL,
                author_name   TEXT NOT NULL,
                content       TEXT NOT NULL,
                timestamp     INTEGER NOT NULL,
                origin        TEXT NOT NULL DEFAULT 'local'
            );
            CREATE INDEX IF NOT EXISTS idx_posts_timestamp ON posts(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_posts_origin ON posts(origin);

            CREATE TABLE IF NOT EXISTS attachments (
                post_id       TEXT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
                blob_id       TEXT NOT NULL,
                mime_type     TEXT NOT NULL,
                size          INTEGER NOT NULL,
                position      INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (post_id, blob_id)
            );

            CREATE TABLE IF NOT EXISTS blob_transfers (
                post_id        TEXT NOT NULL,
                blob_id        TEXT NOT NULL,
                status         TEXT NOT NULL DEFAULT 'pending',
                bytes_received INTEGER NOT NULL DEFAULT 0,
                updated_at     INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                PRIMARY KEY (post_id, blob_id),
                FOREIGN KEY (post_id, blob_id)
                    REFERENCES attachments(post_id, blob_id) ON DELETE CASCADE
            );",
        )?;
        Ok(())
    }

    // ── Posts ─────────────────────────────────────────────────────────────────

    /// Insert a new post with optional attachments. Returns false if a post
    /// with the same ID already exists (dedup).
    pub fn insert_post(&self, post: &Post) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        let inserted = tx.execute(
            "INSERT OR IGNORE INTO posts (id, author_id, author_name, content, timestamp, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                post.id,
                post.author_id,
                post.author_name,
                post.content,
                post.timestamp,
                post.origin,
            ],
        )?;

        if inserted == 0 {
            return Ok(false); // duplicate
        }

        for (i, att) in post.attachments.iter().enumerate() {
            tx.execute(
                "INSERT OR IGNORE INTO attachments (post_id, blob_id, mime_type, size, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![post.id, att.blob_id, att.mime_type, att.size, i as i64],
            )?;
        }

        tx.commit()?;
        Ok(true)
    }

    /// Delete a post by ID. Only deletes if the origin matches the filter.
    /// Pass `None` to delete regardless of origin.
    /// Returns true if a row was removed.
    pub fn delete_post(&self, post_id: &str, origin_filter: Option<&str>) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let removed = match origin_filter {
            Some("local") => conn.execute(
                "DELETE FROM posts WHERE id = ?1 AND origin = 'local'",
                params![post_id],
            )?,
            Some(prefix) => conn.execute(
                "DELETE FROM posts WHERE id = ?1 AND origin LIKE ?2",
                params![post_id, format!("{}%", prefix)],
            )?,
            None => conn.execute("DELETE FROM posts WHERE id = ?1", params![post_id])?,
        };
        Ok(removed > 0)
    }

    /// Load all posts (local + peer), sorted newest first, with pagination.
    pub fn load_all(&self, limit: usize, offset: usize) -> anyhow::Result<(Vec<Post>, usize)> {
        let conn = self.conn.lock().unwrap();
        let total: usize = conn.query_row("SELECT COUNT(*) FROM posts", [], |r| r.get(0))?;
        let posts = Self::query_posts(
            &conn,
            "SELECT id, author_id, author_name, content, timestamp, origin
             FROM posts ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2",
            params![limit as i64, offset as i64],
        )?;
        let posts = Self::attach_attachments(&conn, posts)?;
        Ok((posts, total))
    }

    /// Load only local posts, sorted newest first, with pagination.
    pub fn load_mine(&self, limit: usize, offset: usize) -> anyhow::Result<(Vec<Post>, usize)> {
        let conn = self.conn.lock().unwrap();
        let total: usize = conn.query_row(
            "SELECT COUNT(*) FROM posts WHERE origin = 'local'",
            [],
            |r| r.get(0),
        )?;
        let posts = Self::query_posts(
            &conn,
            "SELECT id, author_id, author_name, content, timestamp, origin
             FROM posts WHERE origin = 'local' ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2",
            params![limit as i64, offset as i64],
        )?;
        let posts = Self::attach_attachments(&conn, posts)?;
        Ok((posts, total))
    }

    /// Load posts from a specific peer, sorted newest first, with pagination.
    pub fn load_peer_feed(
        &self,
        peer_id: &str,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<(Vec<Post>, usize)> {
        let origin = format!("peer:{}", peer_id);
        let conn = self.conn.lock().unwrap();
        let total: usize = conn.query_row(
            "SELECT COUNT(*) FROM posts WHERE origin = ?1",
            params![origin],
            |r| r.get(0),
        )?;
        let posts = Self::query_posts(
            &conn,
            "SELECT id, author_id, author_name, content, timestamp, origin
             FROM posts WHERE origin = ?1 ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3",
            params![origin, limit as i64, offset as i64],
        )?;
        let posts = Self::attach_attachments(&conn, posts)?;
        Ok((posts, total))
    }

    /// Check if a post ID exists.
    pub fn post_exists(&self, post_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM posts WHERE id = ?1)",
            params![post_id],
            |r| r.get(0),
        )?;
        Ok(exists)
    }

    // ── JSON migration ───────────────────────────────────────────────────────

    /// One-time import of posts.json and peer_posts.json into SQLite.
    /// Renames imported files to .json.migrated so it only runs once.
    pub fn migrate_from_json(&self, data_dir: &Path) -> anyhow::Result<()> {
        let posts_path = data_dir.join("posts.json");
        let peer_path = data_dir.join("peer_posts.json");

        if posts_path.exists() {
            let text = std::fs::read_to_string(&posts_path)?;
            let posts: Vec<Post> = serde_json::from_str(&text).unwrap_or_default();
            for post in &posts {
                self.insert_post(post)?;
            }
            std::fs::rename(&posts_path, data_dir.join("posts.json.migrated"))?;
            tracing::info!("migrated {} local posts from posts.json", posts.len());
        }

        if peer_path.exists() {
            let text = std::fs::read_to_string(&peer_path)?;
            let posts: Vec<Post> = serde_json::from_str(&text).unwrap_or_default();
            for post in &posts {
                self.insert_post(post)?;
            }
            std::fs::rename(&peer_path, data_dir.join("peer_posts.json.migrated"))?;
            tracing::info!("migrated {} peer posts from peer_posts.json", posts.len());
        }

        Ok(())
    }

    // ── Blob transfers ────────────────────────────────────────────────────────

    /// Insert a pending blob transfer record for an inbound post attachment.
    pub fn insert_blob_transfer(&self, post_id: &str, blob_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO blob_transfers (post_id, blob_id, status, bytes_received, updated_at)
             VALUES (?1, ?2, 'pending', 0, strftime('%s','now'))",
            params![post_id, blob_id],
        )?;
        Ok(())
    }

    /// Update the status of a blob transfer.
    /// Valid statuses: pending, fetching, complete, failed.
    pub fn update_blob_transfer(
        &self,
        post_id: &str,
        blob_id: &str,
        status: &str,
        bytes_received: u64,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE blob_transfers SET status = ?3, bytes_received = ?4,
             updated_at = strftime('%s','now')
             WHERE post_id = ?1 AND blob_id = ?2",
            params![post_id, blob_id, status, bytes_received],
        )?;
        Ok(())
    }

    /// Get all blob transfer records for a post.
    pub fn get_post_transfers(&self, post_id: &str) -> anyhow::Result<Vec<BlobTransfer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT bt.post_id, bt.blob_id, bt.status, bt.bytes_received, bt.updated_at,
                    a.mime_type, a.size
             FROM blob_transfers bt
             JOIN attachments a ON bt.post_id = a.post_id AND bt.blob_id = a.blob_id
             WHERE bt.post_id = ?1
             ORDER BY a.position ASC",
        )?;
        let rows = stmt.query_map(params![post_id], |row| {
            Ok(BlobTransfer {
                post_id: row.get(0)?,
                blob_id: row.get(1)?,
                status: row.get(2)?,
                bytes_received: row.get(3)?,
                updated_at: row.get(4)?,
                mime_type: row.get(5)?,
                total_size: row.get(6)?,
            })
        })?;
        let mut transfers = Vec::new();
        for row in rows {
            transfers.push(row?);
        }
        Ok(transfers)
    }

    /// Get all pending or fetching transfers (for startup recovery / polling).
    pub fn get_active_transfers(&self) -> anyhow::Result<Vec<BlobTransfer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT bt.post_id, bt.blob_id, bt.status, bt.bytes_received, bt.updated_at,
                    a.mime_type, a.size
             FROM blob_transfers bt
             JOIN attachments a ON bt.post_id = a.post_id AND bt.blob_id = a.blob_id
             WHERE bt.status IN ('pending', 'fetching')
             ORDER BY bt.updated_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(BlobTransfer {
                post_id: row.get(0)?,
                blob_id: row.get(1)?,
                status: row.get(2)?,
                bytes_received: row.get(3)?,
                updated_at: row.get(4)?,
                mime_type: row.get(5)?,
                total_size: row.get(6)?,
            })
        })?;
        let mut transfers = Vec::new();
        for row in rows {
            transfers.push(row?);
        }
        Ok(transfers)
    }

    /// Check if all blob transfers for a post are complete.
    pub fn are_all_transfers_complete(&self, post_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let incomplete: i64 = conn.query_row(
            "SELECT COUNT(*) FROM blob_transfers WHERE post_id = ?1 AND status != 'complete'",
            params![post_id],
            |r| r.get(0),
        )?;
        Ok(incomplete == 0)
    }

    /// Get the origin (peer_id) for a post. Returns the raw origin string.
    pub fn get_post_origin(&self, post_id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let origin: Option<String> = conn
            .query_row(
                "SELECT origin FROM posts WHERE id = ?1",
                params![post_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(origin)
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn query_posts(
        conn: &Connection,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> anyhow::Result<Vec<Post>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params, |row| {
            Ok(Post {
                id: row.get(0)?,
                author_id: row.get(1)?,
                author_name: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                origin: row.get(5)?,
                attachments: vec![], // filled by attach_attachments
            })
        })?;
        let mut posts = Vec::new();
        for row in rows {
            posts.push(row?);
        }
        Ok(posts)
    }

    /// Load attachments for a batch of posts and attach them in place.
    fn attach_attachments(conn: &Connection, mut posts: Vec<Post>) -> anyhow::Result<Vec<Post>> {
        if posts.is_empty() {
            return Ok(posts);
        }

        // For small batches, query per-post. For large batches, a single query
        // with IN clause would be faster, but pagination keeps batches small.
        for post in &mut posts {
            let mut stmt = conn.prepare(
                "SELECT blob_id, mime_type, size FROM attachments
                 WHERE post_id = ?1 ORDER BY position ASC",
            )?;
            let atts = stmt.query_map(params![post.id], |row| {
                Ok(Attachment {
                    blob_id: row.get(0)?,
                    mime_type: row.get(1)?,
                    size: row.get(2)?,
                })
            })?;
            for att in atts {
                post.attachments.push(att?);
            }
        }
        Ok(posts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts;

    fn make_post(id: &str, origin: &str) -> Post {
        Post {
            id: id.to_string(),
            author_id: "alice".to_string(),
            author_name: "Alice".to_string(),
            content: "hello world".to_string(),
            timestamp: 1000,
            origin: origin.to_string(),
            attachments: vec![],
        }
    }

    #[test]
    fn insert_and_load_all() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post("p1", "local");
        assert!(db.insert_post(&post).unwrap());

        let (posts, total) = db.load_all(50, 0).unwrap();
        assert_eq!(total, 1);
        assert_eq!(posts[0].id, "p1");
        assert_eq!(posts[0].origin, "local");
    }

    #[test]
    fn insert_dedup() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post("p1", "local");
        assert!(db.insert_post(&post).unwrap());
        assert!(!db.insert_post(&post).unwrap()); // duplicate
        let (_, total) = db.load_all(50, 0).unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn insert_with_attachments() {
        let db = FeedDb::open_memory().unwrap();
        let post = Post {
            id: "p1".to_string(),
            author_id: "alice".to_string(),
            author_name: "Alice".to_string(),
            content: "photo post".to_string(),
            timestamp: 2000,
            origin: "local".to_string(),
            attachments: vec![
                Attachment {
                    blob_id: "aabbccdd".to_string(),
                    mime_type: "image/jpeg".to_string(),
                    size: 1048576,
                },
                Attachment {
                    blob_id: "eeff0011".to_string(),
                    mime_type: "image/png".to_string(),
                    size: 524288,
                },
            ],
        };
        assert!(db.insert_post(&post).unwrap());

        let (posts, _) = db.load_all(50, 0).unwrap();
        assert_eq!(posts[0].attachments.len(), 2);
        assert_eq!(posts[0].attachments[0].blob_id, "aabbccdd");
        assert_eq!(posts[0].attachments[1].blob_id, "eeff0011");
        assert_eq!(posts[0].attachments[0].size, 1048576);
    }

    #[test]
    fn delete_local_post() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&make_post("p1", "local")).unwrap();
        assert!(db.delete_post("p1", Some("local")).unwrap());
        assert!(!db.delete_post("p1", Some("local")).unwrap()); // already gone
        let (_, total) = db.load_all(50, 0).unwrap();
        assert_eq!(total, 0);
    }

    #[test]
    fn delete_peer_post() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&make_post("p1", "peer:AAAA")).unwrap();
        // Can't delete peer post with local filter
        assert!(!db.delete_post("p1", Some("local")).unwrap());
        // Can delete with peer filter
        assert!(db.delete_post("p1", Some("peer:")).unwrap());
        let (_, total) = db.load_all(50, 0).unwrap();
        assert_eq!(total, 0);
    }

    #[test]
    fn delete_cascades_attachments() {
        let db = FeedDb::open_memory().unwrap();
        let post = Post {
            attachments: vec![Attachment {
                blob_id: "aabb".to_string(),
                mime_type: "image/jpeg".to_string(),
                size: 100,
            }],
            ..make_post("p1", "local")
        };
        db.insert_post(&post).unwrap();
        db.delete_post("p1", None).unwrap();

        // Verify attachment is gone too
        let (posts, _) = db.load_all(50, 0).unwrap();
        assert!(posts.is_empty());
    }

    #[test]
    fn load_mine_only_local() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&make_post("p1", "local")).unwrap();
        db.insert_post(&make_post("p2", "peer:BBBB")).unwrap();

        let (mine, total) = db.load_mine(50, 0).unwrap();
        assert_eq!(total, 1);
        assert_eq!(mine[0].id, "p1");
    }

    #[test]
    fn load_peer_feed_filters() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&Post {
            timestamp: 100,
            ..make_post("p1", "peer:ALICE")
        })
        .unwrap();
        db.insert_post(&Post {
            timestamp: 200,
            ..make_post("p2", "peer:BOB")
        })
        .unwrap();

        let (alice, total) = db.load_peer_feed("ALICE", 50, 0).unwrap();
        assert_eq!(total, 1);
        assert_eq!(alice[0].id, "p1");

        let (bob, _) = db.load_peer_feed("BOB", 50, 0).unwrap();
        assert_eq!(bob[0].id, "p2");

        let (nobody, total) = db.load_peer_feed("NOBODY", 50, 0).unwrap();
        assert!(nobody.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn load_all_sorts_newest_first() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&Post {
            timestamp: 100,
            ..make_post("old", "local")
        })
        .unwrap();
        db.insert_post(&Post {
            timestamp: 300,
            ..make_post("new", "peer:CCCC")
        })
        .unwrap();
        db.insert_post(&Post {
            timestamp: 200,
            ..make_post("mid", "local")
        })
        .unwrap();

        let (posts, _) = db.load_all(50, 0).unwrap();
        assert_eq!(posts[0].id, "new");
        assert_eq!(posts[1].id, "mid");
        assert_eq!(posts[2].id, "old");
    }

    #[test]
    fn pagination() {
        let db = FeedDb::open_memory().unwrap();
        for i in 0..10 {
            db.insert_post(&Post {
                timestamp: i as u64,
                ..make_post(&format!("p{}", i), "local")
            })
            .unwrap();
        }

        let (page1, total) = db.load_all(3, 0).unwrap();
        assert_eq!(total, 10);
        assert_eq!(page1.len(), 3);

        let (page2, _) = db.load_all(3, 3).unwrap();
        assert_eq!(page2.len(), 3);

        // No overlap
        assert_ne!(page1[0].id, page2[0].id);

        // Past the end
        let (empty, _) = db.load_all(50, 100).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn post_exists_check() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&make_post("p1", "local")).unwrap();
        assert!(db.post_exists("p1").unwrap());
        assert!(!db.post_exists("p999").unwrap());
    }

    #[test]
    fn json_migration() {
        let dir = tempfile::TempDir::new().unwrap();

        // Write legacy JSON files
        let local_posts = vec![make_post("local1", "local")];
        std::fs::write(
            dir.path().join("posts.json"),
            serde_json::to_string(&local_posts).unwrap(),
        )
        .unwrap();

        let peer_posts = vec![make_post("peer1", "peer:DDDD")];
        std::fs::write(
            dir.path().join("peer_posts.json"),
            serde_json::to_string(&peer_posts).unwrap(),
        )
        .unwrap();

        let db = FeedDb::open(dir.path()).unwrap();
        db.migrate_from_json(dir.path()).unwrap();

        // Posts imported
        let (all, total) = db.load_all(50, 0).unwrap();
        assert_eq!(total, 2);

        // JSON files renamed
        assert!(!dir.path().join("posts.json").exists());
        assert!(dir.path().join("posts.json.migrated").exists());
        assert!(!dir.path().join("peer_posts.json").exists());
        assert!(dir.path().join("peer_posts.json.migrated").exists());

        // Idempotent — running again does nothing
        db.migrate_from_json(dir.path()).unwrap();
        let (_, total2) = db.load_all(50, 0).unwrap();
        assert_eq!(total2, 2);
    }

    #[test]
    fn json_migration_no_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = FeedDb::open(dir.path()).unwrap();
        // Should not error when no JSON files exist
        db.migrate_from_json(dir.path()).unwrap();
    }

    #[test]
    fn backward_compat_no_attachments_field() {
        // Simulate old JSON without attachments field
        let json = r#"{"id":"old","author_id":"a","author_name":"A","content":"hi","timestamp":1}"#;
        let post: Post = serde_json::from_str(json).unwrap();
        assert!(post.attachments.is_empty());
        assert_eq!(post.origin, "local"); // default

        let db = FeedDb::open_memory().unwrap();
        assert!(db.insert_post(&post).unwrap());
        let (posts, _) = db.load_all(50, 0).unwrap();
        assert_eq!(posts[0].content, "hi");
        assert!(posts[0].attachments.is_empty());
    }

    // ── Blob transfer tests ────────────────────────────────────────────────

    fn make_post_with_attachments(id: &str, origin: &str) -> Post {
        Post {
            id: id.to_string(),
            author_id: "alice".to_string(),
            author_name: "Alice".to_string(),
            content: "media post".to_string(),
            timestamp: 1000,
            origin: origin.to_string(),
            attachments: vec![
                Attachment {
                    blob_id: "aabb0011".to_string(),
                    mime_type: "image/jpeg".to_string(),
                    size: 100_000,
                },
                Attachment {
                    blob_id: "ccdd2233".to_string(),
                    mime_type: "image/png".to_string(),
                    size: 200_000,
                },
            ],
        }
    }

    #[test]
    fn blob_transfer_insert_and_query() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:AAAA");
        db.insert_post(&post).unwrap();

        // Insert transfer records
        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        db.insert_blob_transfer("p1", "ccdd2233").unwrap();

        // Query transfers
        let transfers = db.get_post_transfers("p1").unwrap();
        assert_eq!(transfers.len(), 2);
        assert_eq!(transfers[0].blob_id, "aabb0011");
        assert_eq!(transfers[0].status, "pending");
        assert_eq!(transfers[0].bytes_received, 0);
        assert_eq!(transfers[0].mime_type, "image/jpeg");
        assert_eq!(transfers[0].total_size, 100_000);
        assert_eq!(transfers[1].blob_id, "ccdd2233");
    }

    #[test]
    fn blob_transfer_update_status() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:BBBB");
        db.insert_post(&post).unwrap();

        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        db.update_blob_transfer("p1", "aabb0011", "fetching", 50_000)
            .unwrap();

        let transfers = db.get_post_transfers("p1").unwrap();
        assert_eq!(transfers[0].status, "fetching");
        assert_eq!(transfers[0].bytes_received, 50_000);
    }

    #[test]
    fn blob_transfer_are_all_complete() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:CCCC");
        db.insert_post(&post).unwrap();

        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        db.insert_blob_transfer("p1", "ccdd2233").unwrap();

        // Not all complete yet
        assert!(!db.are_all_transfers_complete("p1").unwrap());

        // Complete one
        db.update_blob_transfer("p1", "aabb0011", "complete", 100_000)
            .unwrap();
        assert!(!db.are_all_transfers_complete("p1").unwrap());

        // Complete both
        db.update_blob_transfer("p1", "ccdd2233", "complete", 200_000)
            .unwrap();
        assert!(db.are_all_transfers_complete("p1").unwrap());
    }

    #[test]
    fn blob_transfer_get_active() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:DDDD");
        db.insert_post(&post).unwrap();

        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        db.insert_blob_transfer("p1", "ccdd2233").unwrap();

        // Both pending — both active
        let active = db.get_active_transfers().unwrap();
        assert_eq!(active.len(), 2);

        // Complete one, mark other fetching
        db.update_blob_transfer("p1", "aabb0011", "complete", 100_000)
            .unwrap();
        db.update_blob_transfer("p1", "ccdd2233", "fetching", 50_000)
            .unwrap();

        let active = db.get_active_transfers().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].blob_id, "ccdd2233");

        // Complete the last one
        db.update_blob_transfer("p1", "ccdd2233", "complete", 200_000)
            .unwrap();
        let active = db.get_active_transfers().unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn blob_transfer_failed_not_active() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:EEEE");
        db.insert_post(&post).unwrap();

        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        db.update_blob_transfer("p1", "aabb0011", "failed", 0)
            .unwrap();

        let active = db.get_active_transfers().unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn blob_transfer_dedup_insert() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:FFFF");
        db.insert_post(&post).unwrap();

        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        db.update_blob_transfer("p1", "aabb0011", "fetching", 50_000)
            .unwrap();

        // Re-insert should be ignored (INSERT OR IGNORE)
        db.insert_blob_transfer("p1", "aabb0011").unwrap();
        let transfers = db.get_post_transfers("p1").unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].status, "fetching"); // not reset to pending
    }

    #[test]
    fn get_post_origin() {
        let db = FeedDb::open_memory().unwrap();
        db.insert_post(&make_post("p1", "peer:GGGG")).unwrap();

        assert_eq!(
            db.get_post_origin("p1").unwrap(),
            Some("peer:GGGG".to_string())
        );
        assert_eq!(db.get_post_origin("nonexistent").unwrap(), None);
    }

    #[test]
    fn blob_transfer_cascades_on_post_delete() {
        let db = FeedDb::open_memory().unwrap();
        let post = make_post_with_attachments("p1", "peer:HHHH");
        db.insert_post(&post).unwrap();
        db.insert_blob_transfer("p1", "aabb0011").unwrap();

        db.delete_post("p1", None).unwrap();

        // Transfer record should be gone (cascade via attachment FK)
        let transfers = db.get_post_transfers("p1").unwrap();
        assert!(transfers.is_empty());
    }
}

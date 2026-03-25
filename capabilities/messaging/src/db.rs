use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Local message storage backed by SQLite.
pub struct MessageDb {
    conn: Mutex<Connection>,
}

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    /// UUIDv7 as hex string (32 chars).
    pub msg_id: String,
    pub conversation_id: String,
    /// "sent" or "received"
    pub direction: String,
    /// Sender WG pubkey as base64.
    pub sender_peer_id: String,
    /// Unix epoch milliseconds.
    pub sent_at: i64,
    pub body: String,
    /// "pending", "delivered", or "failed"
    pub delivery_status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationSummary {
    pub conversation_id: String,
    /// The *other* peer's ID (base64 WG pubkey).
    pub peer_id: String,
    pub last_message: Option<LastMessage>,
    pub unread_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LastMessage {
    pub msg_id: String,
    pub body_preview: String,
    pub sent_at: i64,
    pub direction: String,
}

// ── Implementation ───────────────────────────────────────────────────────────

impl MessageDb {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let db_path = data_dir.join("messaging.db");
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;",
        )?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn migrate(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                msg_id          TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                direction       TEXT NOT NULL,
                sender_peer_id  TEXT NOT NULL,
                sent_at         INTEGER NOT NULL,
                body            TEXT NOT NULL,
                delivery_status TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE INDEX IF NOT EXISTS idx_messages_conv
                ON messages(conversation_id, sent_at);

            CREATE TABLE IF NOT EXISTS read_markers (
                conversation_id TEXT PRIMARY KEY,
                read_at         INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    // ── Conversation ID derivation ───────────────────────────────────────────

    /// Derive a deterministic conversation_id from two peer IDs.
    ///
    /// SHA-256 of sorted concatenation of raw 32-byte peer IDs.
    /// Input: base64-encoded WG pubkeys. Output: 64-char hex.
    pub fn conversation_id(peer_a: &str, peer_b: &str) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let mut a_bytes = STANDARD.decode(peer_a).unwrap_or_default();
        let mut b_bytes = STANDARD.decode(peer_b).unwrap_or_default();

        // Sort so the result is the same regardless of direction
        if a_bytes > b_bytes {
            std::mem::swap(&mut a_bytes, &mut b_bytes);
        }

        let mut hasher = Sha256::new();
        hasher.update(&a_bytes);
        hasher.update(&b_bytes);
        hex::encode(hasher.finalize())
    }

    // ── Insert ───────────────────────────────────────────────────────────────

    pub fn insert_message(&self, msg: &Message) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO messages (msg_id, conversation_id, direction, sender_peer_id, sent_at, body, delivery_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.msg_id,
                msg.conversation_id,
                msg.direction,
                msg.sender_peer_id,
                msg.sent_at,
                msg.body,
                msg.delivery_status,
            ],
        )?;
        Ok(())
    }

    // ── Update ───────────────────────────────────────────────────────────────

    pub fn update_delivery_status(&self, msg_id: &str, status: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE messages SET delivery_status = ?1 WHERE msg_id = ?2",
            params![status, msg_id],
        )?;
        Ok(())
    }

    /// Transition all pending messages to a peer to 'failed'.
    pub fn fail_pending_to_peer(&self, conversation_id: &str, reason: &str) -> anyhow::Result<u64> {
        let _ = reason; // stored in delivery_status as "failed"
        let conn = self.conn.lock();
        let count = conn.execute(
            "UPDATE messages SET delivery_status = 'failed'
             WHERE conversation_id = ?1 AND direction = 'sent' AND delivery_status = 'pending'",
            params![conversation_id],
        )?;
        Ok(count as u64)
    }

    // ── Query ────────────────────────────────────────────────────────────────

    /// Get paginated messages for a conversation, ordered by sent_at ascending.
    /// cursor = sent_at of the last message from the previous page (exclusive).
    pub fn get_conversation(
        &self,
        conversation_id: &str,
        cursor: Option<i64>,
        limit: i64,
    ) -> anyhow::Result<(Vec<Message>, Option<i64>)> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT msg_id, conversation_id, direction, sender_peer_id, sent_at, body, delivery_status
             FROM messages
             WHERE conversation_id = ?1 AND sent_at > ?2
             ORDER BY sent_at ASC
             LIMIT ?3",
        )?;

        let cursor_val = cursor.unwrap_or(0);
        let messages: Vec<Message> = stmt
            .query_map(params![conversation_id, cursor_val, limit + 1], |row| {
                Ok(Message {
                    msg_id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    direction: row.get(2)?,
                    sender_peer_id: row.get(3)?,
                    sent_at: row.get(4)?,
                    body: row.get(5)?,
                    delivery_status: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // If we got limit+1 rows, there's a next page
        if messages.len() as i64 > limit {
            let trimmed: Vec<Message> = messages[..limit as usize].to_vec();
            let next_cursor = trimmed.last().map(|m| m.sent_at);
            Ok((trimmed, next_cursor))
        } else {
            Ok((messages, None))
        }
    }

    /// List conversations with last message and unread count.
    pub fn list_conversations(
        &self,
        local_peer_id: &str,
    ) -> anyhow::Result<Vec<ConversationSummary>> {
        let conn = self.conn.lock();

        // Get distinct conversation_ids with their latest message
        let mut stmt = conn.prepare(
            "SELECT m.conversation_id, m.msg_id, m.body, m.sent_at, m.direction, m.sender_peer_id
             FROM messages m
             INNER JOIN (
                 SELECT conversation_id, MAX(sent_at) as max_sent
                 FROM messages
                 GROUP BY conversation_id
             ) latest ON m.conversation_id = latest.conversation_id AND m.sent_at = latest.max_sent
             ORDER BY m.sent_at DESC",
        )?;

        let rows: Vec<(String, String, String, i64, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut summaries = Vec::new();
        for (conv_id, msg_id, body, sent_at, direction, sender_peer_id) in rows {
            let unread = self.unread_count_inner(&conn, &conv_id)?;

            // Figure out the other peer's ID
            // For sent messages, sender is us; we need to find a received message for the peer ID
            // For received messages, sender is the peer
            let peer_id = if direction == "received" {
                sender_peer_id.clone()
            } else {
                // Find any received message in this conversation to get the peer ID
                let peer: Option<String> = conn
                    .query_row(
                        "SELECT sender_peer_id FROM messages WHERE conversation_id = ?1 AND direction = 'received' LIMIT 1",
                        params![conv_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                peer.unwrap_or_else(|| {
                    // All messages are sent — we need to derive peer from conversation_id
                    // This is a fallback; in practice there should be received messages
                    local_peer_id.to_string()
                })
            };

            let preview = if body.len() > 128 {
                format!("{}…", &body[..128])
            } else {
                body.clone()
            };

            summaries.push(ConversationSummary {
                conversation_id: conv_id,
                peer_id,
                last_message: Some(LastMessage {
                    msg_id,
                    body_preview: preview,
                    sent_at,
                    direction,
                }),
                unread_count: unread,
            });
        }

        Ok(summaries)
    }

    fn unread_count_inner(&self, conn: &Connection, conversation_id: &str) -> anyhow::Result<i64> {
        let read_at: i64 = conn
            .query_row(
                "SELECT read_at FROM read_markers WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE conversation_id = ?1 AND direction = 'received' AND sent_at > ?2",
            params![conversation_id, read_at],
            |row| row.get(0),
        )?;

        Ok(count)
    }

    // ── Mark read ────────────────────────────────────────────────────────────

    pub fn mark_read(&self, conversation_id: &str) -> anyhow::Result<()> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO read_markers (conversation_id, read_at) VALUES (?1, ?2)
             ON CONFLICT(conversation_id) DO UPDATE SET read_at = ?2",
            params![conversation_id, now_ms],
        )?;
        Ok(())
    }

    // ── Delete ───────────────────────────────────────────────────────────────

    /// Delete a message. Only the sender can delete their own sent messages.
    pub fn delete_message(&self, msg_id: &str, local_peer_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock();

        // Check that the message exists and was sent by us
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT direction, sender_peer_id FROM messages WHERE msg_id = ?1",
                params![msg_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        match row {
            Some((direction, sender)) if direction == "sent" && sender == local_peer_id => {
                conn.execute("DELETE FROM messages WHERE msg_id = ?1", params![msg_id])?;
                Ok(true)
            }
            Some(_) => Ok(false), // Not sender or not a sent message
            None => Ok(false),    // Not found
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (MessageDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = MessageDb::open(dir.path()).unwrap();
        (db, dir)
    }

    #[test]
    fn conversation_id_is_deterministic() {
        let a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
        assert_eq!(
            MessageDb::conversation_id(a, b),
            MessageDb::conversation_id(b, a),
        );
    }

    #[test]
    fn insert_and_query() {
        let (db, _dir) = setup();
        let conv = MessageDb::conversation_id("cGVlcjE=", "cGVlcjI=");

        for i in 0..5 {
            db.insert_message(&Message {
                msg_id: format!("msg-{}", i),
                conversation_id: conv.clone(),
                direction: if i % 2 == 0 { "sent" } else { "received" }.into(),
                sender_peer_id: if i % 2 == 0 { "cGVlcjE=" } else { "cGVlcjI=" }.into(),
                sent_at: 1000 + i,
                body: format!("hello {}", i),
                delivery_status: "delivered".into(),
            })
            .unwrap();
        }

        let (msgs, cursor) = db.get_conversation(&conv, None, 50).unwrap();
        assert_eq!(msgs.len(), 5);
        assert!(cursor.is_none());
        assert_eq!(msgs[0].sent_at, 1000);
        assert_eq!(msgs[4].sent_at, 1004);
    }

    #[test]
    fn pagination() {
        let (db, _dir) = setup();
        let conv = MessageDb::conversation_id("cGVlcjE=", "cGVlcjI=");

        for i in 0..10 {
            db.insert_message(&Message {
                msg_id: format!("msg-{}", i),
                conversation_id: conv.clone(),
                direction: "sent".into(),
                sender_peer_id: "cGVlcjE=".into(),
                sent_at: 1000 + i,
                body: format!("msg {}", i),
                delivery_status: "delivered".into(),
            })
            .unwrap();
        }

        let (page1, cursor1) = db.get_conversation(&conv, None, 3).unwrap();
        assert_eq!(page1.len(), 3);
        assert!(cursor1.is_some());

        let (page2, cursor2) = db.get_conversation(&conv, cursor1, 3).unwrap();
        assert_eq!(page2.len(), 3);
        assert!(cursor2.is_some());

        let (page3, cursor3) = db.get_conversation(&conv, cursor2, 3).unwrap();
        assert_eq!(page3.len(), 3);
        assert!(cursor3.is_some());

        let (page4, cursor4) = db.get_conversation(&conv, cursor3, 3).unwrap();
        assert_eq!(page4.len(), 1);
        assert!(cursor4.is_none());
    }

    #[test]
    fn unread_count_and_mark_read() {
        let (db, _dir) = setup();
        let conv = MessageDb::conversation_id("cGVlcjE=", "cGVlcjI=");

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        for i in 0..3 {
            db.insert_message(&Message {
                msg_id: format!("msg-{}", i),
                conversation_id: conv.clone(),
                direction: "received".into(),
                sender_peer_id: "cGVlcjI=".into(),
                sent_at: now_ms - 3000 + (i * 1000),
                body: "hi".into(),
                delivery_status: "delivered".into(),
            })
            .unwrap();
        }

        // Check unread via list_conversations
        let convos = db.list_conversations("cGVlcjE=").unwrap();
        assert_eq!(convos.len(), 1);
        assert_eq!(convos[0].unread_count, 3);

        db.mark_read(&conv).unwrap();

        let convos = db.list_conversations("cGVlcjE=").unwrap();
        assert_eq!(convos[0].unread_count, 0);

        // One more after mark-read — must be in the future relative to mark_read's now()
        std::thread::sleep(std::time::Duration::from_millis(10));
        let future_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        db.insert_message(&Message {
            msg_id: "msg-3".into(),
            conversation_id: conv.clone(),
            direction: "received".into(),
            sender_peer_id: "cGVlcjI=".into(),
            sent_at: future_ms,
            body: "new".into(),
            delivery_status: "delivered".into(),
        })
        .unwrap();

        let convos = db.list_conversations("cGVlcjE=").unwrap();
        assert_eq!(convos[0].unread_count, 1);
    }

    #[test]
    fn delete_message_sender_only() {
        let (db, _dir) = setup();
        let local = "cGVlcjE=";
        let conv = MessageDb::conversation_id(local, "cGVlcjI=");

        db.insert_message(&Message {
            msg_id: "sent-1".into(),
            conversation_id: conv.clone(),
            direction: "sent".into(),
            sender_peer_id: local.into(),
            sent_at: 1000,
            body: "hello".into(),
            delivery_status: "delivered".into(),
        })
        .unwrap();

        db.insert_message(&Message {
            msg_id: "recv-1".into(),
            conversation_id: conv.clone(),
            direction: "received".into(),
            sender_peer_id: "cGVlcjI=".into(),
            sent_at: 1001,
            body: "hi back".into(),
            delivery_status: "delivered".into(),
        })
        .unwrap();

        // Can delete own sent message
        assert!(db.delete_message("sent-1", local).unwrap());

        // Cannot delete received message
        assert!(!db.delete_message("recv-1", local).unwrap());

        // Cannot delete nonexistent
        assert!(!db.delete_message("nope", local).unwrap());
    }

    #[test]
    fn delivery_status_transitions() {
        let (db, _dir) = setup();
        let conv = MessageDb::conversation_id("cGVlcjE=", "cGVlcjI=");

        db.insert_message(&Message {
            msg_id: "m1".into(),
            conversation_id: conv.clone(),
            direction: "sent".into(),
            sender_peer_id: "cGVlcjE=".into(),
            sent_at: 1000,
            body: "test".into(),
            delivery_status: "pending".into(),
        })
        .unwrap();

        db.update_delivery_status("m1", "delivered").unwrap();

        let (msgs, _) = db.get_conversation(&conv, None, 50).unwrap();
        assert_eq!(msgs[0].delivery_status, "delivered");
    }

    #[test]
    fn fail_pending_to_peer() {
        let (db, _dir) = setup();
        let conv = MessageDb::conversation_id("cGVlcjE=", "cGVlcjI=");

        for i in 0..3 {
            db.insert_message(&Message {
                msg_id: format!("m{}", i),
                conversation_id: conv.clone(),
                direction: "sent".into(),
                sender_peer_id: "cGVlcjE=".into(),
                sent_at: 1000 + i,
                body: "test".into(),
                delivery_status: "pending".into(),
            })
            .unwrap();
        }

        let failed = db.fail_pending_to_peer(&conv, "peer_offline").unwrap();
        assert_eq!(failed, 3);

        let (msgs, _) = db.get_conversation(&conv, None, 50).unwrap();
        assert!(msgs.iter().all(|m| m.delivery_status == "failed"));
    }
}

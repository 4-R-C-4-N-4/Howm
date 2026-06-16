use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub id: String,
    pub chain: String,
    pub label: Option<String>,
    pub address: String,
    pub created_at: i64,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub wallet_id: String,
    pub direction: String, // "in" | "out"
    pub chain_tx_id: Option<String>,
    pub from_addr: Option<String>,
    pub to_addr: Option<String>,
    pub amount: String,
    pub token: String,
    pub status: String, // "pending" | "confirmed" | "failed"
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
    pub invoice_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    pub wallet_id: String,
    pub amount: String,
    pub token: String,
    pub status: String, // "pending" | "paid" | "expired"
    pub peer_id: Option<String>,
    pub resource: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub paid_tx_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub peer_id: String,
    pub peer_name: Option<String>,
    pub capability: String,
    pub amount: String,
    pub token: String,
    pub chain: String,
    pub interval_secs: i64,
    pub status: String, // "active" | "cancelled" | "expired"
    pub current_grant_expires: Option<i64>,
    pub last_payment_tx: Option<String>,
    pub created_at: i64,
    pub cancelled_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub transaction_id: String,
    pub invoice_id: Option<String>,
    pub peer_id: String,
    pub peer_name: Option<String>,
    pub direction: String, // "sent" | "received"
    pub amount: String,
    pub token: String,
    pub description: Option<String>,
    pub resource_type: Option<String>, // "file" | "subscription" | "direct"
    pub resource_id: Option<String>,
    pub created_at: i64,
}

// ── Database handle ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct WalletDb {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Clone)]
pub struct SecretsDb {
    conn: Arc<Mutex<Connection>>,
}

impl WalletDb {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("wallet.db");
        let conn = Connection::open(&db_path)?;
        create_tables(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory database for testing.
    #[cfg(test)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        create_tables(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Wallets ──────────────────────────────────────────────────────────

    pub fn insert_wallet(&self, w: &Wallet) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO wallets (id, chain, label, address, created_at, is_default)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![w.id, w.chain, w.label, w.address, w.created_at, w.is_default],
        )?;
        Ok(())
    }

    pub fn list_wallets(&self) -> Result<Vec<Wallet>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, chain, label, address, created_at, is_default
             FROM wallets ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Wallet {
                id: row.get(0)?,
                chain: row.get(1)?,
                label: row.get(2)?,
                address: row.get(3)?,
                created_at: row.get(4)?,
                is_default: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get_wallet(&self, id: &str) -> Result<Option<Wallet>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, chain, label, address, created_at, is_default
             FROM wallets WHERE id = ?1",
            params![id],
            |row| {
                Ok(Wallet {
                    id: row.get(0)?,
                    chain: row.get(1)?,
                    label: row.get(2)?,
                    address: row.get(3)?,
                    created_at: row.get(4)?,
                    is_default: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_wallet(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute("DELETE FROM wallets WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }

    pub fn set_default_wallet(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE wallets SET is_default = 0", [])?;
        conn.execute(
            "UPDATE wallets SET is_default = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn get_default_wallet(&self) -> Result<Option<Wallet>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, chain, label, address, created_at, is_default
             FROM wallets WHERE is_default = 1",
            [],
            |row| {
                Ok(Wallet {
                    id: row.get(0)?,
                    chain: row.get(1)?,
                    label: row.get(2)?,
                    address: row.get(3)?,
                    created_at: row.get(4)?,
                    is_default: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    // ── Transactions ─────────────────────────────────────────────────────

    pub fn insert_transaction(&self, tx: &Transaction) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transactions (id, wallet_id, direction, chain_tx_id, from_addr,
             to_addr, amount, token, status, created_at, confirmed_at, invoice_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                tx.id,
                tx.wallet_id,
                tx.direction,
                tx.chain_tx_id,
                tx.from_addr,
                tx.to_addr,
                tx.amount,
                tx.token,
                tx.status,
                tx.created_at,
                tx.confirmed_at,
                tx.invoice_id,
            ],
        )?;
        Ok(())
    }

    pub fn list_transactions(
        &self,
        direction: Option<&str>,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Transaction>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, wallet_id, direction, chain_tx_id, from_addr, to_addr,
             amount, token, status, created_at, confirmed_at, invoice_id
             FROM transactions WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(d) = direction {
            sql.push_str(&format!(" AND direction = ?{}", param_values.len() + 1));
            param_values.push(Box::new(d.to_string()));
        }
        if let Some(s) = status {
            sql.push_str(&format!(" AND status = ?{}", param_values.len() + 1));
            param_values.push(Box::new(s.to_string()));
        }
        sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            param_values.len() + 1,
            param_values.len() + 2
        ));
        param_values.push(Box::new(limit));
        param_values.push(Box::new(offset));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), row_to_transaction)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get_transaction(&self, id: &str) -> Result<Option<Transaction>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, wallet_id, direction, chain_tx_id, from_addr, to_addr,
             amount, token, status, created_at, confirmed_at, invoice_id
             FROM transactions WHERE id = ?1",
            params![id],
            row_to_transaction,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn update_transaction_status(
        &self,
        id: &str,
        status: &str,
        chain_tx_id: Option<&str>,
        confirmed_at: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE transactions SET status = ?1, chain_tx_id = COALESCE(?2, chain_tx_id),
             confirmed_at = COALESCE(?3, confirmed_at) WHERE id = ?4",
            params![status, chain_tx_id, confirmed_at, id],
        )?;
        Ok(())
    }

    // ── Invoices ─────────────────────────────────────────────────────────

    pub fn insert_invoice(&self, inv: &Invoice) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO invoices (id, wallet_id, amount, token, status, peer_id,
             resource, created_at, expires_at, paid_tx_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                inv.id,
                inv.wallet_id,
                inv.amount,
                inv.token,
                inv.status,
                inv.peer_id,
                inv.resource,
                inv.created_at,
                inv.expires_at,
                inv.paid_tx_id,
            ],
        )?;
        Ok(())
    }

    pub fn get_invoice(&self, id: &str) -> Result<Option<Invoice>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, wallet_id, amount, token, status, peer_id, resource,
             created_at, expires_at, paid_tx_id
             FROM invoices WHERE id = ?1",
            params![id],
            row_to_invoice,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_invoices(&self, status: Option<&str>, limit: i64, offset: i64) -> Result<Vec<Invoice>> {
        let conn = self.conn.lock().unwrap();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = status {
            (
                "SELECT id, wallet_id, amount, token, status, peer_id, resource,
                 created_at, expires_at, paid_tx_id
                 FROM invoices WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3"
                    .to_string(),
                vec![Box::new(s.to_string()), Box::new(limit), Box::new(offset)],
            )
        } else {
            (
                "SELECT id, wallet_id, amount, token, status, peer_id, resource,
                 created_at, expires_at, paid_tx_id
                 FROM invoices ORDER BY created_at DESC LIMIT ?1 OFFSET ?2"
                    .to_string(),
                vec![Box::new(limit), Box::new(offset)],
            )
        };
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), row_to_invoice)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn update_invoice_status(&self, id: &str, status: &str, paid_tx_id: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE invoices SET status = ?1, paid_tx_id = COALESCE(?2, paid_tx_id) WHERE id = ?3",
            params![status, paid_tx_id, id],
        )?;
        Ok(())
    }

    pub fn expire_old_invoices(&self, now: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE invoices SET status = 'expired' WHERE status = 'pending' AND expires_at < ?1",
            params![now],
        )?;
        Ok(count)
    }

    // ── Subscriptions ────────────────────────────────────────────────────

    pub fn insert_subscription(&self, sub: &Subscription) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO subscriptions (id, peer_id, peer_name, capability, amount, token,
             chain, interval_secs, status, current_grant_expires, last_payment_tx,
             created_at, cancelled_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                sub.id,
                sub.peer_id,
                sub.peer_name,
                sub.capability,
                sub.amount,
                sub.token,
                sub.chain,
                sub.interval_secs,
                sub.status,
                sub.current_grant_expires,
                sub.last_payment_tx,
                sub.created_at,
                sub.cancelled_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_subscriptions(&self, status: Option<&str>) -> Result<Vec<Subscription>> {
        let conn = self.conn.lock().unwrap();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = status {
            (
                "SELECT id, peer_id, peer_name, capability, amount, token, chain,
                 interval_secs, status, current_grant_expires, last_payment_tx,
                 created_at, cancelled_at
                 FROM subscriptions WHERE status = ?1 ORDER BY created_at DESC"
                    .to_string(),
                vec![Box::new(s.to_string())],
            )
        } else {
            (
                "SELECT id, peer_id, peer_name, capability, amount, token, chain,
                 interval_secs, status, current_grant_expires, last_payment_tx,
                 created_at, cancelled_at
                 FROM subscriptions ORDER BY created_at DESC"
                    .to_string(),
                vec![],
            )
        };
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), row_to_subscription)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get_subscription(&self, id: &str) -> Result<Option<Subscription>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, peer_id, peer_name, capability, amount, token, chain,
             interval_secs, status, current_grant_expires, last_payment_tx,
             created_at, cancelled_at
             FROM subscriptions WHERE id = ?1",
            params![id],
            row_to_subscription,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn cancel_subscription(&self, id: &str, now: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE subscriptions SET status = 'cancelled', cancelled_at = ?1
             WHERE id = ?2 AND status = 'active'",
            params![now, id],
        )?;
        Ok(count > 0)
    }

    pub fn update_subscription_status(&self, id: &str, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE subscriptions SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn renew_subscription(
        &self,
        id: &str,
        new_grant_expires: i64,
        payment_tx: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE subscriptions SET status = 'active', cancelled_at = NULL,
             current_grant_expires = ?1, last_payment_tx = ?2 WHERE id = ?3",
            params![new_grant_expires, payment_tx, id],
        )?;
        Ok(())
    }

    pub fn expire_subscriptions(&self, now: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE subscriptions SET status = 'expired'
             WHERE status IN ('active', 'cancelled')
             AND current_grant_expires IS NOT NULL AND current_grant_expires < ?1",
            params![now],
        )?;
        Ok(count)
    }

    // ── Receipts ─────────────────────────────────────────────────────────

    pub fn insert_receipt(&self, r: &Receipt) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO receipts (id, transaction_id, invoice_id, peer_id, peer_name,
             direction, amount, token, description, resource_type, resource_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                r.id,
                r.transaction_id,
                r.invoice_id,
                r.peer_id,
                r.peer_name,
                r.direction,
                r.amount,
                r.token,
                r.description,
                r.resource_type,
                r.resource_id,
                r.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_receipts(
        &self,
        peer_id: Option<&str>,
        direction: Option<&str>,
        resource_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Receipt>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, transaction_id, invoice_id, peer_id, peer_name,
             direction, amount, token, description, resource_type, resource_id, created_at
             FROM receipts WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(p) = peer_id {
            sql.push_str(&format!(" AND peer_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(p.to_string()));
        }
        if let Some(d) = direction {
            sql.push_str(&format!(" AND direction = ?{}", param_values.len() + 1));
            param_values.push(Box::new(d.to_string()));
        }
        if let Some(rt) = resource_type {
            sql.push_str(&format!(" AND resource_type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(rt.to_string()));
        }
        sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            param_values.len() + 1,
            param_values.len() + 2
        ));
        param_values.push(Box::new(limit));
        param_values.push(Box::new(offset));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), row_to_receipt)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get_receipt(&self, id: &str) -> Result<Option<Receipt>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, transaction_id, invoice_id, peer_id, peer_name,
             direction, amount, token, description, resource_type, resource_id, created_at
             FROM receipts WHERE id = ?1",
            params![id],
            row_to_receipt,
        )
        .optional()
        .map_err(Into::into)
    }
}

// ── SecretsDb (separate file, restricted permissions) ────────────────────────

impl SecretsDb {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("secrets.db");
        let conn = Connection::open(&db_path)?;

        // Restrict permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600));
        }

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS secrets (
                 wallet_id TEXT PRIMARY KEY,
                 encrypted BLOB NOT NULL
             );",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    #[cfg(test)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS secrets (
                 wallet_id TEXT PRIMARY KEY,
                 encrypted BLOB NOT NULL
             );",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn insert_secret(&self, wallet_id: &str, encrypted: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO secrets (wallet_id, encrypted) VALUES (?1, ?2)",
            params![wallet_id, encrypted],
        )?;
        Ok(())
    }

    pub fn get_secret(&self, wallet_id: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT encrypted FROM secrets WHERE wallet_id = ?1",
            params![wallet_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_secret(&self, wallet_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "DELETE FROM secrets WHERE wallet_id = ?1",
            params![wallet_id],
        )?;
        Ok(count > 0)
    }
}

// ── Row mappers ──────────────────────────────────────────────────────────────

fn row_to_transaction(row: &rusqlite::Row) -> rusqlite::Result<Transaction> {
    Ok(Transaction {
        id: row.get(0)?,
        wallet_id: row.get(1)?,
        direction: row.get(2)?,
        chain_tx_id: row.get(3)?,
        from_addr: row.get(4)?,
        to_addr: row.get(5)?,
        amount: row.get(6)?,
        token: row.get(7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        confirmed_at: row.get(10)?,
        invoice_id: row.get(11)?,
    })
}

fn row_to_invoice(row: &rusqlite::Row) -> rusqlite::Result<Invoice> {
    Ok(Invoice {
        id: row.get(0)?,
        wallet_id: row.get(1)?,
        amount: row.get(2)?,
        token: row.get(3)?,
        status: row.get(4)?,
        peer_id: row.get(5)?,
        resource: row.get(6)?,
        created_at: row.get(7)?,
        expires_at: row.get(8)?,
        paid_tx_id: row.get(9)?,
    })
}

fn row_to_subscription(row: &rusqlite::Row) -> rusqlite::Result<Subscription> {
    Ok(Subscription {
        id: row.get(0)?,
        peer_id: row.get(1)?,
        peer_name: row.get(2)?,
        capability: row.get(3)?,
        amount: row.get(4)?,
        token: row.get(5)?,
        chain: row.get(6)?,
        interval_secs: row.get(7)?,
        status: row.get(8)?,
        current_grant_expires: row.get(9)?,
        last_payment_tx: row.get(10)?,
        created_at: row.get(11)?,
        cancelled_at: row.get(12)?,
    })
}

fn row_to_receipt(row: &rusqlite::Row) -> rusqlite::Result<Receipt> {
    Ok(Receipt {
        id: row.get(0)?,
        transaction_id: row.get(1)?,
        invoice_id: row.get(2)?,
        peer_id: row.get(3)?,
        peer_name: row.get(4)?,
        direction: row.get(5)?,
        amount: row.get(6)?,
        token: row.get(7)?,
        description: row.get(8)?,
        resource_type: row.get(9)?,
        resource_id: row.get(10)?,
        created_at: row.get(11)?,
    })
}

// ── Schema ───────────────────────────────────────────────────────────────────

fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;

         CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
         );
         INSERT OR IGNORE INTO schema_version (rowid, version) VALUES (1, 1);

         CREATE TABLE IF NOT EXISTS wallets (
            id          TEXT PRIMARY KEY,
            chain       TEXT NOT NULL,
            label       TEXT,
            address     TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            is_default  INTEGER NOT NULL DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS transactions (
            id           TEXT PRIMARY KEY,
            wallet_id    TEXT NOT NULL REFERENCES wallets(id),
            direction    TEXT NOT NULL,
            chain_tx_id  TEXT,
            from_addr    TEXT,
            to_addr      TEXT,
            amount       TEXT NOT NULL,
            token        TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'pending',
            created_at   INTEGER NOT NULL,
            confirmed_at INTEGER,
            invoice_id   TEXT
         );

         CREATE TABLE IF NOT EXISTS invoices (
            id          TEXT PRIMARY KEY,
            wallet_id   TEXT NOT NULL REFERENCES wallets(id),
            amount      TEXT NOT NULL,
            token       TEXT NOT NULL,
            status      TEXT NOT NULL DEFAULT 'pending',
            peer_id     TEXT,
            resource    TEXT,
            created_at  INTEGER NOT NULL,
            expires_at  INTEGER NOT NULL,
            paid_tx_id  TEXT
         );

         CREATE TABLE IF NOT EXISTS subscriptions (
            id                    TEXT PRIMARY KEY,
            peer_id               TEXT NOT NULL,
            peer_name             TEXT,
            capability            TEXT NOT NULL,
            amount                TEXT NOT NULL,
            token                 TEXT NOT NULL,
            chain                 TEXT NOT NULL,
            interval_secs         INTEGER NOT NULL,
            status                TEXT NOT NULL DEFAULT 'active',
            current_grant_expires INTEGER,
            last_payment_tx       TEXT,
            created_at            INTEGER NOT NULL,
            cancelled_at          INTEGER
         );

         CREATE TABLE IF NOT EXISTS receipts (
            id              TEXT PRIMARY KEY,
            transaction_id  TEXT NOT NULL REFERENCES transactions(id),
            invoice_id      TEXT,
            peer_id         TEXT NOT NULL,
            peer_name       TEXT,
            direction       TEXT NOT NULL,
            amount          TEXT NOT NULL,
            token           TEXT NOT NULL,
            description     TEXT,
            resource_type   TEXT,
            resource_id     TEXT,
            created_at      INTEGER NOT NULL
         );
        ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn test_wallet(id: &str) -> Wallet {
        Wallet {
            id: id.to_string(),
            chain: "evm".to_string(),
            label: Some("Test".to_string()),
            address: "0xdeadbeef".to_string(),
            created_at: now(),
            is_default: false,
        }
    }

    #[test]
    fn wallet_crud() {
        let db = WalletDb::open_memory().unwrap();
        let w = test_wallet("w1");
        db.insert_wallet(&w).unwrap();

        let wallets = db.list_wallets().unwrap();
        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].id, "w1");

        let fetched = db.get_wallet("w1").unwrap().unwrap();
        assert_eq!(fetched.address, "0xdeadbeef");

        assert!(db.delete_wallet("w1").unwrap());
        assert!(db.get_wallet("w1").unwrap().is_none());
    }

    #[test]
    fn default_wallet() {
        let db = WalletDb::open_memory().unwrap();
        let mut w1 = test_wallet("w1");
        w1.is_default = true;
        db.insert_wallet(&w1).unwrap();
        db.insert_wallet(&test_wallet("w2")).unwrap();

        let def = db.get_default_wallet().unwrap().unwrap();
        assert_eq!(def.id, "w1");

        db.set_default_wallet("w2").unwrap();
        let def = db.get_default_wallet().unwrap().unwrap();
        assert_eq!(def.id, "w2");
    }

    #[test]
    fn transaction_crud() {
        let db = WalletDb::open_memory().unwrap();
        db.insert_wallet(&test_wallet("w1")).unwrap();

        let tx = Transaction {
            id: "tx1".to_string(),
            wallet_id: "w1".to_string(),
            direction: "out".to_string(),
            chain_tx_id: None,
            from_addr: Some("0xaaa".to_string()),
            to_addr: Some("0xbbb".to_string()),
            amount: "1.5".to_string(),
            token: "ETH".to_string(),
            status: "pending".to_string(),
            created_at: now(),
            confirmed_at: None,
            invoice_id: None,
        };
        db.insert_transaction(&tx).unwrap();

        let txs = db.list_transactions(None, None, 50, 0).unwrap();
        assert_eq!(txs.len(), 1);

        let txs = db.list_transactions(Some("out"), None, 50, 0).unwrap();
        assert_eq!(txs.len(), 1);

        let txs = db.list_transactions(Some("in"), None, 50, 0).unwrap();
        assert_eq!(txs.len(), 0);

        db.update_transaction_status("tx1", "confirmed", Some("0xhash"), Some(now()))
            .unwrap();
        let fetched = db.get_transaction("tx1").unwrap().unwrap();
        assert_eq!(fetched.status, "confirmed");
        assert_eq!(fetched.chain_tx_id.as_deref(), Some("0xhash"));
    }

    #[test]
    fn invoice_crud() {
        let db = WalletDb::open_memory().unwrap();
        db.insert_wallet(&test_wallet("w1")).unwrap();

        let inv = Invoice {
            id: "inv1".to_string(),
            wallet_id: "w1".to_string(),
            amount: "0.5".to_string(),
            token: "ETH".to_string(),
            status: "pending".to_string(),
            peer_id: Some("peer123".to_string()),
            resource: Some("files::abc".to_string()),
            created_at: now(),
            expires_at: now() + 3600,
            paid_tx_id: None,
        };
        db.insert_invoice(&inv).unwrap();

        let fetched = db.get_invoice("inv1").unwrap().unwrap();
        assert_eq!(fetched.amount, "0.5");

        db.update_invoice_status("inv1", "paid", Some("tx1")).unwrap();
        let fetched = db.get_invoice("inv1").unwrap().unwrap();
        assert_eq!(fetched.status, "paid");

        let list = db.list_invoices(Some("paid"), 50, 0).unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn invoice_expiry() {
        let db = WalletDb::open_memory().unwrap();
        db.insert_wallet(&test_wallet("w1")).unwrap();

        let inv = Invoice {
            id: "inv1".to_string(),
            wallet_id: "w1".to_string(),
            amount: "1".to_string(),
            token: "ETH".to_string(),
            status: "pending".to_string(),
            peer_id: None,
            resource: None,
            created_at: now() - 7200,
            expires_at: now() - 3600, // expired 1hr ago
            paid_tx_id: None,
        };
        db.insert_invoice(&inv).unwrap();

        let expired = db.expire_old_invoices(now()).unwrap();
        assert_eq!(expired, 1);

        let fetched = db.get_invoice("inv1").unwrap().unwrap();
        assert_eq!(fetched.status, "expired");
    }

    #[test]
    fn subscription_lifecycle() {
        let db = WalletDb::open_memory().unwrap();
        let sub = Subscription {
            id: "sub1".to_string(),
            peer_id: "peer1".to_string(),
            peer_name: Some("Alice".to_string()),
            capability: "tube".to_string(),
            amount: "5".to_string(),
            token: "USDC".to_string(),
            chain: "evm:84532".to_string(),
            interval_secs: 2592000,
            status: "active".to_string(),
            current_grant_expires: Some(now() + 2592000),
            last_payment_tx: Some("tx1".to_string()),
            created_at: now(),
            cancelled_at: None,
        };
        db.insert_subscription(&sub).unwrap();

        let subs = db.list_subscriptions(Some("active")).unwrap();
        assert_eq!(subs.len(), 1);

        // Cancel
        assert!(db.cancel_subscription("sub1", now()).unwrap());
        let fetched = db.get_subscription("sub1").unwrap().unwrap();
        assert_eq!(fetched.status, "cancelled");
        assert!(fetched.cancelled_at.is_some());

        // Can't cancel again
        assert!(!db.cancel_subscription("sub1", now()).unwrap());

        // Renew
        let new_expires = now() + 2592000;
        db.renew_subscription("sub1", new_expires, "tx2").unwrap();
        let fetched = db.get_subscription("sub1").unwrap().unwrap();
        assert_eq!(fetched.status, "active");
        assert!(fetched.cancelled_at.is_none());
    }

    #[test]
    fn subscription_expiry() {
        let db = WalletDb::open_memory().unwrap();
        let sub = Subscription {
            id: "sub1".to_string(),
            peer_id: "peer1".to_string(),
            peer_name: None,
            capability: "files".to_string(),
            amount: "1".to_string(),
            token: "ETH".to_string(),
            chain: "evm:84532".to_string(),
            interval_secs: 3600,
            status: "active".to_string(),
            current_grant_expires: Some(now() - 100), // expired
            last_payment_tx: None,
            created_at: now() - 3700,
            cancelled_at: None,
        };
        db.insert_subscription(&sub).unwrap();

        let expired = db.expire_subscriptions(now()).unwrap();
        assert_eq!(expired, 1);

        let fetched = db.get_subscription("sub1").unwrap().unwrap();
        assert_eq!(fetched.status, "expired");
    }

    #[test]
    fn receipt_crud() {
        let db = WalletDb::open_memory().unwrap();
        db.insert_wallet(&test_wallet("w1")).unwrap();

        let tx = Transaction {
            id: "tx1".to_string(),
            wallet_id: "w1".to_string(),
            direction: "out".to_string(),
            chain_tx_id: Some("0xhash".to_string()),
            from_addr: Some("0xme".to_string()),
            to_addr: Some("0xthem".to_string()),
            amount: "0.5".to_string(),
            token: "ETH".to_string(),
            status: "confirmed".to_string(),
            created_at: now(),
            confirmed_at: Some(now()),
            invoice_id: None,
        };
        db.insert_transaction(&tx).unwrap();

        let r = Receipt {
            id: "r1".to_string(),
            transaction_id: "tx1".to_string(),
            invoice_id: None,
            peer_id: "peer1".to_string(),
            peer_name: Some("Bob".to_string()),
            direction: "sent".to_string(),
            amount: "0.5".to_string(),
            token: "ETH".to_string(),
            description: Some("Direct payment".to_string()),
            resource_type: Some("direct".to_string()),
            resource_id: None,
            created_at: now(),
        };
        db.insert_receipt(&r).unwrap();

        let receipts = db.list_receipts(None, None, None, 50, 0).unwrap();
        assert_eq!(receipts.len(), 1);

        let receipts = db
            .list_receipts(Some("peer1"), Some("sent"), None, 50, 0)
            .unwrap();
        assert_eq!(receipts.len(), 1);

        let receipts = db
            .list_receipts(Some("peer1"), Some("received"), None, 50, 0)
            .unwrap();
        assert_eq!(receipts.len(), 0);

        let fetched = db.get_receipt("r1").unwrap().unwrap();
        assert_eq!(fetched.peer_name.as_deref(), Some("Bob"));
    }

    #[test]
    fn secrets_crud() {
        let db = SecretsDb::open_memory().unwrap();
        db.insert_secret("w1", &[1, 2, 3, 4]).unwrap();

        let secret = db.get_secret("w1").unwrap().unwrap();
        assert_eq!(secret, vec![1, 2, 3, 4]);

        assert!(db.delete_secret("w1").unwrap());
        assert!(db.get_secret("w1").unwrap().is_none());
    }
}

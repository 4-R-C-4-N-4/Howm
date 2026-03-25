use std::sync::Arc;

use crate::db::MessageDb;

/// Fire-and-forget notifier that POSTs badge/toast events to the Howm daemon.
///
/// Uses the daemon's Notification API:
///   POST /notifications/badge  — badge count updates
///   POST /notifications/push   — transient toast notifications
#[derive(Clone)]
pub struct DaemonNotifier {
    client: reqwest::Client,
    badge_url: String,
    push_url: String,
    db: Arc<MessageDb>,
}

impl DaemonNotifier {
    /// Create a new notifier.
    ///
    /// `daemon_base_url` — e.g. `http://127.0.0.1:7000`.
    pub fn new(client: reqwest::Client, daemon_base_url: &str, db: Arc<MessageDb>) -> Self {
        let base = daemon_base_url.trim_end_matches('/');
        Self {
            client,
            badge_url: format!("{base}/notifications/badge"),
            push_url: format!("{base}/notifications/push"),
            db,
        }
    }

    /// Send a badge-only update with the given unread count.
    #[allow(dead_code)]
    pub fn notify_badge_update(&self, count: u64) {
        let client = self.client.clone();
        let url = self.badge_url.clone();
        tokio::spawn(async move {
            let body = serde_json::json!({
                "capability": "messaging",
                "count": count,
            });
            if let Err(e) = client.post(&url).json(&body).send().await {
                tracing::warn!("DaemonNotifier badge POST failed: {e}");
            }
        });
    }

    /// Fire both a toast and a badge update for a newly received message.
    pub fn notify_new_message(&self, sender_name: &str, preview: &str) {
        // Toast via POST /notifications/push
        {
            let client = self.client.clone();
            let url = self.push_url.clone();
            let title = format!("Message from {sender_name}");
            let body_text = preview.to_owned();
            tokio::spawn(async move {
                let body = serde_json::json!({
                    "capability": "messaging",
                    "level": "info",
                    "title": title,
                    "message": body_text,
                });
                if let Err(e) = client.post(&url).json(&body).send().await {
                    tracing::warn!("DaemonNotifier push POST failed: {e}");
                }
            });
        }

        // Badge — read current total unread count from DB
        self.push_badge_from_db();
    }

    /// Read the total unread count from the DB and push a badge update.
    pub fn push_badge_from_db(&self) {
        let db = self.db.clone();
        let client = self.client.clone();
        let url = self.badge_url.clone();
        tokio::spawn(async move {
            let count = match db.total_unread_count() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("DaemonNotifier: failed to query unread count: {e}");
                    return;
                }
            };
            let body = serde_json::json!({
                "capability": "messaging",
                "count": count,
            });
            if let Err(e) = client.post(&url).json(&body).send().await {
                tracing::warn!("DaemonNotifier badge POST failed: {e}");
            }
        });
    }
}

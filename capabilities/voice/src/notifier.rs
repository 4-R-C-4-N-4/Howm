//! Fire-and-forget notifier for voice events — badge/toast to the Howm daemon.
//!
//! Follows messaging's `DaemonNotifier` pattern: spawns tokio tasks for
//! non-blocking POST requests to the daemon notification API.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Fire-and-forget notifier for voice events.
#[derive(Clone)]
pub struct VoiceNotifier {
    client: reqwest::Client,
    badge_url: String,
    push_url: String,
    presence_status_url: String,
    /// Current pending invite count (atomically tracked).
    pending_invites: Arc<AtomicU64>,
}

impl VoiceNotifier {
    /// Create a new notifier.
    ///
    /// `daemon_base_url` — e.g. `http://127.0.0.1:7000`.
    pub fn new(client: reqwest::Client, daemon_base_url: &str) -> Self {
        let base = daemon_base_url.trim_end_matches('/');
        Self {
            client,
            badge_url: format!("{base}/notifications/badge"),
            push_url: format!("{base}/notifications/push"),
            presence_status_url: format!("{base}/cap/presence/status"),
            pending_invites: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Notify about an incoming voice invite.
    pub fn notify_invite(&self, inviter_name: &str, room_name: &str) {
        let count = self.pending_invites.fetch_add(1, Ordering::Relaxed) + 1;

        // Toast
        {
            let client = self.client.clone();
            let url = self.push_url.clone();
            let title = "Voice".to_string();
            let message = format!("{inviter_name} invited you to {room_name}");
            tokio::spawn(async move {
                let body = serde_json::json!({
                    "capability": "social.voice",
                    "level": "info",
                    "title": title,
                    "message": message,
                });
                if let Err(e) = client.post(&url).json(&body).send().await {
                    tracing::warn!("VoiceNotifier push POST failed: {e}");
                }
            });
        }

        // Badge
        self.push_badge(count);
    }

    /// Notify that a room was closed.
    pub fn notify_room_closed(&self, room_name: &str) {
        let client = self.client.clone();
        let url = self.push_url.clone();
        let message = format!("{room_name} was closed");
        tokio::spawn(async move {
            let body = serde_json::json!({
                "capability": "social.voice",
                "level": "info",
                "title": "Voice",
                "message": message,
            });
            if let Err(e) = client.post(&url).json(&body).send().await {
                tracing::warn!("VoiceNotifier push POST failed: {e}");
            }
        });
    }

    /// Decrement pending invite count and push badge update.
    pub fn invite_resolved(&self) {
        let prev = self.pending_invites.fetch_sub(1, Ordering::Relaxed);
        let count = if prev > 0 { prev - 1 } else { 0 };
        self.push_badge(count);
    }

    /// Clear all pending invites (e.g., on room close affecting multiple invites).
    pub fn clear_badge(&self) {
        self.pending_invites.store(0, Ordering::Relaxed);
        self.push_badge(0);
    }

    fn push_badge(&self, count: u64) {
        let client = self.client.clone();
        let url = self.badge_url.clone();
        tokio::spawn(async move {
            let body = serde_json::json!({
                "capability": "social.voice",
                "count": count,
            });
            if let Err(e) = client.post(&url).json(&body).send().await {
                tracing::warn!("VoiceNotifier badge POST failed: {e}");
            }
        });
    }

    /// Set presence status to "In a call" (fire-and-forget).
    pub fn set_in_call(&self, room_name: &str) {
        let client = self.client.clone();
        let url = self.presence_status_url.clone();
        let status = format!("In a call — {room_name}");
        tokio::spawn(async move {
            let body = serde_json::json!({
                "status": status,
                "emoji": "🎙️",
            });
            if let Err(e) = client.put(&url).json(&body).send().await {
                tracing::debug!("VoiceNotifier presence PUT failed (presence may not be running): {e}");
            }
        });
    }

    /// Clear the "In a call" presence status (fire-and-forget).
    pub fn clear_in_call(&self) {
        let client = self.client.clone();
        let url = self.presence_status_url.clone();
        tokio::spawn(async move {
            let body = serde_json::json!({
                "status": null,
                "emoji": null,
            });
            if let Err(e) = client.put(&url).json(&body).send().await {
                tracing::debug!("VoiceNotifier presence clear failed: {e}");
            }
        });
    }
}

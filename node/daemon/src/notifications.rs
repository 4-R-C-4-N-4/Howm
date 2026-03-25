use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Badge ────────────────────────────────────────────────────────────────────

/// Inbound request from a capability process to set its badge count.
#[derive(Debug, Deserialize)]
pub struct BadgeUpdate {
    pub capability: String,
    pub count: u32,
}

/// Response for GET /notifications/badges.
#[derive(Debug, Serialize)]
pub struct BadgesResponse {
    pub badges: HashMap<String, u32>,
}

// ── Push notifications (toasts) ──────────────────────────────────────────────

/// Severity level for a transient notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Inbound request from a capability process to push a notification.
#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub capability: String,
    pub level: NotifyLevel,
    pub title: String,
    pub message: String,
    /// Optional deep-link path (e.g. "/app/social.messaging?peer=...").
    pub action: Option<String>,
}

/// A stored notification entry.
#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: String,
    pub capability: String,
    pub level: NotifyLevel,
    pub title: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    pub created_at: u64,
}

/// Response for GET /notifications/poll.
#[derive(Debug, Serialize)]
pub struct PollResponse {
    pub notifications: Vec<Notification>,
    pub timestamp: u64,
}

// ── Notification buffer ──────────────────────────────────────────────────────

/// Bounded, auto-expiring ring buffer for transient notifications.
/// Max 50 entries, entries older than 60s are pruned on read.
pub struct NotificationBuffer {
    entries: VecDeque<Notification>,
    next_id: AtomicU64,
}

const MAX_ENTRIES: usize = 50;
const EXPIRY_MS: u64 = 60_000;

impl NotificationBuffer {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_ENTRIES),
            next_id: AtomicU64::new(1),
        }
    }

    /// Push a new notification. Prunes expired entries and evicts the oldest
    /// if the buffer is full.
    pub fn push(&mut self, req: PushRequest) -> &Notification {
        let now = now_ms();
        self.prune(now);

        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let notification = Notification {
            id: format!("notif-{}", id),
            capability: req.capability,
            level: req.level,
            title: req.title,
            message: req.message,
            action: req.action,
            created_at: now,
        };

        self.entries.push_back(notification);
        self.entries.back().unwrap()
    }

    /// Return all notifications created after `since_ms` and prune expired.
    pub fn poll(&mut self, since_ms: u64) -> Vec<Notification> {
        let now = now_ms();
        self.prune(now);
        self.entries
            .iter()
            .filter(|n| n.created_at > since_ms)
            .cloned()
            .collect()
    }

    /// Number of entries currently in the buffer (for testing).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn prune(&mut self, now: u64) {
        let cutoff = now.saturating_sub(EXPIRY_MS);
        while self
            .entries
            .front()
            .map(|n| n.created_at < cutoff)
            .unwrap_or(false)
        {
            self.entries.pop_front();
        }
    }
}

// ── Rate limiter (per-capability push rate) ──────────────────────────────────

/// Simple sliding-window rate limiter: max `limit` pushes per `window_ms` per
/// capability.
pub struct PushRateLimiter {
    /// capability → list of push timestamps (ms)
    windows: HashMap<String, VecDeque<u64>>,
    limit: usize,
    window_ms: u64,
}

impl PushRateLimiter {
    pub fn new(limit: usize, window_ms: u64) -> Self {
        Self {
            windows: HashMap::new(),
            limit,
            window_ms,
        }
    }

    /// Returns `true` if the push is allowed, `false` if rate-limited.
    pub fn check_and_record(&mut self, capability: &str) -> bool {
        let now = now_ms();
        let window = self.windows.entry(capability.to_string()).or_default();

        let cutoff = now.saturating_sub(self.window_ms);
        while window.front().map(|&t| t < cutoff).unwrap_or(false) {
            window.pop_front();
        }

        if window.len() >= self.limit {
            return false;
        }

        window.push_back(now);
        true
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_push(cap: &str) -> PushRequest {
        PushRequest {
            capability: cap.to_string(),
            level: NotifyLevel::Info,
            title: "test".to_string(),
            message: "test message".to_string(),
            action: None,
        }
    }

    #[test]
    fn buffer_cap_at_50() {
        let mut buf = NotificationBuffer::new();
        for i in 0..60 {
            buf.push(make_push(&format!("cap-{}", i)));
        }
        assert_eq!(buf.len(), MAX_ENTRIES);
    }

    #[test]
    fn buffer_poll_since() {
        let mut buf = NotificationBuffer::new();
        let n1 = buf.push(make_push("a")).clone();
        let n2 = buf.push(make_push("b")).clone();

        // Poll since before first → both
        let all = buf.poll(0);
        assert_eq!(all.len(), 2);

        // Poll since first → only second
        let after = buf.poll(n1.created_at);
        // created_at could be same ms; filter is strictly >, so if same ms we get 0
        // In practice the push is sequential, but let's be safe
        assert!(after.len() <= 1);
        if !after.is_empty() {
            assert_eq!(after[0].id, n2.id);
        }
    }

    #[test]
    fn rate_limiter_allows_within_limit() {
        let mut rl = PushRateLimiter::new(3, 10_000);
        assert!(rl.check_and_record("cap"));
        assert!(rl.check_and_record("cap"));
        assert!(rl.check_and_record("cap"));
        assert!(!rl.check_and_record("cap")); // 4th blocked
    }

    #[test]
    fn rate_limiter_separate_capabilities() {
        let mut rl = PushRateLimiter::new(1, 10_000);
        assert!(rl.check_and_record("cap-a"));
        assert!(!rl.check_and_record("cap-a")); // blocked
        assert!(rl.check_and_record("cap-b")); // different cap, allowed
    }

    #[test]
    fn notification_ids_are_unique() {
        let mut buf = NotificationBuffer::new();
        let n1 = buf.push(make_push("a")).clone();
        let n2 = buf.push(make_push("a")).clone();
        assert_ne!(n1.id, n2.id);
    }
}

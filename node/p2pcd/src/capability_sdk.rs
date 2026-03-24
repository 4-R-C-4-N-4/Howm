// Capability SDK — shared types and helpers for out-of-process capabilities
//
// Any capability that runs as a separate process and talks to the daemon via
// the bridge uses these types. The SDK provides:
//
//   - Wire types for daemon callbacks (peer-active, peer-inactive, inbound)
//   - `PeerTracker` — manages the active peer list with thread-safe upsert/remove
//   - Lifecycle helpers (init from daemon, handle callbacks)
//
// The capability wires its own axum routes but delegates to PeerTracker and
// BridgeClient for all p2pcd plumbing. This keeps p2pcd free of web framework
// dependencies while ensuring every capability follows the same pattern.
//
// # Example (in a capability's api.rs)
//
// ```no_run
// use p2pcd::capability_sdk::{PeerTracker, PeerActivePayload, PeerInactivePayload, InboundMessage};
// use p2pcd::bridge_client::BridgeClient;
//
// struct AppState {
//     peers: PeerTracker,
//     bridge: BridgeClient,
// }
//
// async fn handle_peer_active(state: &AppState, body: PeerActivePayload) {
//     state.peers.on_peer_active(body).await;
// }
//
// async fn handle_peer_inactive(state: &AppState, body: PeerInactivePayload) {
//     state.peers.on_peer_inactive(&body.peer_id).await;
// }
// ```

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::bridge_client::BridgeClient;

// ── Wire types for daemon → capability callbacks ────────────────────────────
//
// These mirror the types in daemon/src/p2pcd/cap_notify.rs. Both sides must
// agree on the JSON shape. Kept in the SDK so capabilities don't duplicate them.

/// Payload from daemon: `POST /p2pcd/peer-active`
///
/// Sent when a peer successfully negotiates this capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerActivePayload {
    /// Base64-encoded 32-byte WireGuard public key.
    pub peer_id: String,
    /// Peer's WireGuard IP address (for direct connections).
    pub wg_address: String,
    /// Capability name this notification is for.
    pub capability: String,
    /// Agreed scope parameters for this capability.
    #[serde(default)]
    pub scope: serde_json::Value,
    /// Unix timestamp when the session became ACTIVE.
    #[serde(default)]
    pub active_since: u64,
}

/// Payload from daemon: `POST /p2pcd/peer-inactive`
///
/// Sent when a peer session ends (timeout, close, re-exchange).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInactivePayload {
    /// Base64-encoded 32-byte WireGuard public key.
    pub peer_id: String,
    /// Capability name this notification is for.
    pub capability: String,
    /// Reason for the peer becoming inactive.
    pub reason: String,
}

/// Payload from daemon: `POST /p2pcd/inbound`
///
/// Forwarded when an inbound CapabilityMsg arrives that has no in-process
/// handler (i.e. app-level message types >= 100).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Base64-encoded 32-byte peer ID of the sender.
    pub peer_id: String,
    /// Message type number.
    pub message_type: u64,
    /// Base64-encoded payload bytes.
    pub payload: String,
    /// Capability name this message belongs to.
    pub capability: String,
}

// ── Active peer record ──────────────────────────────────────────────────────

/// An active peer for this capability, maintained by PeerTracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivePeer {
    /// Base64-encoded WireGuard public key.
    pub peer_id: String,
    /// WireGuard IP address — used for direct HTTP calls to this peer.
    pub wg_address: String,
    /// Unix timestamp when this peer became active.
    pub active_since: u64,
}

// ── PeerTracker ─────────────────────────────────────────────────────────────

/// Thread-safe active peer list for a capability.
///
/// Handles the standard lifecycle: upsert on peer-active, remove on
/// peer-inactive, restore from daemon on startup.
#[derive(Clone)]
pub struct PeerTracker {
    cap_name: String,
    peers: Arc<RwLock<Vec<ActivePeer>>>,
}

impl PeerTracker {
    /// Create a new tracker for the given capability name.
    pub fn new(cap_name: impl Into<String>) -> Self {
        Self {
            cap_name: cap_name.into(),
            peers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// The capability name this tracker manages.
    pub fn capability_name(&self) -> &str {
        &self.cap_name
    }

    /// Handle a peer-active callback from the daemon.
    ///
    /// Upserts the peer (removes old entry with same peer_id, adds new).
    /// Returns `true` if this was a new peer, `false` if it was an update.
    pub async fn on_peer_active(&self, payload: PeerActivePayload) -> bool {
        if payload.capability != self.cap_name {
            return false; // not for us
        }

        let peer = ActivePeer {
            peer_id: payload.peer_id.clone(),
            wg_address: payload.wg_address,
            active_since: payload.active_since,
        };

        let mut peers = self.peers.write().await;
        let was_new = !peers.iter().any(|p| p.peer_id == payload.peer_id);
        peers.retain(|p| p.peer_id != payload.peer_id);
        peers.push(peer);
        was_new
    }

    /// Handle a peer-inactive callback from the daemon.
    ///
    /// Returns `true` if the peer was found and removed.
    pub async fn on_peer_inactive(&self, peer_id: &str) -> bool {
        let mut peers = self.peers.write().await;
        let before = peers.len();
        peers.retain(|p| p.peer_id != peer_id);
        peers.len() < before
    }

    /// Check if an inbound message belongs to this capability.
    pub fn is_for_us(&self, msg: &InboundMessage) -> bool {
        msg.capability == self.cap_name
    }

    /// Decode the base64 payload from an InboundMessage into raw bytes.
    pub fn decode_payload(msg: &InboundMessage) -> Result<Vec<u8>, String> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&msg.payload)
            .map_err(|e| format!("bad base64 payload: {e}"))
    }

    /// Get a snapshot of all active peers.
    pub async fn peers(&self) -> Vec<ActivePeer> {
        self.peers.read().await.clone()
    }

    /// Get the count of active peers.
    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    /// Find a peer by peer_id.
    pub async fn find_peer(&self, peer_id: &str) -> Option<ActivePeer> {
        self.peers
            .read()
            .await
            .iter()
            .find(|p| p.peer_id == peer_id)
            .cloned()
    }

    /// Restore active peers from the daemon on startup.
    ///
    /// Queries the bridge for already-active peers. This handles the case where
    /// the capability restarts while peers are already connected.
    pub async fn init_from_daemon(&self, bridge: &BridgeClient) {
        match bridge.list_peers(Some(&self.cap_name)).await {
            Ok(bridge_peers) => {
                let mut peers = self.peers.write().await;
                for bp in &bridge_peers {
                    // Avoid duplicates
                    if !peers.iter().any(|p| p.peer_id == bp.peer_id) {
                        peers.push(ActivePeer {
                            peer_id: bp.peer_id.clone(),
                            wg_address: String::new(), // filled on next peer-active callback
                            active_since: 0,
                        });
                    }
                }
                if !peers.is_empty() {
                    tracing::info!(
                        "capability_sdk: restored {} active peers for '{}' from daemon",
                        peers.len(),
                        self.cap_name
                    );
                }
            }
            Err(e) => {
                // Daemon may not be running yet — not fatal
                tracing::debug!(
                    "capability_sdk: daemon not reachable for '{}' ({e}), starting with empty peer list",
                    self.cap_name
                );
            }
        }
    }

    /// Clear all peers (e.g. on shutdown or daemon disconnect).
    pub async fn clear(&self) {
        self.peers.write().await.clear();
    }
}

// ── CapabilityRuntime ───────────────────────────────────────────────────────

/// Convenience struct bundling PeerTracker + BridgeClient.
///
/// Most capabilities need both. This saves a few lines of setup:
///
/// ```ignore
/// use p2pcd::capability_sdk::CapabilityRuntime;
///
/// let runtime = CapabilityRuntime::new("howm.feed.1", 7000);
/// runtime.init_from_daemon().await;
///
/// // Use runtime.bridge() for outbound messages
/// // Use runtime.peers() for the peer tracker
/// ```
#[derive(Clone)]
pub struct CapabilityRuntime {
    tracker: PeerTracker,
    bridge: BridgeClient,
}

impl CapabilityRuntime {
    /// Create a new runtime for a capability.
    ///
    /// `cap_name` is the fully-qualified capability name (e.g. "howm.feed.1").
    /// `daemon_port` is the port the howm daemon listens on (default 7000).
    pub fn new(cap_name: impl Into<String>, daemon_port: u16) -> Self {
        let cap_name = cap_name.into();
        Self {
            tracker: PeerTracker::new(cap_name),
            bridge: BridgeClient::new(daemon_port),
        }
    }

    /// Create with a custom bridge URL (for testing).
    pub fn with_bridge_url(cap_name: impl Into<String>, bridge_url: String) -> Self {
        Self {
            tracker: PeerTracker::new(cap_name.into()),
            bridge: BridgeClient::with_base_url(bridge_url),
        }
    }

    /// Access the peer tracker.
    pub fn peers(&self) -> &PeerTracker {
        &self.tracker
    }

    /// Access the bridge client.
    pub fn bridge(&self) -> &BridgeClient {
        &self.bridge
    }

    /// Restore active peers from the daemon.
    pub async fn init_from_daemon(&self) {
        self.tracker.init_from_daemon(&self.bridge).await;
    }

    /// Capability name.
    pub fn capability_name(&self) -> &str {
        self.tracker.capability_name()
    }

    /// Broadcast an event to all peers with this capability.
    pub async fn broadcast(
        &self,
        message_type: u64,
        payload: &[u8],
    ) -> Result<usize, crate::bridge_client::BridgeError> {
        self.bridge
            .broadcast_event(self.capability_name(), message_type, payload)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_active_payload_serde_roundtrip() {
        let p = PeerActivePayload {
            peer_id: "AAAA".into(),
            wg_address: "100.222.0.2".into(),
            capability: "howm.feed.1".into(),
            scope: serde_json::json!({}),
            active_since: 1234567890,
        };
        let json = serde_json::to_string(&p).unwrap();
        let q: PeerActivePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(q.peer_id, "AAAA");
        assert_eq!(q.capability, "howm.feed.1");
    }

    #[test]
    fn peer_inactive_payload_serde() {
        let p = PeerInactivePayload {
            peer_id: "BBBB".into(),
            capability: "howm.feed.1".into(),
            reason: "timeout".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("timeout"));
    }

    #[test]
    fn inbound_message_serde() {
        let m = InboundMessage {
            peer_id: "CCCC".into(),
            message_type: 100,
            payload: "dGVzdA==".into(),
            capability: "howm.feed.1".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let m2: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(m2.message_type, 100);
    }

    #[tokio::test]
    async fn peer_tracker_upsert_and_remove() {
        let tracker = PeerTracker::new("test.cap.1");

        // Add a peer
        let payload = PeerActivePayload {
            peer_id: "peer-a".into(),
            wg_address: "100.222.0.2".into(),
            capability: "test.cap.1".into(),
            scope: serde_json::json!({}),
            active_since: 100,
        };
        assert!(tracker.on_peer_active(payload).await); // new
        assert_eq!(tracker.peer_count().await, 1);

        // Upsert same peer (update, not new)
        let payload2 = PeerActivePayload {
            peer_id: "peer-a".into(),
            wg_address: "100.222.0.3".into(), // changed address
            capability: "test.cap.1".into(),
            scope: serde_json::json!({}),
            active_since: 200,
        };
        assert!(!tracker.on_peer_active(payload2).await); // update
        assert_eq!(tracker.peer_count().await, 1);

        // Address should be updated
        let peer = tracker.find_peer("peer-a").await.unwrap();
        assert_eq!(peer.wg_address, "100.222.0.3");
        assert_eq!(peer.active_since, 200);

        // Remove
        assert!(tracker.on_peer_inactive("peer-a").await);
        assert_eq!(tracker.peer_count().await, 0);

        // Remove non-existent
        assert!(!tracker.on_peer_inactive("peer-b").await);
    }

    #[tokio::test]
    async fn peer_tracker_ignores_wrong_capability() {
        let tracker = PeerTracker::new("test.cap.1");

        let payload = PeerActivePayload {
            peer_id: "peer-a".into(),
            wg_address: "100.222.0.2".into(),
            capability: "other.cap.1".into(), // wrong cap
            scope: serde_json::json!({}),
            active_since: 100,
        };
        assert!(!tracker.on_peer_active(payload).await);
        assert_eq!(tracker.peer_count().await, 0);
    }

    #[tokio::test]
    async fn peer_tracker_multiple_peers() {
        let tracker = PeerTracker::new("test.cap.1");

        for i in 0..5 {
            let payload = PeerActivePayload {
                peer_id: format!("peer-{i}"),
                wg_address: format!("100.222.0.{}", i + 2),
                capability: "test.cap.1".into(),
                scope: serde_json::json!({}),
                active_since: i as u64 * 100,
            };
            tracker.on_peer_active(payload).await;
        }
        assert_eq!(tracker.peer_count().await, 5);

        // Remove middle peer
        tracker.on_peer_inactive("peer-2").await;
        assert_eq!(tracker.peer_count().await, 4);
        assert!(tracker.find_peer("peer-2").await.is_none());
        assert!(tracker.find_peer("peer-3").await.is_some());
    }

    #[test]
    fn decode_payload_valid() {
        let msg = InboundMessage {
            peer_id: "x".into(),
            message_type: 100,
            payload: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"hello"),
            capability: "test".into(),
        };
        let bytes = PeerTracker::decode_payload(&msg).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn decode_payload_bad_base64() {
        let msg = InboundMessage {
            peer_id: "x".into(),
            message_type: 100,
            payload: "not-base64!!!".into(),
            capability: "test".into(),
        };
        assert!(PeerTracker::decode_payload(&msg).is_err());
    }

    #[tokio::test]
    async fn peer_tracker_clear() {
        let tracker = PeerTracker::new("test.cap.1");
        let payload = PeerActivePayload {
            peer_id: "peer-a".into(),
            wg_address: "100.222.0.2".into(),
            capability: "test.cap.1".into(),
            scope: serde_json::json!({}),
            active_since: 100,
        };
        tracker.on_peer_active(payload).await;
        assert_eq!(tracker.peer_count().await, 1);

        tracker.clear().await;
        assert_eq!(tracker.peer_count().await, 0);
    }

    #[test]
    fn capability_runtime_creates_correctly() {
        let rt = CapabilityRuntime::new("howm.feed.1", 7000);
        assert_eq!(rt.capability_name(), "howm.feed.1");
    }

    #[test]
    fn is_for_us_checks_capability() {
        let tracker = PeerTracker::new("howm.feed.1");
        let msg_yes = InboundMessage {
            peer_id: "x".into(),
            message_type: 100,
            payload: "".into(),
            capability: "howm.feed.1".into(),
        };
        let msg_no = InboundMessage {
            peer_id: "x".into(),
            message_type: 100,
            payload: "".into(),
            capability: "other.cap.1".into(),
        };
        assert!(tracker.is_for_us(&msg_yes));
        assert!(!tracker.is_for_us(&msg_no));
    }
}

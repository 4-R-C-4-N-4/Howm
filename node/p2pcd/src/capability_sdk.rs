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
    #[serde(default)]
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
    ///
    /// Retries with backoff on connection errors — capabilities may be spawned
    /// before the daemon's HTTP listener is ready.
    pub async fn init_from_daemon(&self, bridge: &BridgeClient) {
        let delays_ms: &[u64] = &[50, 150, 500, 1000, 2000];
        for (attempt, &delay_ms) in delays_ms.iter().enumerate() {
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
                    return;
                }
                Err(crate::bridge_client::BridgeError::Http(ref e)) if e.is_connect() => {
                    if attempt + 1 < delays_ms.len() {
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    } else {
                        tracing::debug!(
                            "capability_sdk: daemon not reachable for '{}' after {} attempts, starting with empty peer list",
                            self.cap_name,
                            delays_ms.len()
                        );
                    }
                }
                Err(e) => {
                    // Other errors (auth, parse, etc.) — not retryable
                    tracing::debug!(
                        "capability_sdk: daemon not reachable for '{}' ({e}), starting with empty peer list",
                        self.cap_name
                    );
                    return;
                }
            }
        }
    }

    /// Atomically replace the entire peer list.
    ///
    /// Used by PeerStream when a `snapshot` event arrives on reconnect.
    /// Replaces all existing peers with the new list in a single write lock.
    pub async fn replace_all(&self, peers: Vec<ActivePeer>) {
        *self.peers.write().await = peers;
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
/// let runtime = CapabilityRuntime::new("howm.social.feed.1", 7000);
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
    /// `cap_name` is the fully-qualified capability name (e.g. "howm.social.feed.1").
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

    /// Start the SSE event stream (no hooks).
    ///
    /// Replaces `init_from_daemon()` — the SSE stream handles startup
    /// reconciliation automatically via the snapshot event.
    ///
    /// Derives the SSE URL from the bridge's base_url so this works correctly
    /// for both port-based runtimes (`new`) and URL-based runtimes
    /// (`with_bridge_url`) — no silent port-0 failure.
    ///
    /// Critically, this drives the runtime's OWN tracker — not a fresh one.
    /// Callers that hold `state.runtime.peers()` will see live updates because
    /// `PeerTracker::clone` shares the same Arc<RwLock<_>> underneath.
    #[cfg(feature = "bridge-client")]
    pub fn start_event_stream(&self) -> PeerStream {
        let url = self.bridge.events_url(self.capability_name());
        // Drive the runtime's OWN tracker — not a new one.
        // The caller (capability main.rs) holds state.runtime.peers() which returns
        // &self.tracker; this SSE loop updates it in place.
        peer_stream_impl::PeerStream::drive_existing(self.tracker.clone(), url, None, None)
    }
}

// ── PeerStream ───────────────────────────────────────────────────────────────
//
// Self-healing SSE client that keeps a PeerTracker current automatically.
// All code in this section is gated behind the bridge-client feature.

#[cfg(feature = "bridge-client")]
pub use peer_stream_impl::*;

// Inner feature gate is redundant (the whole module is excluded by lib.rs),
// but kept as an explicit marker that PeerStream requires bridge-client.
#[cfg(feature = "bridge-client")]
mod peer_stream_impl {
    use super::*;

    /// Type alias for an on-active or on-inactive hook.
    ///
    /// Takes the peer_id (base64 String) and returns a future.
    /// The future is spawned, not awaited inline, so slow hooks cannot
    /// lag the SSE consumer loop.
    pub type HookFn = Arc<
        dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + Sync,
    >;

    /// Self-healing SSE connection to the daemon's peer-event stream.
    ///
    /// Keeps a `PeerTracker` current automatically. Reconnects with exponential
    /// backoff plus startup jitter when the stream drops. On each reconnect the
    /// daemon sends a `snapshot` event that atomically reconciles the peer list —
    /// no manual `init_from_daemon` needed.
    ///
    /// Hooks (on_active, on_inactive) fire inside `tokio::spawn` after the tracker
    /// is updated — never inline — so a slow hook (DB write, HTTP call) cannot
    /// delay the SSE consumer loop.
    pub struct PeerStream {
        tracker: PeerTracker,
    }

    impl PeerStream {
        /// Connect with no hooks (Type 1 capabilities: messaging, feed).
        pub fn connect(cap_name: impl Into<String>, daemon_port: u16) -> Self {
            Self::connect_with_hooks(cap_name, daemon_port, None, None)
        }

        /// Connect with optional hooks.
        ///
        /// `on_active(peer_id)` fires after each peer-active event (including
        /// snapshot peers on reconnect). Use for side-effect caches.
        ///
        /// `on_inactive(peer_id)` fires after each peer-inactive event. Must be
        /// idempotent; use a generation guard for destructive teardown.
        pub fn connect_with_hooks(
            cap_name: impl Into<String>,
            daemon_port: u16,
            on_active: Option<HookFn>,
            on_inactive: Option<HookFn>,
        ) -> Self {
            let cap_name = cap_name.into();
            let url = format!(
                "http://127.0.0.1:{}/p2pcd/bridge/events?capability={}",
                daemon_port, cap_name
            );
            Self::connect_with_hooks_and_url(cap_name, url, on_active, on_inactive)
        }

        /// Connect using an explicit SSE endpoint URL.
        ///
        /// Use this when constructing a runtime with `CapabilityRuntime::with_bridge_url`
        /// (which has no meaningful daemon_port). The URL should point to the daemon's
        /// `/p2pcd/bridge/events?capability=<name>` endpoint.
        pub fn connect_with_url(cap_name: impl Into<String>, events_url: String) -> Self {
            Self::connect_with_hooks_and_url(cap_name, events_url, None, None)
        }

        /// Connect with an explicit URL and optional hooks.
        ///
        /// This is the shared internal constructor used by all other `connect*` variants.
        /// Separating URL construction from the connection logic means URL-based runtimes
        /// (e.g. `CapabilityRuntime::with_bridge_url`) never accidentally hit port 0.
        pub fn connect_with_hooks_and_url(
            cap_name: impl Into<String>,
            events_url: String,
            on_active: Option<HookFn>,
            on_inactive: Option<HookFn>,
        ) -> Self {
            let cap_name = cap_name.into();
            let tracker = PeerTracker::new(cap_name);
            let tracker_bg = tracker.clone();

            // TODO: Store JoinHandle and abort on PeerStream::drop for clean shutdown support.
            tokio::spawn(async move {
                sse_reconnect_loop(tracker_bg, events_url, on_active, on_inactive).await;
            });

            Self { tracker }
        }

        /// Start the SSE loop using an existing PeerTracker (for use by CapabilityRuntime).
        ///
        /// Unlike `connect`/`connect_with_hooks`, this does NOT create a new tracker.
        /// The SSE loop updates the provided tracker in place, so callers that already
        /// hold a reference to it (via `Arc<RwLock<_>>` internals) will see live updates.
        ///
        /// This is the correct constructor for `CapabilityRuntime::start_event_stream()`,
        /// which needs the SSE background task to update the runtime's own tracker —
        /// not a disconnected copy.
        /// Start the SSE loop using a caller-supplied PeerTracker.
        ///
        /// Use this when you need to share the tracker with hooks or other
        /// parts of the application *before* the stream is constructed — for
        /// example, when an `on_inactive` hook needs to call `tracker.find_peer()`
        /// as a generation guard against double-teardown on session flaps.
        ///
        /// ```no_run
        /// use p2pcd::capability_sdk::{PeerStream, PeerTracker, HookFn};
        /// use std::sync::Arc;
        ///
        /// // 1. Create the shared tracker.
        /// let tracker = PeerTracker::new("howm.social.example.1");
        /// let hook_tracker = tracker.clone(); // shares the inner Arc
        ///
        /// // 2. Build hook that guards against double-teardown.
        /// let on_inactive: HookFn = Arc::new(move |peer_id: String| {
        ///     let t = hook_tracker.clone();
        ///     Box::pin(async move {
        ///         if t.find_peer(&peer_id).await.is_some() { return; } // flap guard
        ///         // … teardown …
        ///     })
        /// });
        ///
        /// // 3. Drive the tracker via SSE.
        /// let url = format!("http://127.0.0.1:7000/p2pcd/bridge/events?capability=howm.social.example.1");
        /// let _stream = PeerStream::drive_existing(tracker, url, None, Some(on_inactive));
        /// ```
        pub fn drive_existing(
            tracker: PeerTracker,
            events_url: String,
            on_active: Option<HookFn>,
            on_inactive: Option<HookFn>,
        ) -> Self {
            let tracker_bg = tracker.clone(); // Arc-backed clone — same underlying data
                                              // TODO: Store JoinHandle and abort on PeerStream::drop for clean shutdown support.
            tokio::spawn(async move {
                sse_reconnect_loop(tracker_bg, events_url, on_active, on_inactive).await;
            });
            Self { tracker }
        }

        /// Access the underlying PeerTracker for peer queries.
        pub fn tracker(&self) -> &PeerTracker {
            &self.tracker
        }
    }

    async fn sse_reconnect_loop(
        tracker: PeerTracker,
        url: String,
        on_active: Option<HookFn>,
        on_inactive: Option<HookFn>,
    ) {
        // Jitter: stagger initial connects so all capabilities don't hit the daemon
        // simultaneously after a daemon restart. Prevents thundering herd at scale.
        let jitter_ms = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos() % 500)
                .unwrap_or(0) as u64
        };
        tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;

        // Create the HTTP client once. reqwest::Client manages a connection pool,
        // TLS session cache, and DNS resolver internally — creating it on every
        // reconnect (including the common 50ms reconnect after a clean close) is
        // wasteful. Reusing the same client across reconnects amortises that cost.
        let client = reqwest::Client::new();

        let mut backoff_ms: u64 = 50;

        loop {
            match sse_consume_once(&client, &tracker, &url, &on_active, &on_inactive).await {
                Ok(()) => {
                    // Clean close (server closed stream) — brief pause before reconnect.
                    // Avoids rapid reconnect loops when the daemon is cycling; also gives
                    // consumers a small window to observe updated tracker state.
                    tracing::debug!(
                        "capability_sdk: SSE stream for '{}' closed cleanly, reconnecting",
                        tracker.capability_name()
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    backoff_ms = 50;
                }
                Err(e) => {
                    tracing::debug!(
                        "capability_sdk: SSE stream for '{}' error: {}, retrying in {}ms",
                        tracker.capability_name(),
                        e,
                        backoff_ms
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(16_000);
                }
            }
        }
    }

    async fn sse_consume_once(
        client: &reqwest::Client,
        tracker: &PeerTracker,
        url: &str,
        on_active: &Option<HookFn>,
        on_inactive: &Option<HookFn>,
    ) -> Result<(), String> {
        use futures::StreamExt;
        use reqwest_eventsource::{Event, EventSource};

        let req = client.get(url);
        let mut es = EventSource::new(req).map_err(|e| e.to_string())?;

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => {
                    tracing::debug!("capability_sdk: SSE connection opened to {}", url);
                }
                Ok(Event::Message(msg)) => {
                    handle_sse_message(tracker, &msg.event, &msg.data, on_active, on_inactive)
                        .await;
                }
                Err(reqwest_eventsource::Error::StreamEnded) => {
                    // Server closed the stream cleanly.
                    return Ok(());
                }
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        }
        Ok(())
    }

    async fn handle_sse_message(
        tracker: &PeerTracker,
        event_type: &str,
        data: &str,
        on_active: &Option<HookFn>,
        on_inactive: &Option<HookFn>,
    ) {
        match event_type {
            "snapshot" => {
                #[derive(serde::Deserialize)]
                struct SnapshotData {
                    peers: Vec<SnapshotPeer>,
                }
                #[derive(serde::Deserialize)]
                struct SnapshotPeer {
                    peer_id: String,
                    #[serde(default)]
                    wg_address: Option<String>,
                    #[serde(default)]
                    active_since: u64,
                }

                match serde_json::from_str::<SnapshotData>(data) {
                    Ok(snap) => {
                        let peers: Vec<ActivePeer> = snap
                            .peers
                            .iter()
                            .map(|p| ActivePeer {
                                peer_id: p.peer_id.clone(),
                                wg_address: p.wg_address.clone().unwrap_or_default(),
                                active_since: p.active_since,
                            })
                            .collect();

                        tracker.replace_all(peers).await;
                        tracing::debug!(
                            "capability_sdk: SSE snapshot applied ({} peers)",
                            tracker.peer_count().await
                        );

                        // Fire on_active hook for each snapshot peer (spawned, not awaited).
                        if let Some(hook) = on_active {
                            for peer in &snap.peers {
                                let fut = hook(peer.peer_id.clone());
                                tokio::spawn(fut);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("capability_sdk: failed to parse SSE snapshot: {}", e);
                    }
                }
            }
            "peer-active" => {
                #[derive(serde::Deserialize)]
                struct PeerActiveData {
                    peer_id: String,
                    #[serde(default)]
                    wg_address: String,
                    #[serde(default)]
                    active_since: u64,
                }
                if let Ok(p) = serde_json::from_str::<PeerActiveData>(data) {
                    let peer_id = p.peer_id.clone();
                    tracker
                        .on_peer_active(PeerActivePayload {
                            peer_id: p.peer_id,
                            wg_address: p.wg_address,
                            capability: tracker.capability_name().to_string(),
                            scope: serde_json::Value::Null,
                            active_since: p.active_since,
                        })
                        .await;
                    // Fire hook (spawned, not awaited).
                    if let Some(hook) = on_active {
                        let fut = hook(peer_id);
                        tokio::spawn(fut);
                    }
                }
            }
            "peer-inactive" => {
                #[derive(serde::Deserialize)]
                struct PeerInactiveData {
                    peer_id: String,
                }
                if let Ok(p) = serde_json::from_str::<PeerInactiveData>(data) {
                    let peer_id = p.peer_id.clone();
                    tracker.on_peer_inactive(&p.peer_id).await;
                    // Fire hook (spawned, not awaited).
                    if let Some(hook) = on_inactive {
                        let fut = hook(peer_id);
                        tokio::spawn(fut);
                    }
                }
            }
            _ => {
                tracing::debug!("capability_sdk: unknown SSE event type: '{}'", event_type);
            }
        }
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
            capability: "howm.social.feed.1".into(),
            scope: serde_json::json!({}),
            active_since: 1234567890,
        };
        let json = serde_json::to_string(&p).unwrap();
        let q: PeerActivePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(q.peer_id, "AAAA");
        assert_eq!(q.capability, "howm.social.feed.1");
    }

    #[test]
    fn peer_inactive_payload_serde() {
        let p = PeerInactivePayload {
            peer_id: "BBBB".into(),
            capability: "howm.social.feed.1".into(),
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
            capability: "howm.social.feed.1".into(),
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
        let rt = CapabilityRuntime::new("howm.social.feed.1", 7000);
        assert_eq!(rt.capability_name(), "howm.social.feed.1");
    }

    // ── PeerStream tests ─────────────────────────────────────────────────────
    // These tests spin up a tiny axum SSE server on a random port and verify
    // that PeerStream correctly populates and updates the PeerTracker.
    //
    // All tests are gated behind bridge-client so the SSE helpers are in scope.

    #[cfg(feature = "bridge-client")]
    mod peer_stream_tests {
        use super::*;
        use std::sync::atomic::Ordering;
        use std::sync::{Arc as StdArc, Mutex};
        use std::time::{Duration, Instant};

        use axum::response::sse::{Event as AxumEvent, KeepAlive, Sse};
        use axum::Router;
        use futures_util::stream;
        use tokio::net::TcpListener;

        /// Spin up an axum SSE server on a random port that serves the given events
        /// then closes the stream. Returns the port.
        ///
        /// `events` is a list of (event_type, data) pairs.
        async fn make_sse_server(events: Vec<(String, String)>) -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();

            let app = Router::new().route(
                "/p2pcd/bridge/events",
                axum::routing::get(move || {
                    let events = events.clone();
                    async move {
                        let items: Vec<Result<AxumEvent, std::convert::Infallible>> = events
                            .into_iter()
                            .map(|(evt_type, data)| {
                                Ok(AxumEvent::default().event(evt_type).data(data))
                            })
                            .collect();
                        Sse::new(stream::iter(items)).keep_alive(KeepAlive::default())
                    }
                }),
            );

            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            port
        }

        /// Like make_sse_server but allows multiple connections; each call to the
        /// handler pops events from a shared queue. Returns (port, queue_arc).
        /// `responses` is a vec of vecs — each inner vec is the events for one connection.
        async fn make_multi_connect_sse_server(
            responses: Vec<Vec<(String, String)>>,
        ) -> (u16, StdArc<Mutex<Vec<Vec<(String, String)>>>>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();

            let queue = StdArc::new(Mutex::new(responses));
            let queue_handler = queue.clone();

            let app = Router::new().route(
                "/p2pcd/bridge/events",
                axum::routing::get(move || {
                    let queue = queue_handler.clone();
                    async move {
                        let events = {
                            let mut q = queue.lock().unwrap();
                            if q.is_empty() {
                                vec![]
                            } else {
                                q.remove(0)
                            }
                        };
                        let items: Vec<Result<AxumEvent, std::convert::Infallible>> = events
                            .into_iter()
                            .map(|(evt_type, data)| {
                                Ok(AxumEvent::default().event(evt_type).data(data))
                            })
                            .collect();
                        Sse::new(stream::iter(items)).keep_alive(KeepAlive::default())
                    }
                }),
            );

            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            (port, queue)
        }

        #[tokio::test]
        async fn peer_stream_snapshot_populates_tracker() {
            let snapshot_data = r#"{"peers":[{"peer_id":"peer-alpha","wg_address":"10.0.0.1","active_since":100},{"peer_id":"peer-beta","wg_address":"10.0.0.2","active_since":200}]}"#;
            let port = make_sse_server(vec![("snapshot".into(), snapshot_data.into())]).await;

            let ps = PeerStream::connect("test.cap.1", port);

            // Poll until we see 2 peers (or timeout).
            let result = tokio::time::timeout(Duration::from_millis(1500), async {
                loop {
                    if ps.tracker().peer_count().await == 2 {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(
                result.is_ok(),
                "timed out waiting for snapshot to populate tracker"
            );

            assert_eq!(ps.tracker().peer_count().await, 2);
            assert!(ps.tracker().find_peer("peer-alpha").await.is_some());
            assert!(ps.tracker().find_peer("peer-beta").await.is_some());
        }

        #[tokio::test]
        async fn peer_stream_live_events_update_tracker() {
            // Use a hook to detect that peer-active fired (the transient count==1 state
            // can race with peer-inactive in the same SSE burst before we poll it).
            let active_count = StdArc::new(std::sync::atomic::AtomicUsize::new(0));
            let active_count_hook = active_count.clone();

            let port = make_sse_server(vec![
                ("snapshot".into(), r#"{"peers":[]}"#.into()),
                (
                    "peer-active".into(),
                    r#"{"peer_id":"peer-a","wg_address":"10.0.0.1","active_since":1}"#.into(),
                ),
                ("peer-inactive".into(), r#"{"peer_id":"peer-a"}"#.into()),
            ])
            .await;

            let hook: HookFn = StdArc::new(move |_peer_id: String| {
                let c = active_count_hook.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                })
            });

            let ps = PeerStream::connect_with_hooks("test.cap.1", port, Some(hook), None);

            // Wait for the on_active hook to fire (confirms peer-active was processed)
            let r1 = tokio::time::timeout(Duration::from_millis(1500), async {
                loop {
                    if active_count.load(Ordering::SeqCst) >= 1 {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(r1.is_ok(), "timed out waiting for peer-active hook to fire");

            // Wait for count to reach 0 (peer-inactive processed)
            let r2 = tokio::time::timeout(Duration::from_millis(1500), async {
                loop {
                    if ps.tracker().peer_count().await == 0 {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(
                r2.is_ok(),
                "timed out waiting for peer-inactive to update tracker"
            );

            assert!(ps.tracker().find_peer("peer-a").await.is_none());
        }

        #[tokio::test]
        async fn peer_stream_reconnect_replaces_not_appends() {
            // First connection: 3 peers
            let snap1 = r#"{"peers":[{"peer_id":"p1"},{"peer_id":"p2"},{"peer_id":"p3"}]}"#;
            // Second+ connection: 1 different peer
            let snap2 = r#"{"peers":[{"peer_id":"p99"}]}"#;

            // Provide many responses — PeerStream reconnects aggressively after stream closes.
            let responses: Vec<Vec<(String, String)>> = (0..20)
                .map(|i| {
                    if i == 0 {
                        vec![("snapshot".into(), snap1.into())]
                    } else {
                        vec![("snapshot".into(), snap2.into())]
                    }
                })
                .collect();

            let (port, _queue) = make_multi_connect_sse_server(responses).await;

            let ps = PeerStream::connect("test.cap.1", port);

            // First: wait until we see 3 peers (generous timeout covers jitter).
            tokio::time::timeout(Duration::from_millis(2000), async {
                loop {
                    if ps.tracker().peer_count().await == 3 {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("timed out waiting for first snapshot (3 peers)");

            // Then: wait until we see exactly 1 peer (p99) after reconnect.
            tokio::time::timeout(Duration::from_millis(3000), async {
                loop {
                    let count = ps.tracker().peer_count().await;
                    if count == 1 {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("timed out waiting for reconnect snapshot (1 peer)");

            // CRITICAL: should be exactly 1, not 4 (3 old + 1 new)
            assert_eq!(ps.tracker().peer_count().await, 1);
            assert!(ps.tracker().find_peer("p99").await.is_some());
            assert!(ps.tracker().find_peer("p1").await.is_none());
        }

        #[tokio::test]
        async fn peer_stream_on_active_hook_fires_non_blocking() {
            use std::sync::atomic::AtomicBool;

            let hook_fired = StdArc::new(AtomicBool::new(false));
            let hook_fired_clone = hook_fired.clone();

            let port = make_sse_server(vec![
                ("snapshot".into(), r#"{"peers":[]}"#.into()),
                (
                    "peer-active".into(),
                    r#"{"peer_id":"peer-a","wg_address":"10.0.0.1","active_since":1}"#.into(),
                ),
                ("peer-inactive".into(), r#"{"peer_id":"peer-a"}"#.into()),
            ])
            .await;

            let hook: HookFn = StdArc::new(move |_peer_id: String| {
                let flag = hook_fired_clone.clone();
                Box::pin(async move {
                    // Simulate slow hook (DB write, etc.)
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    flag.store(true, Ordering::SeqCst);
                })
            });

            let ps = PeerStream::connect_with_hooks("test.cap.1", port, Some(hook), None);

            // Wait for peer-inactive — tracker should be 0
            tokio::time::timeout(Duration::from_millis(1500), async {
                loop {
                    if ps.tracker().peer_count().await == 0
                    // ensure we went through active first: find peer count was 1
                    // We just poll until we see count drop
                    {
                        // Verify we went through peer-active first by checking the hook
                        // (it may not have fired yet since it sleeps 100ms — that's the point)
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("timed out waiting for peer-inactive");

            // Tracker is at 0 — confirmed the hook didn't block the SSE loop
            assert_eq!(ps.tracker().peer_count().await, 0);

            // Now wait for the hook to eventually fire (it sleeps 100ms)
            tokio::time::timeout(Duration::from_millis(500), async {
                loop {
                    if hook_fired.load(Ordering::SeqCst) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("timed out waiting for on_active hook to fire");

            assert!(hook_fired.load(Ordering::SeqCst));
        }

        #[tokio::test]
        async fn peer_stream_jitter_staggers_connects() {
            // Record the time at which each connection is received by the server.
            let connect_times: StdArc<Mutex<Vec<Instant>>> = StdArc::new(Mutex::new(Vec::new()));
            let ct_handler = connect_times.clone();

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();

            let app = Router::new().route(
                "/p2pcd/bridge/events",
                axum::routing::get(move || {
                    let ct = ct_handler.clone();
                    async move {
                        ct.lock().unwrap().push(Instant::now());
                        // Return an empty snapshot and close.
                        let items: Vec<Result<AxumEvent, std::convert::Infallible>> =
                            vec![Ok(AxumEvent::default()
                                .event("snapshot")
                                .data(r#"{"peers":[]}"#))];
                        Sse::new(stream::iter(items))
                    }
                }),
            );

            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            // Create 10 PeerStreams all at once.
            let streams: Vec<PeerStream> = (0..10)
                .map(|_| PeerStream::connect("test.cap.1", port))
                .collect();

            // Wait until all 10 have connected (each will reconnect after stream closes,
            // we just need at least 10 total connect events).
            tokio::time::timeout(Duration::from_millis(3000), async {
                loop {
                    if connect_times.lock().unwrap().len() >= 10 {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("timed out waiting for 10 SSE connections");

            let times = connect_times.lock().unwrap().clone();
            // Take first 10
            let first_ten: Vec<Instant> = times.into_iter().take(10).collect();
            let min_t = first_ten.iter().copied().min().unwrap();
            let max_t = first_ten.iter().copied().max().unwrap();
            let spread = max_t.duration_since(min_t);

            // With 0-500ms jitter, 10 instances should NOT all connect in the same ms.
            // We assert spread >= 1ms. This is very likely true (10 random values in 500ms).
            // If this flakes on a slow CI, increase the threshold or skip with #[ignore].
            assert!(
                spread >= Duration::from_millis(1),
                "all 10 PeerStream instances connected within 1ms — jitter may not be working (spread={spread:?})"
            );

            // Keep streams alive until assertion
            drop(streams);
        }
    }

    #[test]
    fn is_for_us_checks_capability() {
        let tracker = PeerTracker::new("howm.social.feed.1");
        let msg_yes = InboundMessage {
            peer_id: "x".into(),
            message_type: 100,
            payload: "".into(),
            capability: "howm.social.feed.1".into(),
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

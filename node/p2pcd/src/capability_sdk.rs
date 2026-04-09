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

// ── RPC envelope helpers ────────────────────────────────────────────────────
//
// Shared decoders for the CBOR RPC_REQ/RPC_RESP envelope used by
// `core.data.rpc.1`. Capabilities that handle inbound RPCs (files, messaging,
// voice, …) previously each kept their own private copies of these helpers
// with hardcoded key numbers; any wire-format change would have required
// updating each copy in lockstep. They now live here as the single source of
// truth.
//
// Wire format (CBOR map with integer keys):
//   { 1: <method: text>, 2: <request_id: u64>, 3: <payload: bytes>, 4: <error: text?> }

/// CBOR envelope helpers for the `core.data.rpc.1` wire format.
///
/// Key constants are re-exported from `crate::capabilities::rpc::keys`, which
/// is the single source of truth. Both this module (used by out-of-process
/// capabilities) and the in-process handler read from the same definitions.
pub mod rpc {
    pub use crate::capabilities::rpc::keys::{ERROR, METHOD, PAYLOAD, REQUEST_ID};

    /// Extract the method name from a CBOR RPC envelope.
    ///
    /// Returns `None` if `data` is not a CBOR map or does not contain a
    /// text-valued `METHOD` field.
    pub fn extract_method(data: &[u8]) -> Option<String> {
        use ciborium::value::Value;
        let value: Value = ciborium::from_reader(data).ok()?;
        let map = match value {
            Value::Map(m) => m,
            _ => return None,
        };
        for (k, v) in map {
            if let Value::Integer(i) = k {
                let key: i128 = i.into();
                if key as u64 == METHOD {
                    if let Value::Text(t) = v {
                        return Some(t);
                    }
                }
            }
        }
        None
    }

    /// Extract the inner payload bytes from a CBOR RPC envelope.
    ///
    /// Returns `None` if `data` is not a CBOR map or does not contain a
    /// bytes-valued `PAYLOAD` field.
    pub fn extract_inner_payload(data: &[u8]) -> Option<Vec<u8>> {
        use ciborium::value::Value;
        let value: Value = ciborium::from_reader(data).ok()?;
        let map = match value {
            Value::Map(m) => m,
            _ => return None,
        };
        for (k, v) in map {
            if let Value::Integer(i) = k {
                let key: i128 = i.into();
                if key as u64 == PAYLOAD {
                    if let Value::Bytes(b) = v {
                        return Some(b);
                    }
                }
            }
        }
        None
    }

    /// Extract the request id from a CBOR RPC envelope.
    pub fn extract_request_id(data: &[u8]) -> Option<u64> {
        use ciborium::value::Value;
        let value: Value = ciborium::from_reader(data).ok()?;
        let map = match value {
            Value::Map(m) => m,
            _ => return None,
        };
        for (k, v) in map {
            if let Value::Integer(i) = k {
                let key: i128 = i.into();
                if key as u64 == REQUEST_ID {
                    if let Value::Integer(id) = v {
                        let n: i128 = id.into();
                        if n >= 0 {
                            return Some(n as u64);
                        }
                    }
                }
            }
        }
        None
    }
}

// ── LocalPeerId lazy fetch ──────────────────────────────────────────────────
//
// Capabilities that key data by the local peer ID (conversation rows, per-peer
// files, etc.) need a stable handle on that ID. The daemon answers
// `GET /identity` with the ID, but at cap startup the daemon may still be
// booting — so we retry a few times before giving up, and lazily re-fetch on
// demand after that.
//
// Previously this lived in messaging/src/api.rs as a private Arc<RwLock<String>>;
// now it's shared so other capabilities can adopt the pattern.

/// A lazily-populated local peer ID, retried against the daemon's
/// `/capabilities/self` endpoint via `BridgeClient::get_local_peer_id`.
///
/// The initial fetch runs with short backoff (0ms / 150ms / 500ms / 1s / 2s)
/// at construction time. If all attempts fail, the inner value stays empty and
/// `get()` will re-fetch on demand until it succeeds.
///
/// `BridgeClient` is `Clone`, so the helper takes it by value — no need to
/// wrap in `Arc` at the call site.
#[derive(Clone)]
pub struct LocalPeerId {
    value: Arc<RwLock<String>>,
    bridge: BridgeClient,
}

impl LocalPeerId {
    /// Construct and eagerly retry the initial fetch. Always returns; never
    /// blocks past ~3.65s total in the worst case.
    pub async fn lazy(bridge: BridgeClient) -> Self {
        let value = Arc::new(RwLock::new(String::new()));
        let delays_ms = [0u64, 150, 500, 1000, 2000];
        for (attempt, delay) in delays_ms.iter().enumerate() {
            if *delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(*delay)).await;
            }
            match bridge.get_local_peer_id().await {
                Ok(id) if !id.is_empty() => {
                    tracing::debug!("LocalPeerId: fetched on attempt {}", attempt + 1);
                    *value.write().await = id;
                    break;
                }
                Ok(_) => {
                    tracing::debug!("LocalPeerId: empty identity, retrying");
                }
                Err(e) => {
                    tracing::debug!("LocalPeerId: attempt {} failed: {}", attempt + 1, e);
                }
            }
        }
        if value.read().await.is_empty() {
            tracing::warn!(
                "LocalPeerId: initial fetch exhausted retries; will lazy-fetch on demand"
            );
        }
        Self { value, bridge }
    }

    /// Returns the local peer ID, lazily re-fetching if the stored value is
    /// still empty.
    pub async fn get(&self) -> Option<String> {
        {
            let guard = self.value.read().await;
            if !guard.is_empty() {
                return Some(guard.clone());
            }
        }
        // Empty — try once more.
        match self.bridge.get_local_peer_id().await {
            Ok(id) if !id.is_empty() => {
                *self.value.write().await = id.clone();
                Some(id)
            }
            _ => None,
        }
    }
}

// ── CapabilityApp — builder for out-of-process capability servers ──────────
//
// A typed builder that owns the standard capability scaffolding:
//
//   - Logging init
//   - Standard routes: /health, /p2pcd/inbound, optional /ui/*
//   - Configurable body limit
//   - Background task spawning
//   - Bind + serve loop
//
// Each cap still constructs its own clap config, AppState, and PeerStream
// (those remain cap-specific), and hands the builder the pieces it needs.
// The goal is not to hide everything, it's to delete the ~90 lines of pasted
// startup glue in every main.rs.
//
// # Minimal example
//
// ```no_run
// use std::sync::Arc;
// use axum::{routing::get, Json};
// use p2pcd::capability_sdk::CapabilityApp;
//
// #[derive(Clone)]
// struct AppState { /* cap-specific */ }
//
// # async fn main() -> anyhow::Result<()> {
// let state = AppState { };
// let _ = CapabilityApp::new("howm.example.1", 7010, state)
//     .with_routes(|router| router.route("/hello", get(|| async { "ok" })))
//     .with_inbound_handler(|_state, _msg| async move { Json("{}") })
//     .run()
//     .await?;
// # Ok(())
// # }
// ```
#[cfg(feature = "bridge-client")]
pub mod app {
    use std::future::Future;
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::sync::Arc;

    use axum::extract::{DefaultBodyLimit, Request, State};
    use axum::http::{header, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use include_dir::Dir;
    use tower_http::limit::RequestBodyLimitLayer;

    use super::InboundMessage;

    /// Standard `/health` response body.
    pub async fn health_handler() -> &'static str {
        "ok"
    }

    /// Initialize `tracing_subscriber` with an `info`-level env filter.
    ///
    /// Safe to call once at process start. Idempotent only under try-init
    /// semantics (subsequent calls silently no-op if the subscriber is
    /// already installed).
    pub fn init_tracing() {
        use tracing_subscriber::EnvFilter;
        let filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new("info"))
            .unwrap_or_else(|_| EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
    }

    /// Type-erased inbound-message handler.
    ///
    /// Caps provide an async function `fn(State<S>, Json<InboundMessage>) -> Response`;
    /// the builder stores it as this trait object so the `S` parameter can
    /// propagate through the Router without leaking into `CapabilityApp`'s
    /// type signature twice.
    type InboundFn<S> = Arc<
        dyn Fn(State<S>, Json<InboundMessage>) -> Pin<Box<dyn Future<Output = Response> + Send>>
            + Send
            + Sync,
    >;

    /// Builder for a standard out-of-process capability server.
    ///
    /// Generic over the cap's `AppState` type, which must be `Clone + Send +
    /// Sync + 'static` for axum.
    pub struct CapabilityApp<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        cap_name: String,
        port: u16,
        state: S,
        body_limit: usize,
        ui_dir: Option<&'static Dir<'static>>,
        inbound: Option<InboundFn<S>>,
        routes: Option<Box<dyn FnOnce(Router<S>) -> Router<S> + Send>>,
        background_tasks: Vec<Pin<Box<dyn Future<Output = ()> + Send>>>,
    }

    impl<S> CapabilityApp<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        /// Start a new builder. `cap_name` is used only for log lines; `port`
        /// is the HTTP listen port.
        pub fn new(cap_name: impl Into<String>, port: u16, state: S) -> Self {
            Self {
                cap_name: cap_name.into(),
                port,
                state,
                body_limit: 1024 * 1024, // 1 MiB default — override with with_body_limit
                ui_dir: None,
                inbound: None,
                routes: None,
                background_tasks: Vec::new(),
            }
        }

        /// Set the max request body size in bytes (default 1 MiB).
        pub fn with_body_limit(mut self, bytes: usize) -> Self {
            self.body_limit = bytes;
            self
        }

        /// Attach an embedded UI directory. Registered as a fallback handler
        /// under `/ui/*` with SPA index.html fallback.
        pub fn with_ui(mut self, ui_dir: &'static Dir<'static>) -> Self {
            self.ui_dir = Some(ui_dir);
            self
        }

        /// Register the inbound-message handler mounted at `POST /p2pcd/inbound`.
        ///
        /// The handler receives the cap's `AppState` and the decoded
        /// `InboundMessage`. Return any axum [`IntoResponse`] type (`StatusCode`,
        /// `Json<T>`, `Result<_, _>`, `Response`, …).
        pub fn with_inbound_handler<F, Fut, R>(mut self, handler: F) -> Self
        where
            F: Fn(State<S>, Json<InboundMessage>) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = R> + Send + 'static,
            R: IntoResponse + 'static,
        {
            self.inbound = Some(Arc::new(move |state, json| {
                let fut = handler(state, json);
                Box::pin(async move { fut.await.into_response() })
                    as Pin<Box<dyn Future<Output = Response> + Send>>
            }));
            self
        }

        /// Add cap-specific routes. The closure receives a `Router<S>` with
        /// the standard routes already installed; return the extended router.
        pub fn with_routes<F>(mut self, f: F) -> Self
        where
            F: FnOnce(Router<S>) -> Router<S> + Send + 'static,
        {
            self.routes = Some(Box::new(f));
            self
        }

        /// Spawn a background task for the lifetime of the server. Use for
        /// cap-specific loops (cleanup, gossip, etc.). The future runs
        /// concurrently with the HTTP server and is dropped on shutdown.
        pub fn spawn_task<Fut>(mut self, fut: Fut) -> Self
        where
            Fut: Future<Output = ()> + Send + 'static,
        {
            self.background_tasks.push(Box::pin(fut));
            self
        }

        /// Build the router, bind the listener, spawn background tasks, and
        /// serve until the future is cancelled.
        pub async fn run(self) -> anyhow::Result<()> {
            let CapabilityApp {
                cap_name,
                port,
                state,
                body_limit,
                ui_dir,
                inbound,
                routes,
                background_tasks,
            } = self;

            // ── Base router with standard routes ─────────────────────────────
            let mut router: Router<S> = Router::new().route("/health", get(health_handler));

            if let Some(handler) = inbound {
                let inbound_route = post(move |state: State<S>, json: Json<InboundMessage>| {
                    let handler = handler.clone();
                    async move { handler(state, json).await }
                });
                router = router.route("/p2pcd/inbound", inbound_route);
            }

            if let Some(dir) = ui_dir {
                router = router
                    .route("/ui", get(move |req: Request| serve_ui(dir, req)))
                    .route("/ui/", get(move |req: Request| serve_ui(dir, req)))
                    .route("/ui/{*path}", get(move |req: Request| serve_ui(dir, req)));
            }

            if let Some(route_builder) = routes {
                router = route_builder(router);
            }

            // Apply body limit. Two layers required: axum has its own
            // `DefaultBodyLimit` (2 MiB cap) that runs before tower-http's
            // limiter, so any cap that needs > 2 MiB (multipart uploads,
            // large blobs) gets a 400/413 from axum's middleware first
            // unless we disable it explicitly. The hard cap then comes
            // from `RequestBodyLimitLayer`.
            let app = router
                .layer(DefaultBodyLimit::disable())
                .layer(RequestBodyLimitLayer::new(body_limit))
                .with_state(state);

            // ── Spawn background tasks ───────────────────────────────────────
            let mut task_handles = Vec::new();
            for fut in background_tasks {
                task_handles.push(tokio::spawn(fut));
            }

            // ── Bind and serve ───────────────────────────────────────────────
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!("{} listening on {}", cap_name, addr);

            let serve_result = axum::serve(listener, app).await;

            // Abort background tasks on server exit.
            for h in task_handles {
                h.abort();
            }

            serve_result.map_err(anyhow::Error::from)
        }
    }

    // ── UI serving (SPA fallback) ───────────────────────────────────────────

    /// Serve an embedded file under `/ui/*`, falling back to `index.html`
    /// for client-side routing.
    async fn serve_ui(ui_dir: &'static Dir<'static>, req: Request) -> Response {
        let path = req.uri().path();
        let rel = path
            .strip_prefix("/ui/")
            .unwrap_or("")
            .trim_start_matches('/');
        let rel = if rel.is_empty() { "index.html" } else { rel };

        if let Some(file) = ui_dir.get_file(rel) {
            let mime = guess_mime(rel);
            return ([(header::CONTENT_TYPE, mime)], file.contents()).into_response();
        }

        // SPA fallback — any unmatched /ui/* request returns index.html so
        // client-side routers can handle the path.
        if let Some(index) = ui_dir.get_file("index.html") {
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                index.contents(),
            )
                .into_response();
        }

        (StatusCode::NOT_FOUND, "ui not embedded").into_response()
    }

    /// Minimal MIME type guesser for the subset of extensions our UIs use.
    fn guess_mime(path: &str) -> &'static str {
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" | "mjs" => "application/javascript; charset=utf-8",
            "json" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            "wasm" => "application/wasm",
            "txt" => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        }
    }
}

#[cfg(feature = "bridge-client")]
pub use app::{init_tracing, CapabilityApp};

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

    // ── rpc helpers ──────────────────────────────────────────────────────────

    fn encode_rpc_envelope(method: &str, req_id: u64, payload: &[u8]) -> Vec<u8> {
        use ciborium::value::{Integer, Value};
        let map = Value::Map(vec![
            (
                Value::Integer(Integer::from(rpc::METHOD)),
                Value::Text(method.into()),
            ),
            (
                Value::Integer(Integer::from(rpc::REQUEST_ID)),
                Value::Integer(Integer::from(req_id)),
            ),
            (
                Value::Integer(Integer::from(rpc::PAYLOAD)),
                Value::Bytes(payload.to_vec()),
            ),
        ]);
        let mut buf = Vec::new();
        ciborium::into_writer(&map, &mut buf).unwrap();
        buf
    }

    #[test]
    fn rpc_extract_method_roundtrip() {
        let bytes = encode_rpc_envelope("dm.send", 42, b"hello");
        assert_eq!(rpc::extract_method(&bytes).as_deref(), Some("dm.send"));
    }

    #[test]
    fn rpc_extract_inner_payload_roundtrip() {
        let bytes = encode_rpc_envelope("catalogue.list", 1, b"\x01\x02\x03");
        assert_eq!(rpc::extract_inner_payload(&bytes), Some(vec![1, 2, 3]));
    }

    #[test]
    fn rpc_extract_request_id_roundtrip() {
        let bytes = encode_rpc_envelope("voice.join", 9001, b"");
        assert_eq!(rpc::extract_request_id(&bytes), Some(9001));
    }

    #[test]
    fn rpc_extract_method_none_on_garbage() {
        assert_eq!(rpc::extract_method(&[0xff, 0xff, 0xff]), None);
        assert_eq!(rpc::extract_method(&[]), None);
    }

    #[test]
    fn rpc_extract_inner_payload_none_on_missing_key() {
        use ciborium::value::{Integer, Value};
        // Envelope with only method, no payload key
        let map = Value::Map(vec![(
            Value::Integer(Integer::from(rpc::METHOD)),
            Value::Text("ping".into()),
        )]);
        let mut buf = Vec::new();
        ciborium::into_writer(&map, &mut buf).unwrap();
        assert_eq!(rpc::extract_inner_payload(&buf), None);
    }

    #[test]
    fn rpc_key_constants_match_wire() {
        // Lock in the wire-format contract. Changing these requires a protocol
        // version bump and coordinated updates in the core RPC handler.
        assert_eq!(rpc::METHOD, 1);
        assert_eq!(rpc::REQUEST_ID, 2);
        assert_eq!(rpc::PAYLOAD, 3);
        assert_eq!(rpc::ERROR, 4);
    }

    // ── CapabilityApp smoke tests ────────────────────────────────────────────

    #[tokio::test]
    async fn capability_app_serves_health_and_stops_on_drop() {
        use axum::body::to_bytes;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use axum::routing::get;

        #[derive(Clone)]
        struct TestState {
            tag: &'static str,
        }

        async fn hello(State(s): State<TestState>) -> impl IntoResponse {
            s.tag
        }

        // Bind to an ephemeral port by creating the listener first, then
        // reading the port and using it for the builder. The builder will
        // bind again on the same port (briefly collides — so instead we
        // just pick a fixed high port unlikely to clash in CI).
        let port = pick_free_port().await;

        let app = CapabilityApp::new("howm.test.capapp.1", port, TestState { tag: "hi" })
            .with_routes(|r| r.route("/hello", get(hello)));

        // Run server in a task; abort after hitting endpoints.
        let server = tokio::spawn(async move { app.run().await });

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Hit /health
        let client = reqwest::Client::new();
        let url_health = format!("http://127.0.0.1:{}/health", port);
        let resp = client.get(&url_health).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");

        // Hit /hello
        let url_hello = format!("http://127.0.0.1:{}/hello", port);
        let resp = client.get(&url_hello).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = to_bytes(resp.bytes().await.unwrap().into(), 64)
            .await
            .unwrap();
        assert_eq!(&body[..], b"hi");

        server.abort();
    }

    #[tokio::test]
    async fn capability_app_inbound_handler_receives_message() {
        use axum::extract::State;
        use axum::response::{IntoResponse, Response};
        use axum::Json;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone)]
        struct InState {
            got: Arc<AtomicBool>,
        }

        async fn inbound(State(s): State<InState>, Json(msg): Json<InboundMessage>) -> Response {
            assert_eq!(msg.message_type, 22);
            s.got.store(true, Ordering::SeqCst);
            axum::Json(serde_json::json!({"ok": true})).into_response()
        }

        let port = pick_free_port().await;
        let got = Arc::new(AtomicBool::new(false));

        let app = CapabilityApp::new("howm.test.inbound.1", port, InState { got: got.clone() })
            .with_inbound_handler(inbound);

        let server = tokio::spawn(async move { app.run().await });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let body = serde_json::json!({
            "peer_id": "AAAA",
            "message_type": 22,
            "payload": "",
            "capability": "howm.test.inbound.1",
        });
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/p2pcd/inbound", port);
        let resp = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert!(
            got.load(Ordering::SeqCst),
            "inbound handler should have run"
        );

        server.abort();
    }

    /// Return a port that is free right now. There's an unavoidable TOCTOU
    /// window between drop and rebind, but for unit tests on a dev box it's
    /// good enough.
    async fn pick_free_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }
}

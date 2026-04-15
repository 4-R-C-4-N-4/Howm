// P2P-CD Capability Notification Interface — Task 6.2
//
// When a session reaches ACTIVE or leaves ACTIVE, the daemon notifies all
// registered capabilities via HTTP POST to their local port.
//
// Delivery:
//   POST http://127.0.0.1:<cap_port>/p2pcd/peer-active   — peer became available
//   POST http://127.0.0.1:<cap_port>/p2pcd/peer-inactive — peer no longer available
//
// Capabilities can also poll:
//   GET /p2pcd/peers-for/:capability_name (daemon API endpoint, Task 7.1)
//
// Design: The daemon is a gatekeeper, not a proxy. After notification, the
// capability opens its own TCP/HTTP connections to the peer's WG address.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use p2pcd_types::{PeerId, ScopeParams};

use super::event_bus::{CapEvent, EventBus};

// ── Wire types for HTTP callbacks ────────────────────────────────────────────

/// Payload sent to a capability when a peer becomes active for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerActivePayload {
    /// Base64-encoded WireGuard public key (= peer_id).
    pub peer_id: String,
    /// Peer's WireGuard IP address (for the capability to connect directly).
    pub wg_address: String,
    /// Capability name this notification is for.
    pub capability: String,
    /// Agreed scope params for this capability (may be default/zero).
    pub scope: ScopeParams,
    /// Unix timestamp of when the session became ACTIVE.
    pub active_since: u64,
}

/// Payload sent to a capability when a peer is no longer available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInactivePayload {
    pub peer_id: String,
    pub capability: String,
    pub reason: String,
}

/// Payload forwarded to a capability when an inbound CapabilityMsg arrives
/// that has no in-process handler (i.e. app-level message types).
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

// ── Capability registry ───────────────────────────────────────────────────────

/// A registered capability endpoint that receives P2P-CD peer notifications.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CapabilityEndpoint {
    /// Capability name (e.g. \"howm.social.feed.1\").
    pub cap_name: String,
    /// Local port the capability is listening on.
    pub port: u16,
    /// Optional full base URL override (used in tests to point at a mock server).
    /// When set, overrides the default `http://127.0.0.1:<port>` base.
    url_override: Option<String>,
}

/// Registry of capability endpoints to notify.
/// Populated from the capabilities module at engine startup.
pub struct CapabilityNotifier {
    endpoints: RwLock<HashMap<String, CapabilityEndpoint>>,
    event_bus: Arc<EventBus>,
}

impl CapabilityNotifier {
    pub fn new(event_bus: Arc<EventBus>) -> Arc<Self> {
        Arc::new(Self {
            endpoints: RwLock::new(HashMap::new()),
            event_bus,
        })
    }

    /// Register a capability to receive peer notifications (default URL: 127.0.0.1:<port>).
    pub async fn register(&self, cap_name: String, port: u16) {
        let endpoint = CapabilityEndpoint {
            cap_name: cap_name.clone(),
            port,
            url_override: None,
        };
        self.endpoints.write().await.insert(cap_name, endpoint);
    }

    /// Register with a full base URL override — used in integration tests to
    /// point at a mock HTTP server instead of a real capability on localhost.
    #[cfg(test)]
    pub async fn register_with_url(&self, cap_name: String, base_url: String) {
        let endpoint = CapabilityEndpoint {
            cap_name: cap_name.clone(),
            port: 0,
            url_override: Some(base_url),
        };
        self.endpoints.write().await.insert(cap_name, endpoint);
    }

    /// Unregister a capability.
    #[allow(dead_code)]
    pub async fn unregister(&self, cap_name: &str) {
        self.endpoints.write().await.remove(cap_name);
    }

    /// Notify all capabilities that have `cap_name` in their active_set
    /// that a peer is now available.
    pub async fn notify_peer_active(
        &self,
        peer_id: PeerId,
        wg_address: IpAddr,
        active_set: &[String],
        scope_params: &std::collections::BTreeMap<String, ScopeParams>,
        active_since: u64,
    ) {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let peer_id_b64 = STANDARD.encode(peer_id);

        let endpoints = self.endpoints.read().await;
        for cap_name in active_set {
            if let Some(ep) = endpoints.get(cap_name) {
                let payload = PeerActivePayload {
                    peer_id: peer_id_b64.clone(),
                    wg_address: wg_address.to_string(),
                    capability: cap_name.clone(),
                    scope: scope_params.get(cap_name).cloned().unwrap_or_default(),
                    active_since,
                };
                let base = ep
                    .url_override
                    .clone()
                    .unwrap_or_else(|| format!("http://127.0.0.1:{}", ep.port));
                let url = format!("{}/p2pcd/peer-active", base);
                tokio::spawn(post_notification(url, payload));
            }
            // Also publish to the in-process event bus (runs regardless of endpoint registration).
            self.event_bus.publish(CapEvent::PeerActive {
                peer_id: peer_id_b64.clone(),
                wg_address: wg_address.to_string(),
                capability: cap_name.clone(),
                scope: scope_params.get(cap_name).cloned().unwrap_or_default(),
                active_since,
            });
        }
    }

    /// Forward an inbound capability message to the appropriate out-of-process capability.
    ///
    /// Called by the dispatch loop when a message_type has no registered in-process handler.
    /// Looks up the capability name in the active_set, finds its endpoint, and POSTs
    /// the message to `POST /p2pcd/inbound` on that capability's HTTP server.
    pub async fn forward_to_capability(
        &self,
        peer_id: PeerId,
        message_type: u64,
        payload: &[u8],
        active_set: &[String],
    ) -> bool {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let endpoints = self.endpoints.read().await;

        // Find which app-level capability in the active_set has an endpoint registered.
        // Core caps are handled in-process, so we only match app caps here.
        for cap_name in active_set {
            if let Some(ep) = endpoints.get(cap_name) {
                let base = ep
                    .url_override
                    .clone()
                    .unwrap_or_else(|| format!("http://127.0.0.1:{}", ep.port));
                let url = format!("{}/p2pcd/inbound", base);

                let peer_id_b64 = STANDARD.encode(peer_id);
                let payload_b64 = STANDARD.encode(payload);

                let body = InboundMessage {
                    peer_id: peer_id_b64.clone(),
                    message_type,
                    payload: payload_b64.clone(),
                    capability: cap_name.clone(),
                };

                tokio::spawn(post_inbound_with_retry(url, body));

                // Also publish to the in-process event bus.
                self.event_bus.publish(CapEvent::Inbound {
                    peer_id: peer_id_b64,
                    capability: cap_name.clone(),
                    message_type,
                    payload: payload_b64,
                });

                return true;
            }
        }
        false
    }

    /// Forward an RPC request to a capability and **await** the response.
    ///
    /// Unlike `forward_to_capability()` (fire-and-forget), this waits for the
    /// HTTP response so the caller can build an RPC_RESP with the result.
    /// The capability returns `{ "response": "<base64-encoded CBOR>" }` on
    /// success.
    pub async fn forward_rpc_to_capability(
        &self,
        peer_id: PeerId,
        method: &str,
        payload: &[u8],
        active_set: &[String],
    ) -> anyhow::Result<Vec<u8>> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let endpoints = self.endpoints.read().await;
        let registered: Vec<String> = endpoints.keys().cloned().collect();

        // Route RPC method → owning capability. Method names are conventionally
        // prefixed with the capability's domain (`dm.*`, `catalogue.*`, `voice.*`,
        // etc.), so we match by prefix. If no prefix matches, we fall back to
        // iterating the full active_set (legacy behavior).
        //
        // TODO: replace with a registered method table built from capability
        // manifests once the SDK refactor (R1/R2) lands.
        let preferred_cap: Option<&'static str> = match method.split('.').next().unwrap_or("") {
            "dm" | "conversation" => Some("howm.social.messaging.1"),
            "catalogue" | "blob" => Some("howm.social.files.1"),
            "voice" => Some("howm.social.voice.1"),
            "feed" | "post" => Some("howm.social.feed.1"),
            "presence" => Some("howm.social.presence.1"),
            "room" | "world" => Some("howm.world.room.1"),
            _ => None,
        };

        tracing::debug!(
            "forward_rpc: method='{}' preferred_cap={:?} active_set={:?} registered={:?}",
            method,
            preferred_cap,
            active_set,
            registered,
        );

        // Strict routing: if we know which cap owns this method, only try
        // that one. Otherwise fall back to iterating the full active_set.
        let ordered: Vec<&String> = if let Some(pref) = preferred_cap {
            let matched: Vec<&String> = active_set.iter().filter(|c| c.as_str() == pref).collect();
            if matched.is_empty() {
                tracing::warn!(
                    "forward_rpc: preferred cap '{}' for method '{}' is not in peer's active_set",
                    pref,
                    method,
                );
            }
            matched
        } else {
            active_set.iter().collect()
        };

        for cap_name in ordered {
            if let Some(ep) = endpoints.get(cap_name) {
                let base = ep
                    .url_override
                    .clone()
                    .unwrap_or_else(|| format!("http://127.0.0.1:{}", ep.port));
                let url = format!("{}/p2pcd/inbound", base);

                let peer_id_b64 = STANDARD.encode(peer_id);
                let payload_b64 = STANDARD.encode(payload);

                let body = InboundMessage {
                    peer_id: peer_id_b64,
                    message_type: p2pcd_types::message_types::RPC_REQ,
                    payload: payload_b64,
                    capability: cap_name.clone(),
                };

                tracing::debug!(
                    "forward_rpc: POST {} payload_b64_len={}",
                    url,
                    body.payload.len(),
                );

                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .unwrap_or_default();

                let resp = client.post(&url).json(&body).send().await.map_err(|e| {
                    tracing::warn!("forward_rpc: POST {} send error: {}", url, e);
                    anyhow::anyhow!("RPC forward to {}: {}", cap_name, e)
                })?;

                let status = resp.status();
                if !status.is_success() {
                    let body_text = resp.text().await.unwrap_or_default();
                    anyhow::bail!(
                        "RPC forward to {} returned {} (body: {:?})",
                        cap_name,
                        status,
                        body_text.chars().take(200).collect::<String>()
                    );
                }

                // Read body as text first so we can log it on parse failure.
                let body_text = resp
                    .text()
                    .await
                    .map_err(|e| anyhow::anyhow!("RPC forward read body: {}", e))?;

                tracing::debug!(
                    "forward_rpc: {} → {} ({} body bytes)",
                    cap_name,
                    status,
                    body_text.len(),
                );

                // Empty body is valid: capability accepted the message but
                // returned no payload (e.g. fire-and-forget RPCs like voice.join).
                if body_text.is_empty() {
                    return Ok(vec![]);
                }

                // Parse the response JSON for { "response": "<base64>" }
                let resp_json: serde_json::Value =
                    serde_json::from_str(&body_text).map_err(|e| {
                        anyhow::anyhow!(
                            "RPC forward response parse: {} | raw body ({} bytes): {:?}",
                            e,
                            body_text.len(),
                            body_text.chars().take(200).collect::<String>()
                        )
                    })?;

                if let Some(resp_b64) = resp_json.get("response").and_then(|v| v.as_str()) {
                    let decoded = STANDARD
                        .decode(resp_b64)
                        .map_err(|e| anyhow::anyhow!("RPC forward response decode: {}", e))?;
                    return Ok(decoded);
                }

                // No "response" field — capability handled it but returned no payload.
                // Return empty success.
                return Ok(vec![]);
            }
        }

        tracing::warn!(
            "forward_rpc: NO ENDPOINT for method '{}' — active_set={:?} registered={:?}",
            method,
            active_set,
            registered,
        );
        anyhow::bail!("no capability endpoint for RPC method '{}'", method)
    }

    /// Notify all capabilities that a peer is no longer available.
    pub async fn notify_peer_inactive(&self, peer_id: PeerId, active_set: &[String], reason: &str) {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let peer_id_b64 = STANDARD.encode(peer_id);

        let endpoints = self.endpoints.read().await;
        for cap_name in active_set {
            if let Some(ep) = endpoints.get(cap_name) {
                let payload = PeerInactivePayload {
                    peer_id: peer_id_b64.clone(),
                    capability: cap_name.clone(),
                    reason: reason.to_string(),
                };
                let base = ep
                    .url_override
                    .clone()
                    .unwrap_or_else(|| format!("http://127.0.0.1:{}", ep.port));
                let url = format!("{}/p2pcd/peer-inactive", base);
                tokio::spawn(post_inactive_notification(url, payload));
            }
            // Also publish to the in-process event bus (runs regardless of endpoint registration).
            self.event_bus.publish(CapEvent::PeerInactive {
                peer_id: peer_id_b64.clone(),
                capability: cap_name.clone(),
                reason: reason.to_string(),
            });
        }
    }
}

/// Fire-and-forget HTTP POST for peer-active notification.
async fn post_notification(url: String, payload: PeerActivePayload) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    match client.post(&url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!("cap_notify: POST {} → {}", url, resp.status());
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            // 404 is expected for capabilities that have migrated to the SSE event stream
            // and intentionally removed their /p2pcd/peer-active HTTP endpoint.
            tracing::debug!(
                "cap_notify: POST {} → 404 (capability uses SSE, skipping POST)",
                url
            );
        }
        Ok(resp) => {
            tracing::warn!("cap_notify: POST {} returned {}", url, resp.status());
        }
        Err(e) => {
            tracing::debug!(
                "cap_notify: POST {} failed (cap may not be running): {}",
                url,
                e
            );
        }
    }
}

/// HTTP POST for inbound capability message forwarding with retry-with-backoff.
/// Makes up to 4 attempts with delays: 0, 100ms, 500ms, 2000ms.
async fn post_inbound_with_retry(url: String, body: InboundMessage) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    for (attempt, delay_ms) in [0u64, 100, 500, 2000].iter().enumerate() {
        if *delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
        }
        match client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                tracing::debug!(
                    "cap_notify: inbound delivered to {} on attempt {}",
                    url,
                    attempt + 1
                );
                return;
            }
            Ok(r) => {
                tracing::warn!("cap_notify: inbound POST {} returned {}", url, r.status());
            }
            Err(e) if attempt < 3 => {
                tracing::debug!("cap_notify: inbound POST {} failed ({e}), retrying", url);
            }
            Err(e) => {
                tracing::warn!(
                    "cap_notify: inbound POST {} failed after 4 attempts: {e}",
                    url
                );
            }
        }
    }
}

async fn post_inactive_notification(url: String, payload: PeerInactivePayload) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    match client.post(&url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!("cap_notify: POST {} → {}", url, resp.status());
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            tracing::debug!(
                "cap_notify: POST {} → 404 (capability uses SSE, skipping POST)",
                url
            );
        }
        Ok(resp) => {
            tracing::warn!("cap_notify: POST {} returned {}", url, resp.status());
        }
        Err(e) => {
            tracing::debug!(
                "cap_notify: POST {} failed (cap may not be running): {}",
                url,
                e
            );
        }
    }
}

// ── RpcForwarder impl ────────────────────────────────────────────────────────

impl p2pcd::capabilities::rpc::RpcForwarder for CapabilityNotifier {
    fn forward_rpc(
        &self,
        peer_id: PeerId,
        method: &str,
        payload: &[u8],
        active_set: &[String],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>>
    {
        let method = method.to_string();
        let payload = payload.to_vec();
        let active_set = active_set.to_vec();
        Box::pin(async move {
            self.forward_rpc_to_capability(peer_id, &method, &payload, &active_set)
                .await
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use std::collections::BTreeMap;
    use tokio::net::TcpListener;

    async fn handle_peer_active(Json(_body): Json<PeerActivePayload>) -> axum::http::StatusCode {
        axum::http::StatusCode::OK
    }

    async fn handle_peer_inactive(
        Json(_body): Json<PeerInactivePayload>,
    ) -> axum::http::StatusCode {
        axum::http::StatusCode::OK
    }

    #[tokio::test]
    async fn notifier_sends_to_registered_cap() {
        use crate::p2pcd::event_bus::{CapEvent, EventBus};

        // Spin up a tiny axum server simulating a capability
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let router = Router::new()
            .route("/p2pcd/peer-active", post(handle_peer_active))
            .route("/p2pcd/peer-inactive", post(handle_peer_inactive));

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let event_bus = std::sync::Arc::new(EventBus::new());
        let notifier = CapabilityNotifier::new(std::sync::Arc::clone(&event_bus));
        notifier
            .register("core.session.heartbeat.1".to_string(), port)
            .await;

        let peer_id = [1u8; 32];
        let wg_addr: IpAddr = "100.222.0.2".parse().unwrap();
        let active_set = vec!["core.session.heartbeat.1".to_string()];
        let scope = BTreeMap::new();

        // Subscribe before calling notify to ensure we don't miss the event.
        let mut bus_rx = event_bus.subscribe();

        // Should not panic
        notifier
            .notify_peer_active(peer_id, wg_addr, &active_set, &scope, 1234)
            .await;

        // Assert the CapEvent::PeerActive appeared on the bus.
        let event = bus_rx.recv().await.expect("expected bus event");
        match event {
            CapEvent::PeerActive {
                capability,
                active_since,
                ..
            } => {
                assert_eq!(capability, "core.session.heartbeat.1");
                assert_eq!(active_since, 1234);
            }
            other => panic!("unexpected event: {:?}", other),
        }

        // Give the spawned HTTP call time to fire
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        notifier
            .notify_peer_inactive(peer_id, &active_set, "Normal")
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn notifier_ignores_unregistered_cap() {
        use crate::p2pcd::event_bus::{CapEvent, EventBus};
        use std::time::Duration;

        let event_bus = std::sync::Arc::new(EventBus::new());
        let notifier = CapabilityNotifier::new(std::sync::Arc::clone(&event_bus));
        let peer_id = [2u8; 32];
        let active_set = vec!["unknown.cap.1".to_string()];
        let scope = BTreeMap::new();

        // Subscribe before calling notify_peer_active so we don't miss the event.
        let mut bus_rx = event_bus.subscribe();

        // Should not panic even if cap not registered in endpoints map.
        notifier
            .notify_peer_active(
                peer_id,
                "100.222.0.3".parse().unwrap(),
                &active_set,
                &scope,
                0,
            )
            .await;

        // Give the spawn a moment to propagate.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The bus fires unconditionally regardless of endpoint registration.
        let event = bus_rx.recv().await.expect("expected bus event");
        match event {
            CapEvent::PeerActive { capability, .. } => {
                assert_eq!(capability, "unknown.cap.1");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn post_inbound_with_retry_succeeds_on_third_attempt() {
        use axum::{extract::State, response::IntoResponse};
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Shared counter for how many requests the server has received.
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_srv = std::sync::Arc::clone(&counter);

        async fn handler(
            State(counter): State<std::sync::Arc<AtomicUsize>>,
            Json(_body): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 3 {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            } else {
                axum::http::StatusCode::OK
            }
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let router = axum::Router::new()
            .route("/p2pcd/inbound", post(handler))
            .with_state(counter_srv);

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        // Give the server a moment to start.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let url = format!("http://127.0.0.1:{}/p2pcd/inbound", port);
        let body = InboundMessage {
            peer_id: "AAAA".to_string(),
            message_type: 1,
            payload: "dGVzdA==".to_string(),
            capability: "test.cap.1".to_string(),
        };

        // Guard against hanging — the delays are 0+100+500 = 600ms max before 3rd attempt.
        tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            post_inbound_with_retry(url, body),
        )
        .await
        .expect("post_inbound_with_retry timed out");

        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "expected exactly 3 requests"
        );
    }

    #[test]
    fn payloads_serialize() {
        let p = PeerActivePayload {
            peer_id: "AAAA".to_string(),
            wg_address: "100.222.0.2".to_string(),
            capability: "core.session.heartbeat.1".to_string(),
            scope: ScopeParams::default(),
            active_since: 0,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("peer_id"));

        let q: PeerActivePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(q.capability, "core.session.heartbeat.1");
    }
}

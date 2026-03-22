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

// ── Capability registry ───────────────────────────────────────────────────────

/// A registered capability endpoint that receives P2P-CD peer notifications.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CapabilityEndpoint {
    /// Capability name (e.g. \"p2pcd.social.post.1\").
    pub cap_name: String,
    /// Local port the capability is listening on.
    pub port: u16,
    /// Optional full base URL override (used in tests to point at a mock server).
    /// When set, overrides the default `http://127.0.0.1:<port>` base.
    url_override: Option<String>,
}

/// Registry of capability endpoints to notify.
/// Populated from the capabilities module at engine startup.
#[derive(Default)]
pub struct CapabilityNotifier {
    endpoints: RwLock<HashMap<String, CapabilityEndpoint>>,
}

impl CapabilityNotifier {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            endpoints: RwLock::new(HashMap::new()),
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
        }
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

async fn post_inactive_notification(url: String, payload: PeerInactivePayload) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let _ = client.post(&url).json(&payload).send().await;
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

        let notifier = CapabilityNotifier::new();
        notifier
            .register("core.session.heartbeat.1".to_string(), port)
            .await;

        let peer_id = [1u8; 32];
        let wg_addr: IpAddr = "100.222.0.2".parse().unwrap();
        let active_set = vec!["core.session.heartbeat.1".to_string()];
        let scope = BTreeMap::new();

        // Should not panic
        notifier
            .notify_peer_active(peer_id, wg_addr, &active_set, &scope, 0)
            .await;
        // Give the spawned HTTP call time to fire
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        notifier
            .notify_peer_inactive(peer_id, &active_set, "Normal")
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn notifier_ignores_unregistered_cap() {
        let notifier = CapabilityNotifier::new();
        let peer_id = [2u8; 32];
        let active_set = vec!["unknown.cap.1".to_string()];
        let scope = BTreeMap::new();
        // Should not panic even if cap not registered
        notifier
            .notify_peer_active(
                peer_id,
                "100.222.0.3".parse().unwrap(),
                &active_set,
                &scope,
                0,
            )
            .await;
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

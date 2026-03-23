// Bridge client — HTTP interface for out-of-process capabilities
//
// Any capability process (social-feed, etc.) uses this client to talk to the
// daemon's bridge endpoints:
//
//   send_msg()        — send a raw CapabilityMsg to a specific peer
//   rpc_call()        — send an RPC request, wait for the response
//   broadcast_event() — broadcast an event to all peers with a given capability
//   list_peers()      — list active peers (optionally filtered by capability)
//
// The client handles base64 encoding/decoding and serialization. Capability
// code just works with Rust types.
//
// # Example
//
// ```no_run
// use p2pcd::bridge_client::BridgeClient;
//
// let client = BridgeClient::new(7000);
// let peers = client.list_peers(Some("howm.social.feed.1")).await?;
// client.broadcast_event("howm.social.feed.1", 100, &payload).await?;
// ```

use base64::Engine;
use serde::{Deserialize, Serialize};

// ── Client ──────────────────────────────────────────────────────────────────────

/// HTTP client for the daemon's bridge endpoints.
///
/// Created once at capability startup; cloned freely (reqwest::Client is
/// Arc-backed internally).
#[derive(Clone)]
pub struct BridgeClient {
    http: reqwest::Client,
    base_url: String,
}

impl BridgeClient {
    /// Create a new bridge client pointing at the daemon on `localhost:port`.
    pub fn new(daemon_port: u16) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            http,
            base_url: format!("http://127.0.0.1:{}/p2pcd/bridge", daemon_port),
        }
    }

    /// Create with a custom base URL (for testing or non-standard setups).
    pub fn with_base_url(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { http, base_url }
    }

    // ── Send ────────────────────────────────────────────────────────────────

    /// Send a raw capability message to a specific peer.
    ///
    /// `peer_id` is the 32-byte WireGuard public key.
    /// `message_type` is the capability message type number (6+).
    /// `payload` is the CBOR-encoded message body.
    pub async fn send_msg(
        &self,
        peer_id: &[u8; 32],
        message_type: u64,
        payload: &[u8],
    ) -> Result<(), BridgeError> {
        let body = SendRequest {
            peer_id: encode_b64(peer_id),
            message_type,
            payload: encode_b64(payload),
        };

        let resp = self
            .http
            .post(format!("{}/send", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(BridgeError::Http)?;

        let status = resp.status();
        let result: SendResponse = resp.json().await.map_err(BridgeError::Http)?;

        if result.ok {
            Ok(())
        } else {
            Err(BridgeError::Bridge {
                status: status.as_u16(),
                message: result.error.unwrap_or_else(|| "unknown error".into()),
            })
        }
    }

    // ── RPC ─────────────────────────────────────────────────────────────────

    /// Send an RPC request to a specific peer and wait for the response.
    ///
    /// Returns the CBOR-encoded response payload bytes.
    /// Times out after `timeout_ms` milliseconds (default 5000).
    pub async fn rpc_call(
        &self,
        peer_id: &[u8; 32],
        method: &str,
        payload: &[u8],
        timeout_ms: Option<u64>,
    ) -> Result<Vec<u8>, BridgeError> {
        let body = RpcRequest {
            peer_id: encode_b64(peer_id),
            method: method.to_string(),
            payload: encode_b64(payload),
            timeout_ms: timeout_ms.unwrap_or(5000),
        };

        let resp = self
            .http
            .post(format!("{}/rpc", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(BridgeError::Http)?;

        let status = resp.status();
        let result: RpcResponse = resp.json().await.map_err(BridgeError::Http)?;

        if result.ok {
            match result.payload {
                Some(b64) => decode_b64(&b64).map_err(|e| BridgeError::Decode(e.to_string())),
                None => Ok(vec![]),
            }
        } else {
            Err(BridgeError::Bridge {
                status: status.as_u16(),
                message: result.error.unwrap_or_else(|| "unknown error".into()),
            })
        }
    }

    // ── Event broadcast ─────────────────────────────────────────────────────

    /// Broadcast an event to all peers that negotiated the given capability.
    ///
    /// Returns the number of peers the event was sent to.
    pub async fn broadcast_event(
        &self,
        capability: &str,
        message_type: u64,
        payload: &[u8],
    ) -> Result<usize, BridgeError> {
        let body = EventRequest {
            capability: capability.to_string(),
            message_type,
            payload: encode_b64(payload),
        };

        let resp = self
            .http
            .post(format!("{}/event", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(BridgeError::Http)?;

        let status = resp.status();
        let result: EventResponse = resp.json().await.map_err(BridgeError::Http)?;

        if result.ok {
            Ok(result.sent_to)
        } else {
            Err(BridgeError::Bridge {
                status: status.as_u16(),
                message: result.error.unwrap_or_else(|| "unknown error".into()),
            })
        }
    }

    // ── Peer listing ────────────────────────────────────────────────────────

    /// List active peers, optionally filtered by capability name.
    pub async fn list_peers(&self, capability: Option<&str>) -> Result<Vec<PeerInfo>, BridgeError> {
        let mut url = format!("{}/peers", self.base_url);
        if let Some(cap) = capability {
            url.push_str(&format!("?capability={}", cap));
        }

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(BridgeError::Http)?;

        if resp.status().is_success() {
            let peers: Vec<PeerInfo> = resp.json().await.map_err(BridgeError::Http)?;
            Ok(peers)
        } else {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            Err(BridgeError::Bridge {
                status,
                message: text,
            })
        }
    }

    /// Check if the daemon bridge is reachable.
    pub async fn is_available(&self) -> bool {
        self.list_peers(None).await.is_ok()
    }
}

// ── Error type ──────────────────────────────────────────────────────────────────

/// Errors from bridge client operations.
#[derive(Debug)]
pub enum BridgeError {
    /// HTTP transport error (connection refused, timeout, etc.)
    Http(reqwest::Error),
    /// Bridge returned an error response.
    Bridge { status: u16, message: String },
    /// Failed to decode a response payload.
    Decode(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Http(e) => write!(f, "bridge HTTP error: {e}"),
            BridgeError::Bridge { status, message } => {
                write!(f, "bridge error ({status}): {message}")
            }
            BridgeError::Decode(e) => write!(f, "bridge decode error: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BridgeError::Http(e) => Some(e),
            _ => None,
        }
    }
}

// ── Wire types (mirror daemon/src/p2pcd/bridge.rs) ──────────────────────────────

#[derive(Debug, Serialize)]
struct SendRequest {
    peer_id: String,
    message_type: u64,
    payload: String,
}

#[derive(Debug, Deserialize)]
struct SendResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RpcRequest {
    peer_id: String,
    method: String,
    payload: String,
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    ok: bool,
    payload: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct EventRequest {
    capability: String,
    message_type: u64,
    payload: String,
}

#[derive(Debug, Deserialize)]
struct EventResponse {
    ok: bool,
    sent_to: usize,
    #[allow(dead_code)]
    error: Option<String>,
}

/// Info about an active peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Base64-encoded 32-byte peer ID.
    pub peer_id: String,
    /// Capabilities this peer has negotiated.
    pub capabilities: Vec<String>,
}

impl PeerInfo {
    /// Decode the peer_id from base64 to a 32-byte array.
    pub fn peer_id_bytes(&self) -> Result<[u8; 32], String> {
        let bytes = decode_b64(&self.peer_id).map_err(|e| format!("bad base64: {e}"))?;
        if bytes.len() != 32 {
            return Err(format!("expected 32 bytes, got {}", bytes.len()));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        Ok(id)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────────

fn encode_b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn decode_b64(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD.decode(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_client_new_builds_url() {
        let client = BridgeClient::new(7000);
        assert_eq!(client.base_url, "http://127.0.0.1:7000/p2pcd/bridge");
    }

    #[test]
    fn bridge_client_custom_url() {
        let client = BridgeClient::with_base_url("http://10.0.0.1:9999/bridge".into());
        assert_eq!(client.base_url, "http://10.0.0.1:9999/bridge");
    }

    #[test]
    fn peer_info_decode_valid() {
        let id = [42u8; 32];
        let info = PeerInfo {
            peer_id: encode_b64(&id),
            capabilities: vec!["test.cap.1".into()],
        };
        assert_eq!(info.peer_id_bytes().unwrap(), id);
    }

    #[test]
    fn peer_info_decode_wrong_length() {
        let info = PeerInfo {
            peer_id: encode_b64(&[1, 2, 3]),
            capabilities: vec![],
        };
        assert!(info.peer_id_bytes().is_err());
    }

    #[test]
    fn bridge_error_display() {
        let err = BridgeError::Bridge {
            status: 404,
            message: "peer not found".into(),
        };
        assert!(err.to_string().contains("404"));
        assert!(err.to_string().contains("peer not found"));
    }
}

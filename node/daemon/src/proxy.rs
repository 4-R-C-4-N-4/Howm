use axum::{
    body::Body,
    http::{Request, Response},
};
use reqwest::Client;
use std::time::Duration;
use tracing::warn;

use crate::{error::AppError, state::AppState};

/// Proxy a request to a local capability process.
///
/// `peer_pubkey`: If Some, the base64-encoded WG public key of the remote peer
/// making the request. Injected as `X-Peer-Id` header so the capability process
/// can identify the caller for its own authorization logic.
pub async fn proxy_request_with_peer(
    state: &AppState,
    cap_name: &str,
    rest_path: &str,
    req: Request<Body>,
    peer_pubkey: Option<&str>,
) -> Result<Response<Body>, AppError> {
    // Find capability by name — "social" matches "social.feed" (first segment before '.')
    let cap = {
        let caps = state.capabilities.read().await;
        caps.iter()
            .find(|c| {
                let first_seg = c.name.split('.').next().unwrap_or(&c.name);
                let last_seg = c.name.rsplit('.').next().unwrap_or(&c.name);
                first_seg == cap_name || last_seg == cap_name || c.name == cap_name
            })
            .cloned()
    };

    let cap =
        cap.ok_or_else(|| AppError::NotFound(format!("capability not found: {}", cap_name)))?;

    let port = cap.port;
    let target_url = format!(
        "http://localhost:{}/{}",
        port,
        rest_path.trim_start_matches('/')
    );

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let method = req.method().clone();
    let headers = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut proxy_req = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
        &target_url,
    );

    // Forward headers, skipping hop-by-hop ones
    for (name, value) in headers.iter() {
        let name_str = name.as_str();
        if !matches!(
            name_str,
            "host" | "connection" | "transfer-encoding" | "x-peer-id" | "x-node-id" | "x-node-name"
        ) {
            proxy_req = proxy_req.header(name_str, value.as_bytes());
        }
    }

    // Inject node identity headers so the capability knows which node it's running on
    proxy_req = proxy_req
        .header("X-Node-Id", &state.identity.node_id)
        .header("X-Node-Name", &state.identity.name);

    // Inject peer identity if this is a remote request
    if let Some(pubkey) = peer_pubkey {
        proxy_req = proxy_req.header("X-Peer-Id", pubkey);
    }

    if !body_bytes.is_empty() {
        proxy_req = proxy_req.body(body_bytes.to_vec());
    }

    let resp = proxy_req.send().await.map_err(|e| {
        warn!("Proxy failed for capability {}: {}", cap_name, e);
        AppError::PeerUnreachable(format!("capability unavailable: {}", e))
    })?;

    // Rebuild response for axum
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let resp_body = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut builder = Response::builder().status(status.as_u16());

    for (name, value) in resp_headers.iter() {
        let name_str = name.as_str();
        if !matches!(name_str, "transfer-encoding" | "connection") {
            builder = builder.header(name_str, value.as_bytes());
        }
    }

    let response = builder
        .body(Body::from(resp_body))
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(response)
}

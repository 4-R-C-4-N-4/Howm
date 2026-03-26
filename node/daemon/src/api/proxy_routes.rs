use axum::{
    body::Body,
    extract::{ConnectInfo, Path, State},
    http::Request,
    response::Response,
};
use std::net::SocketAddr;

use crate::{error::AppError, proxy, state::AppState};

/// Handler for `/cap/:name` (no trailing path) — proxies to the capability root.
pub async fn proxy_handler_root(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(name): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, AppError> {
    proxy_handler_inner(state, addr, name, String::new(), req).await
}

/// Handler for `/cap/:name/*rest` — proxies to a sub-path of the capability.
pub async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, rest)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, AppError> {
    proxy_handler_inner(state, addr, name, rest, req).await
}

async fn proxy_handler_inner(
    state: AppState,
    addr: SocketAddr,
    name: String,
    rest: String,
    req: Request<Body>,
) -> Result<Response<Body>, AppError> {
    let source_ip = addr.ip().to_string();
    let is_local = source_ip == "127.0.0.1" || source_ip == "::1";

    // ── Phase 4: AccessDb-backed capability enforcement ──────────────────

    if !is_local {
        // Resolve WG source IP → peer identity (wg_pubkey)
        let peer_pubkey = {
            let peers = state.peers.read().await;
            peers
                .iter()
                .find(|p| p.wg_address == source_ip)
                .map(|p| p.wg_pubkey.clone())
        };

        let peer_pubkey = match peer_pubkey {
            Some(pk) => pk,
            None => {
                // Unknown source IP — not a recognized WG peer
                return Err(AppError::Forbidden(
                    "unknown peer — not on WireGuard network".to_string(),
                ));
            }
        };

        // Decode WG pubkey to 32-byte peer_id
        let peer_bytes = {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            STANDARD
                .decode(&peer_pubkey)
                .map_err(|_| AppError::Internal("invalid base64 peer pubkey".to_string()))?
        };

        if peer_bytes.len() != 32 {
            return Err(AppError::Internal(
                "peer pubkey is not 32 bytes".to_string(),
            ));
        }

        // Map installed capability short name (e.g. "feed") to P2P-CD
        // fully-qualified name (e.g. "howm.social.feed.0") for permission check.
        // Deterministic: reads the p2pcd_name stored on the CapabilityEntry at install time.
        let p2pcd_cap_name = resolve_p2pcd_cap_name(&state, &name).await;

        if let Some(cap_name) = &p2pcd_cap_name {
            let perm = state.access_db.resolve_permission(&peer_bytes, cap_name);
            if !perm.is_allowed() {
                tracing::info!(
                    "access denied: peer {} for capability {} ({})",
                    &peer_pubkey[..8],
                    cap_name,
                    source_ip,
                );
                return Err(AppError::Forbidden(format!(
                    "access denied for capability '{}'",
                    cap_name
                )));
            }
        }
        // If no P2P-CD name mapping found, fall through to proxy
        // (the trust gate in P2P-CD is the primary gate; this is defense-in-depth)

        // Inject peer identity into the proxied request
        return proxy::proxy_request_with_peer(&state, &name, &rest, req, Some(&peer_pubkey)).await;
    }

    // Local requests always allowed — no peer identity injection
    proxy::proxy_request_with_peer(&state, &name, &rest, req, None).await
}

/// Map an installed capability's proxy route name to its P2P-CD fully-qualified name.
///
/// Deterministic lookup: find the installed capability by route_name (or name),
/// then read its `p2pcd_name` field which was set at install time.
/// No segment-scanning or guessing — the mapping is explicit.
async fn resolve_p2pcd_cap_name(state: &AppState, short_name: &str) -> Option<String> {
    let caps = state.capabilities.read().await;
    let cap = caps
        .iter()
        .find(|c| c.route_name.as_deref() == Some(short_name) || c.name == short_name);

    match cap {
        Some(c) => c.p2pcd_name.clone(),
        None => None,
    }
}

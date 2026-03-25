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
        // fully-qualified name (e.g. "howm.feed.1") for permission check.
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

/// Map an installed capability short name to its P2P-CD fully-qualified name.
///
/// Installed capabilities use short names like "feed" while AccessDb
/// rules use P2P-CD names like "howm.social.feed.1".
///
/// Resolution: scan the P2P-CD engine's local manifest for any capability
/// whose second or third namespace segment matches the short name.
/// e.g. "feed" matches "howm.social.feed.1" (third segment),
///      "social" matches "howm.social.feed.1" (second segment).
/// Falls back to `howm.{short_name}.1` if no manifest match.
async fn resolve_p2pcd_cap_name(state: &AppState, short_name: &str) -> Option<String> {
    if let Some(engine) = &state.p2pcd_engine {
        let manifest = engine.local_manifest().await;
        for cap in &manifest.capabilities {
            let parts: Vec<&str> = cap.name.split('.').collect();
            if parts.len() >= 3 {
                // Match against any middle segment (between prefix and version)
                // e.g. for "howm.social.feed.1", check "social" and "feed"
                let middle = &parts[1..parts.len() - 1];
                if middle.contains(&short_name) {
                    return Some(cap.name.clone());
                }
                // Also match the joined middle: "social.feed"
                let joined = middle.join(".");
                if joined == short_name {
                    return Some(cap.name.clone());
                }
            }
        }
    }

    // Fallback: construct from convention
    let candidate = format!("howm.{}.1", short_name);
    Some(candidate)
}

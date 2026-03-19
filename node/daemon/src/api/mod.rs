use crate::state::AppState;
use axum::{
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{any, delete, get, post},
    Router,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

pub mod auth_layer;
pub mod capability_routes;
pub mod network_routes;
pub mod node_routes;
pub mod p2pcd_routes;
pub mod proxy_routes;
pub mod settings_routes;

/// Check if a source IP is localhost or on the WireGuard subnet (100.222.0.0/16).
pub(crate) fn is_local_or_wg(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || is_wg_subnet(*v4),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn is_wg_subnet(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && o[1] == 222
}

/// Single router for the daemon.
///
/// Three route groups:
///   1. `authenticated` — bearer token required (local owner mutations)
///   2. `local_or_wg`  — no bearer, but restricted to localhost + WG subnet
///   3. `peer_ceremony` — fully public (invite handshake over the internet)
pub fn build_router(state: AppState, ui_dir: Option<PathBuf>) -> Router {
    // ── 1. Authenticated routes (bearer token required) ──────────────────
    let authenticated = Router::new()
        .route("/node/peers/:node_id", delete(node_routes::remove_peer))
        .route(
            "/node/peers/:node_id/trust",
            axum::routing::patch(node_routes::update_peer_trust),
        )
        .route("/node/invite", post(node_routes::create_invite))
        .route("/node/redeem-invite", post(node_routes::redeem_invite))
        .route(
            "/node/open-invite",
            get(node_routes::get_open_invite)
                .post(node_routes::create_open_invite)
                .delete(node_routes::revoke_open_invite),
        )
        .route(
            "/node/redeem-open-invite",
            post(node_routes::redeem_open_invite),
        )
        .route(
            "/capabilities/install",
            post(capability_routes::install_capability),
        )
        .route(
            "/capabilities/:name/stop",
            post(capability_routes::stop_capability),
        )
        .route(
            "/capabilities/:name/start",
            post(capability_routes::start_capability),
        )
        .route(
            "/capabilities/:name",
            delete(capability_routes::uninstall_capability),
        )
        .route("/p2pcd/friends", post(p2pcd_routes::p2pcd_add_friend))
        .route(
            "/p2pcd/friends/:pubkey",
            delete(p2pcd_routes::p2pcd_remove_friend),
        )
        .route(
            "/settings/p2pcd",
            axum::routing::put(settings_routes::update_p2pcd_config),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bearer_auth_middleware,
        ));

    // ── 2. Local/WG-only routes (read-only, no bearer, subnet-restricted) ─
    let local_or_wg = Router::new()
        .route("/node/info", get(node_routes::get_info))
        .route("/node/peers", get(node_routes::get_peers))
        .route("/node/wireguard", get(node_routes::get_wg_status))
        .route("/capabilities", get(capability_routes::list_capabilities))
        .route(
            "/network/capabilities",
            get(network_routes::network_capabilities),
        )
        .route(
            "/network/capability/:name",
            get(network_routes::find_capability_providers),
        )
        .route("/network/feed", get(network_routes::network_feed))
        .route("/cap/:name/*rest", any(proxy_routes::proxy_handler))
        .route("/p2pcd/status", get(p2pcd_routes::p2pcd_status))
        .route("/p2pcd/sessions", get(p2pcd_routes::p2pcd_sessions))
        .route(
            "/p2pcd/sessions/:peer_id",
            get(p2pcd_routes::p2pcd_session_detail),
        )
        .route("/p2pcd/manifest", get(p2pcd_routes::p2pcd_manifest))
        .route("/p2pcd/cache", get(p2pcd_routes::p2pcd_cache))
        .route("/p2pcd/peers-for/:cap", get(p2pcd_routes::p2pcd_peers_for))
        .route("/p2pcd/friends", get(p2pcd_routes::p2pcd_list_friends))
        .route("/settings/node", get(settings_routes::get_node_settings))
        .route("/settings/identity", get(settings_routes::get_identity))
        .route("/settings/p2pcd", get(settings_routes::get_p2pcd_config))
        .layer(middleware::from_fn(local_or_wg_middleware));

    // ── 3. Peer ceremony routes (public internet, no bearer, no IP check) ─
    let peer_ceremony = Router::new()
        .route("/node/complete-invite", post(node_routes::complete_invite))
        .route("/node/open-join", post(node_routes::open_join));

    let mut router = Router::new()
        .merge(authenticated)
        .merge(local_or_wg)
        .merge(peer_ceremony);

    // Serve static UI files if --ui-dir is provided; fall back to embedded UI.
    // Fallback must be set before .with_state() so embedded handler can access State<AppState>.
    if let Some(dir) = ui_dir {
        if dir.exists() {
            let index = dir.join("index.html");
            router = router.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(index)));
            tracing::info!("Serving UI from {}", dir.display());
        } else {
            tracing::warn!(
                "UI directory {} does not exist — using embedded UI",
                dir.display()
            );
            router = router.fallback(crate::embedded_ui::serve_embedded);
        }
    } else {
        router = router.fallback(crate::embedded_ui::serve_embedded);
    }

    router.with_state(state)
}

/// Middleware: restrict to localhost or WireGuard subnet (100.222.0.0/16).
async fn local_or_wg_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if is_local_or_wg(&addr.ip()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Bearer token auth middleware (S2).
/// Checks `Authorization: Bearer <token>` against the stored API token.
async fn bearer_auth_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = header.trim_start_matches("Bearer ").trim();
            if token == state.api_token {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::FORBIDDEN)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

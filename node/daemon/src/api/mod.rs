use axum::{
    Router,
    routing::{get, post, delete, any},
};
use crate::state::AppState;

pub mod node_routes;
pub mod capability_routes;
pub mod network_routes;
pub mod proxy_routes;
pub mod auth_layer;

/// Local management API (127.0.0.1, bearer token required for mutations)
pub fn build_local_router(state: AppState) -> Router {
    Router::new()
        // Node routes
        .route("/node/info", get(node_routes::get_info))
        .route("/node/peers", get(node_routes::get_peers))
        .route("/node/peers/:node_id", delete(node_routes::remove_peer))
        .route("/node/invite", post(node_routes::create_invite))
        .route("/node/redeem-invite", post(node_routes::redeem_invite))
        .route("/node/wireguard", get(node_routes::get_wg_status))
        // Capability routes
        .route("/capabilities", get(capability_routes::list_capabilities))
        .route("/capabilities/install", post(capability_routes::install_capability))
        .route("/capabilities/:name/stop", post(capability_routes::stop_capability))
        .route("/capabilities/:name/start", post(capability_routes::start_capability))
        .route("/capabilities/:name", delete(capability_routes::uninstall_capability))
        // Network routes
        .route("/network/capabilities", get(network_routes::network_capabilities))
        .route("/network/capability/:name", get(network_routes::find_capability_providers))
        .route("/network/feed", get(network_routes::network_feed))
        // Proxy
        .route("/cap/:name/*rest", any(proxy_routes::proxy_handler))
        .with_state(state)
}

/// Peer API (WG address only, no extra auth — WG tunnel IS the auth)
pub fn build_peer_router(state: AppState) -> Router {
    Router::new()
        .route("/node/info", get(node_routes::get_info))
        .route("/node/complete-invite", post(node_routes::complete_invite))
        .route("/capabilities", get(capability_routes::list_capabilities))
        .route("/cap/:name/*rest", any(proxy_routes::proxy_handler))
        .with_state(state)
}

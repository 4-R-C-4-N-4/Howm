use axum::{
    Router,
    routing::{get, post, delete, any},
};
use crate::state::AppState;

pub mod node_routes;
pub mod capability_routes;
pub mod network_routes;
pub mod proxy_routes;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Node routes
        .route("/node/info", get(node_routes::get_info))
        .route("/node/peers", get(node_routes::get_peers))
        .route("/node/peers", post(node_routes::add_peer))
        .route("/node/peers/:node_id", delete(node_routes::remove_peer))
        .route("/node/invite", post(node_routes::create_invite))
        .route("/node/redeem-invite", post(node_routes::redeem_invite))
        .route("/node/consume-invite", post(node_routes::consume_invite))
        .route("/node/auth-keys", get(node_routes::list_auth_keys))
        .route("/node/auth-keys", post(node_routes::add_auth_key))
        .route("/node/auth-keys/:prefix", delete(node_routes::remove_auth_key))
        .route("/node/tailnet", get(node_routes::get_tailnet))
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

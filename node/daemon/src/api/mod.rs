use crate::state::AppState;
use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{any, delete, get, post},
    Router,
};
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

pub mod auth_layer;
pub mod capability_routes;
pub mod network_routes;
pub mod node_routes;
pub mod proxy_routes;

/// Single router for the daemon.
///
/// Bearer token is required on all POST/PUT/DELETE routes EXCEPT:
///   - /node/complete-invite (uses PSK-based auth, called by remote peers)
///   - /node/info and other GET routes (read-only)
pub fn build_router(state: AppState, ui_dir: Option<PathBuf>) -> Router {
    // Routes that require bearer token for mutations
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bearer_auth_middleware,
        ));

    // Routes that don't need bearer token
    let open = Router::new()
        // Read-only node info
        .route("/node/info", get(node_routes::get_info))
        .route("/node/peers", get(node_routes::get_peers))
        .route("/node/wireguard", get(node_routes::get_wg_status))
        // complete-invite: called by remote peer using PSK auth, no bearer needed
        .route("/node/complete-invite", post(node_routes::complete_invite))
        // open-join: called by remote peer to join via open invite, no bearer needed
        .route("/node/open-join", post(node_routes::open_join))
        // Read-only capability/network info
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
        // Proxy
        .route("/cap/:name/*rest", any(proxy_routes::proxy_handler));

    let mut router = Router::new()
        .merge(authenticated)
        .merge(open)
        .with_state(state);

    // Serve static UI files if --ui-dir is provided.
    // Fallback to index.html for SPA client-side routing.
    if let Some(dir) = ui_dir {
        if dir.exists() {
            let index = dir.join("index.html");
            router = router.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(index)));
            tracing::info!("Serving UI from {}", dir.display());
        } else {
            tracing::warn!("UI directory {} does not exist, skipping", dir.display());
        }
    }

    router
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

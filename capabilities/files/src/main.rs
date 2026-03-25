use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
    Router,
};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

static UI_ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui");

mod api;
mod db;

#[derive(Parser, Debug)]
#[command(name = "files", about = "Howm file transfer offerings capability")]
struct Config {
    #[arg(long, default_value = "7003", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: PathBuf,

    /// Port the Howm daemon HTTP API listens on.
    #[arg(long, default_value = "7000", env = "HOWM_DAEMON_PORT")]
    daemon_port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let files_db = db::FilesDb::open(&config.data_dir)?;
    let bridge = p2pcd::bridge_client::BridgeClient::new(config.daemon_port);

    let state = api::AppState::new(
        files_db,
        bridge,
        config.daemon_port,
        config.port,
        config.data_dir.clone(),
    );

    // Restore active peers from daemon on startup
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            api::init_peers_from_daemon(state_clone).await;
        });
    }

    let app = Router::new()
        // Health
        .route("/health", get(api::health))
        // Active peers list (for UI)
        .route("/peers", get(api::list_active_peers))
        // Operator offerings API
        .route(
            "/offerings",
            get(api::list_offerings).post(api::create_offering),
        )
        // JSON path for creating from pre-registered blob
        .route("/offerings/json", put(api::create_offering_json))
        .route(
            "/offerings/{offering_id}",
            patch(api::update_offering).delete(api::delete_offering),
        )
        // Peer catalogue browsing (wired in FEAT-003-E)
        .route("/peer/{peer_id}/catalogue", get(api::peer_catalogue))
        // Downloads (wired in FEAT-003-E)
        .route(
            "/downloads",
            get(api::list_downloads).post(api::initiate_download),
        )
        .route("/downloads/{blob_id}/status", get(api::download_status))
        .route("/downloads/{blob_id}/data", get(api::download_data))
        // P2P-CD lifecycle hooks (called by daemon cap_notify)
        .route("/p2pcd/peer-active", post(api::peer_active))
        .route("/p2pcd/peer-inactive", post(api::peer_inactive))
        .route("/p2pcd/inbound", post(api::inbound_message))
        // Internal: transfer-complete callback from daemon bridge
        .route("/internal/transfer-complete", post(api::transfer_complete))
        .with_state(state)
        .layer(DefaultBodyLimit::disable()) // Disable axum's 2MB default body limit for Multipart
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            500 * 1024 * 1024,
        )) // Hard 500MB cap on all requests
        // Embedded capability UI at /ui/*
        .fallback(serve_ui);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    info!("Files capability listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Embedded UI ──────────────────────────────────────────────────────────────

async fn serve_ui(req: Request<Body>) -> Response {
    let path = req.uri().path();
    let rel = path.strip_prefix("/ui").unwrap_or(path);
    let rel = rel.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };

    match UI_ASSETS.get_file(rel) {
        Some(file) => (
            [(header::CONTENT_TYPE, ui_mime(rel))],
            Body::from(file.contents()),
        )
            .into_response(),
        None => {
            if path.starts_with("/ui") {
                match UI_ASSETS.get_file("index.html") {
                    Some(index) => (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        Body::from(index.contents()),
                    )
                        .into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

fn ui_mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

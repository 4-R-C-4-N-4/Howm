use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
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
mod posts;

#[derive(Parser, Debug)]
#[command(name = "social-feed", about = "Howm social feed capability")]
struct Config {
    #[arg(long, default_value = "7001", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: PathBuf,

    /// Port the Howm daemon HTTP API listens on (for P2P-CD peer queries).
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

    let state = api::FeedState::new(config.data_dir.clone(), config.daemon_port);

    // Restore active peers from daemon on startup (Task 7.3)
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            api::init_peers_from_daemon(state_clone).await;
        });
    }

    let app = Router::new()
        // Existing feed endpoints
        .route("/feed",   get(api::get_feed))
        .route("/post",   post(api::create_post))
        .route("/health", get(api::health))
        // Active peer list (for debugging / UI)
        .route("/peers",  get(api::list_social_peers))
        // P2P-CD daemon callbacks (Task 7.3)
        .route("/p2pcd/peer-active",   post(api::p2pcd_peer_active))
        .route("/p2pcd/peer-inactive", post(api::p2pcd_peer_inactive))
        .with_state(state)
        // Embedded capability UI — served at /ui/*
        .fallback(serve_ui);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    info!("Social feed capability starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ── Embedded UI ──────────────────────────────────────────────────────────────

async fn serve_ui(req: Request<Body>) -> Response {
    let path = req.uri().path();
    // Strip /ui prefix; treat /ui and /ui/ as index.html
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
            // SPA fallback to index.html for unknown paths under /ui
            if path.starts_with("/ui") {
                match UI_ASSETS.get_file("index.html") {
                    Some(index) => (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        Body::from(index.contents()),
                    )
                        .into_response(),
                    None => (StatusCode::NOT_FOUND, "UI not found").into_response(),
                }
            } else {
                (StatusCode::NOT_FOUND, "Not found").into_response()
            }
        }
    }
}

fn ui_mime(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else {
        "application/octet-stream"
    }
}

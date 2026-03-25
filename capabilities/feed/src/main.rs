use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
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
mod blob_fetcher;
mod db;
mod posts;

#[derive(Parser, Debug)]
#[command(name = "feed", about = "Howm feed capability")]
struct Config {
    #[arg(long, default_value = "7001", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: PathBuf,

    /// Port the Howm daemon HTTP API listens on (for P2P-CD peer queries).
    #[arg(long, default_value = "7000", env = "HOWM_DAEMON_PORT")]
    daemon_port: u16,

    /// Max number of attachments per post.
    #[arg(long, default_value = "4", env = "MAX_ATTACHMENTS")]
    max_attachments: usize,

    /// Max image size in bytes (default 8 MiB).
    #[arg(long, default_value = "8388608", env = "MAX_IMAGE_BYTES")]
    max_image_bytes: u64,

    /// Max video size in bytes (default 50 MiB).
    #[arg(long, default_value = "52428800", env = "MAX_VIDEO_BYTES")]
    max_video_bytes: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();

    std::fs::create_dir_all(&config.data_dir)?;

    // Open SQLite database and run JSON migration if needed
    let feed_db = db::FeedDb::open(&config.data_dir)?;
    feed_db.migrate_from_json(&config.data_dir)?;

    let limits = posts::MediaLimits {
        max_attachments: config.max_attachments,
        max_image_bytes: config.max_image_bytes,
        max_video_bytes: config.max_video_bytes,
        ..Default::default()
    };
    let state = api::FeedState::new(config.data_dir.clone(), feed_db, config.daemon_port)
        .with_limits(limits);

    // Restore active peers from daemon on startup
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            api::init_peers_from_daemon(state_clone).await;
        });
    }

    // Resume any pending blob transfers from a previous run
    {
        let db_clone = state.db.clone();
        let bridge_clone = state.bridge().clone();
        tokio::spawn(async move {
            blob_fetcher::resume_active_transfers(db_clone, bridge_clone).await;
        });
    }

    let app = Router::new()
        // Feed endpoints (all paginated via ?limit=N&offset=N)
        .route("/feed", get(api::get_feed))
        .route("/feed/mine", get(api::get_my_feed))
        .route("/feed/peer/:peer_id", get(api::get_peer_feed))
        // Post CRUD — JSON for text-only, multipart for media attachments
        .route("/post", post(api::create_post))
        .route("/post/upload", post(api::create_post_multipart))
        .route("/post/limits", get(api::get_limits))
        .route("/post/:id", delete(api::delete_post))
        .route("/post/:id/attachments", get(api::get_attachment_status))
        // Blob serving (content-addressed media)
        .route("/blob/:hash", get(api::serve_blob))
        // Utility
        .route("/health", get(api::health))
        .route("/peers", get(api::list_peers))
        // P2P-CD daemon callbacks
        .route("/p2pcd/peer-active", post(api::p2pcd_peer_active))
        .route("/p2pcd/peer-inactive", post(api::p2pcd_peer_inactive))
        .route("/p2pcd/inbound", post(api::p2pcd_inbound))
        .with_state(state)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50 MB for media uploads
        // Embedded capability UI — served at /ui/*
        .fallback(serve_ui);

    let addr: SocketAddr = format!("127.0.0.1:{}", config.port).parse()?;
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

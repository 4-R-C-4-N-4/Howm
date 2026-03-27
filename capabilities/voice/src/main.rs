use axum::{
    extract::Path as AxumPath,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod signal;
mod state;

use signal::SignalHub;
use state::{RoomStore, VoiceConfig};

static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");

#[derive(Parser, Debug)]
#[command(name = "voice", about = "Howm voice chat capability")]
struct Config {
    #[arg(long, default_value = "7005", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: std::path::PathBuf,

    /// Port the Howm daemon HTTP API listens on.
    #[arg(long, default_value = "7000", env = "HOWM_DAEMON_PORT")]
    daemon_port: u16,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub rooms: RoomStore,
    pub signal_hub: SignalHub,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let voice_config = VoiceConfig::from_env();
    info!(
        "Voice config: max_room_size={}, room_timeout={}s, invite_timeout={}s",
        voice_config.max_room_size,
        voice_config.room_timeout_secs,
        voice_config.invite_timeout_secs
    );

    let state = AppState {
        rooms: RoomStore::new(voice_config),
        signal_hub: SignalHub::new(),
    };

    // Background: room cleanup loop
    {
        let cleanup_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let removed = cleanup_state.rooms.cleanup_stale_rooms();
                if removed > 0 {
                    info!("Cleaned up {} stale room(s)", removed);
                }
            }
        });
    }

    let app = Router::new()
        // Room management API
        .route("/rooms", post(api::create_room).get(api::list_rooms))
        .route(
            "/rooms/{room_id}",
            get(api::get_room).delete(api::close_room),
        )
        .route("/rooms/{room_id}/join", post(api::join_room))
        .route("/rooms/{room_id}/leave", post(api::leave_room))
        .route("/rooms/{room_id}/invite", post(api::invite_peers))
        .route("/rooms/{room_id}/mute", post(api::mute))
        // WebSocket signaling
        .route("/rooms/{room_id}/signal", get(signal::signal_ws))
        // Embedded UI
        .route("/ui", get(serve_ui_index))
        .route("/ui/", get(serve_ui_index))
        .route("/ui/{*path}", get(serve_ui_asset))
        // Health
        .route("/health", get(api::health))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    info!("Voice capability listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn serve_ui_index() -> impl IntoResponse {
    match UI_DIR.get_file("index.html") {
        Some(f) => Html(f.contents_utf8().unwrap_or("")).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_ui_asset(AxumPath(path): AxumPath<String>) -> impl IntoResponse {
    let rel = path.strip_prefix("/ui").unwrap_or(&path);
    let rel = rel.strip_prefix('/').unwrap_or(rel);
    if rel.is_empty() {
        return serve_ui_index().await.into_response();
    }
    match UI_DIR.get_file(rel) {
        Some(f) => {
            let mime = if rel.ends_with(".js") {
                "application/javascript"
            } else if rel.ends_with(".css") {
                "text/css"
            } else if rel.ends_with(".html") {
                "text/html"
            } else {
                "application/octet-stream"
            };
            Response::builder()
                .header("content-type", mime)
                .body(axum::body::Body::from(f.contents().to_vec()))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

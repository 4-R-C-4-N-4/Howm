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
mod gossip;
mod peers;
mod state;

static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");

#[derive(Parser, Debug)]
#[command(name = "presence", about = "Howm peer presence capability")]
struct Config {
    #[arg(long, default_value = "7004", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: std::path::PathBuf,

    /// Port the Howm daemon HTTP API listens on.
    #[arg(long, default_value = "7000", env = "HOWM_DAEMON_PORT")]
    daemon_port: u16,

    /// Base URL for the Howm daemon.
    #[arg(long, default_value = "http://127.0.0.1:7000", env = "HOWM_DAEMON_URL")]
    daemon_url: String,

    /// Seconds without a UI heartbeat before flipping to "away".
    #[arg(long, default_value = "300", env = "PRESENCE_IDLE_TIMEOUT")]
    idle_timeout: u64,

    /// Seconds between background gossip broadcasts to peers.
    #[arg(long, default_value = "60", env = "PRESENCE_BROADCAST_INTERVAL")]
    broadcast_interval: u64,

    /// Seconds without a gossip broadcast before marking a peer offline.
    #[arg(long, default_value = "180", env = "PRESENCE_OFFLINE_TIMEOUT")]
    offline_timeout: u64,

    /// UDP port for presence gossip protocol.
    #[arg(long, default_value = "7104", env = "PRESENCE_GOSSIP_PORT")]
    gossip_port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let bridge = p2pcd::bridge_client::BridgeClient::new(config.daemon_port);

    let app_state = state::AppState::new(
        bridge,
        config.idle_timeout,
        config.broadcast_interval,
        config.offline_timeout,
        config.gossip_port,
    );

    // Restore active peers from daemon on startup
    {
        let s = app_state.clone();
        tokio::spawn(async move {
            api::init_peers_from_daemon(s).await;
        });
    }

    // Idle timeout background task — checks every 10s
    {
        let s = app_state.clone();
        let idle_timeout = config.idle_timeout;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                let now = state::now_secs();
                let last_hb = *s.last_heartbeat.read().await;
                if now.saturating_sub(last_hb) > idle_timeout {
                    let mut presence = s.presence.write().await;
                    if presence.activity == state::Activity::Active {
                        presence.activity = state::Activity::Away;
                        presence.updated_at = now;
                        info!("Activity flipped to away (idle timeout)");
                    }
                }
            }
        });
    }

    // Start gossip sender and receiver
    gossip::start_gossip_sender(app_state.clone());
    gossip::start_gossip_receiver(app_state.clone());

    let app = Router::new()
        // Presence API
        .route("/heartbeat", post(api::heartbeat))
        .route("/status", get(api::get_status).put(api::set_status))
        .route("/peers", get(api::list_peers))
        .route("/peers/{peer_id}", get(api::get_peer))
        // Embedded UI
        .route("/ui", get(serve_ui_index))
        .route("/ui/", get(serve_ui_index))
        .route("/ui/{*path}", get(serve_ui_asset))
        // Health
        .route("/health", get(api::health))
        // P2P-CD lifecycle hooks (called by daemon cap_notify)
        .route("/p2pcd/peer-active", post(api::peer_active))
        .route("/p2pcd/peer-inactive", post(api::peer_inactive))
        .route("/p2pcd/inbound", post(api::inbound_message))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    info!("Presence capability listening on {}", addr);
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

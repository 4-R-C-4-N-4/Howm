use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod db;

#[derive(Parser, Debug)]
#[command(name = "messaging", about = "Howm peer messaging capability")]
struct Config {
    #[arg(long, default_value = "7002", env = "PORT")]
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

    let msg_db = db::MessageDb::open(&config.data_dir)?;
    let bridge = p2pcd::bridge_client::BridgeClient::new(config.daemon_port);

    let state = api::AppState::new(msg_db, bridge, config.daemon_port);

    // Restore active peers from daemon on startup
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            api::init_peers_from_daemon(state_clone).await;
        });
    }

    let app = Router::new()
        // Messaging API
        .route("/send", post(api::send_message))
        .route("/conversations", get(api::list_conversations))
        .route("/conversations/{peer_id}", get(api::get_conversation))
        .route("/conversations/{peer_id}/read", post(api::mark_read))
        .route(
            "/conversations/{peer_id}/messages/{msg_id}",
            delete(api::delete_message),
        )
        // Health
        .route("/health", get(api::health))
        // P2P-CD lifecycle hooks (called by daemon cap_notify)
        .route("/p2pcd/peer-active", post(api::peer_active))
        .route("/p2pcd/peer-inactive", post(api::peer_inactive))
        .route("/p2pcd/inbound", post(api::inbound_message))
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576)); // 1 MB for text messages

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!("Messaging capability listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

use axum::{
    extract::{DefaultBodyLimit, Path as AxumPath},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod db;
mod notifier;

static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");

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

    /// Base URL for the Howm daemon (used for push notifications).
    #[arg(long, default_value = "http://127.0.0.1:7000", env = "HOWM_DAEMON_URL")]
    daemon_url: String,
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

    // Fetch local peer ID once at startup (with retry)
    let local_peer_id = {
        let delays = [0u64, 150, 500, 1000, 2000];
        let mut id = String::new();
        for delay in &delays {
            if *delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(*delay)).await;
            }
            match bridge.get_local_peer_id().await {
                Ok(pid) => {
                    id = pid;
                    break;
                }
                Err(_) => continue,
            }
        }
        if id.is_empty() {
            tracing::warn!(
                "messaging: could not fetch local peer ID from daemon; \
                 inbound messages will be rejected until daemon is reachable"
            );
        }
        std::sync::Arc::new(id)
    };

    // Start SSE stream — no hooks needed for messaging (Type 1 pure presence)
    let stream = std::sync::Arc::new(p2pcd::capability_sdk::PeerStream::connect(
        "howm.social.messaging.1",
        config.daemon_port,
    ));

    let http_client = reqwest::Client::new();
    let msg_db_arc = std::sync::Arc::new(msg_db);
    let daemon_notifier =
        notifier::DaemonNotifier::new(http_client, &config.daemon_url, msg_db_arc.clone());

    let state = api::AppState::new_with_notifier(
        msg_db_arc,
        bridge,
        config.daemon_port,
        daemon_notifier,
        stream,
        local_peer_id,
    );

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
        // Embedded UI
        .route("/ui", get(serve_ui_index))
        .route("/ui/", get(serve_ui_index))
        .route("/ui/{*path}", get(serve_ui_asset))
        // Health
        .route("/health", get(api::health))
        // P2P-CD inbound message forwarding (lifecycle hooks removed — using PeerStream)
        .route("/p2pcd/inbound", post(api::inbound_message))
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576)); // 1 MB for text messages

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    info!("Messaging capability listening on {}", addr);
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
    // Treat empty path (/ui/) as index.html
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

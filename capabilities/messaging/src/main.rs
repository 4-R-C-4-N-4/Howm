use axum::routing::{delete, get, post};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::path::PathBuf;
use std::sync::Arc;

use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{init_tracing, CapabilityApp, LocalPeerId, PeerStream};

mod api;
mod db;
mod notifier;

static UI_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui");

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
    init_tracing();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let msg_db = Arc::new(db::MessageDb::open(&config.data_dir)?);
    let bridge = BridgeClient::new(config.daemon_port);

    // Fetch local peer ID once at startup with retry; lazy re-fetch on demand.
    let local_peer_id = LocalPeerId::lazy(bridge.clone()).await;

    // Type-1 PeerStream: pure presence tracking, no hooks.
    let stream = Arc::new(PeerStream::connect(
        "howm.social.messaging.1",
        config.daemon_port,
    ));

    let daemon_notifier = notifier::DaemonNotifier::new(
        reqwest::Client::new(),
        &config.daemon_url,
        msg_db.clone(),
    );

    let state = api::AppState::new_with_notifier(
        msg_db,
        bridge,
        config.daemon_port,
        daemon_notifier,
        stream,
        local_peer_id,
    );

    CapabilityApp::new("howm.social.messaging.1", config.port, state)
        .with_body_limit(1_048_576) // 1 MiB for text messages
        .with_ui(&UI_DIR)
        .with_inbound_handler(api::inbound_message)
        .with_routes(|router| {
            router
                .route("/send", post(api::send_message))
                .route("/conversations", get(api::list_conversations))
                .route("/conversations/{peer_id}", get(api::get_conversation))
                .route("/conversations/{peer_id}/read", post(api::mark_read))
                .route(
                    "/conversations/{peer_id}/messages/{msg_id}",
                    delete(api::delete_message),
                )
        })
        .run()
        .await
}

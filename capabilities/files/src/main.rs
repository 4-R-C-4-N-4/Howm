use axum::routing::{get, patch, post, put};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{init_tracing, CapabilityApp, HookFn, PeerStream};

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
    init_tracing();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let files_db = db::FilesDb::open(&config.data_dir)?;
    let bridge = BridgeClient::new(config.daemon_port);

    // Group cache — populated by the on_active hook for each peer.
    let peer_groups = Arc::new(tokio::sync::RwLock::new(
        std::collections::HashMap::<String, Vec<db::PeerGroup>>::new(),
    ));

    // Type-2 PeerStream: on_active fetches ACL group memberships per peer.
    let on_active: HookFn = {
        let groups = Arc::clone(&peer_groups);
        let bridge_for_hook = bridge.clone();
        Arc::new(move |peer_id: String| {
            let groups = Arc::clone(&groups);
            let bridge = bridge_for_hook.clone();
            Box::pin(async move {
                let fetched = api::fetch_peer_groups_by_id(&bridge, &peer_id).await;
                info!(
                    "on_active hook: cached {} groups for peer {}",
                    fetched.len(),
                    &peer_id[..8.min(peer_id.len())]
                );
                groups.write().await.insert(peer_id, fetched);
            })
        })
    };

    let stream = Arc::new(PeerStream::connect_with_hooks(
        "howm.social.files.1",
        config.daemon_port,
        Some(on_active),
        None,
    ));

    let state = api::AppState::new(
        files_db,
        bridge,
        config.daemon_port,
        config.port,
        config.data_dir.clone(),
        stream,
        peer_groups,
    );

    CapabilityApp::new("howm.social.files.1", config.port, state)
        // Files supports large multipart uploads — 500 MiB hard cap.
        .with_body_limit(500 * 1024 * 1024)
        .with_ui(&UI_ASSETS)
        .with_inbound_handler(api::inbound_message)
        .with_routes(|router| {
            router
                .route("/peers", get(api::list_active_peers))
                // Operator offerings API
                .route(
                    "/offerings",
                    get(api::list_offerings).post(api::create_offering),
                )
                .route("/offerings/json", put(api::create_offering_json))
                .route(
                    "/offerings/{offering_id}",
                    patch(api::update_offering).delete(api::delete_offering),
                )
                // Peer catalogue browsing
                .route("/peer/{peer_id}/catalogue", get(api::peer_catalogue))
                // Downloads
                .route(
                    "/downloads",
                    get(api::list_downloads).post(api::initiate_download),
                )
                .route("/downloads/{blob_id}/status", get(api::download_status))
                .route("/downloads/{blob_id}/data", get(api::download_data))
                // Internal: transfer-complete callback from daemon bridge
                .route("/internal/transfer-complete", post(api::transfer_complete))
        })
        .run()
        .await
}

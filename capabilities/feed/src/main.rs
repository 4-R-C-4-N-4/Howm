use axum::routing::{delete, get, post};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::path::PathBuf;

use p2pcd::capability_sdk::{init_tracing, CapabilityApp};

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
    init_tracing();

    let config = Config::parse();

    // Open SQLite database and run JSON migration if needed. feed keeps its
    // own DB opener because it layers a one-time JSON-to-SQLite migration on
    // top of the standard open; the SDK's `cap_db::open_sqlite` handles the
    // pragmas but not that legacy import step.
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

    // Start SSE stream — PeerStream keeps the tracker current via daemon events.
    state.runtime.start_event_stream();

    // Resume any pending blob transfers from a previous run.
    let resume_fut = {
        let db_clone = state.db.clone();
        let bridge_clone = state.bridge().clone();
        async move {
            blob_fetcher::resume_active_transfers(db_clone, bridge_clone).await;
        }
    };

    CapabilityApp::new("howm.social.feed.1", config.port, state)
        .with_body_limit(50 * 1024 * 1024) // 50 MiB for media uploads
        .with_ui(&UI_ASSETS)
        .with_inbound_handler(api::p2pcd_inbound)
        .with_routes(|router| {
            router
                // Feed endpoints (paginated via ?limit=N&offset=N)
                .route("/feed", get(api::get_feed))
                .route("/feed/mine", get(api::get_my_feed))
                .route("/feed/peer/{peer_id}", get(api::get_peer_feed))
                // Post CRUD — JSON for text-only, multipart for media attachments
                .route("/post", post(api::create_post))
                .route("/post/upload", post(api::create_post_multipart))
                .route("/post/limits", get(api::get_limits))
                .route("/post/{id}", delete(api::delete_post))
                .route("/post/{id}/attachments", get(api::get_attachment_status))
                // Blob serving (content-addressed media)
                .route("/blob/{hash}", get(api::serve_blob))
                // Peer list
                .route("/peers", get(api::list_peers))
        })
        .spawn_task(resume_fut)
        .run()
        .await
}

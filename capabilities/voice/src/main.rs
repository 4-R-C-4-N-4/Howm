use axum::routing::{get, post};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::sync::Arc;
use tracing::info;

use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{init_tracing, CapabilityApp, HookFn, PeerStream, PeerTracker};

mod api;
mod bridge;
mod notifier;
mod signal;
mod state;

use notifier::VoiceNotifier;
use signal::SignalHub;
use state::{RoomStore, VoiceConfig};

static UI_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui");

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

    /// Base URL for the Howm daemon (used for push notifications).
    #[arg(long, default_value = "http://127.0.0.1:7000", env = "HOWM_DAEMON_URL")]
    daemon_url: String,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub rooms: RoomStore,
    pub signal_hub: SignalHub,
    pub bridge: BridgeClient,
    pub notifier: VoiceNotifier,
    pub tracker: PeerTracker,
    #[allow(dead_code)]
    pub daemon_port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let voice_config = VoiceConfig::from_env();
    info!(
        "Voice config: max_room_size={}, room_timeout={}s, invite_timeout={}s",
        voice_config.max_room_size,
        voice_config.room_timeout_secs,
        voice_config.invite_timeout_secs
    );

    let bridge = BridgeClient::new(config.daemon_port);
    let notifier = VoiceNotifier::new(reqwest::Client::new(), &config.daemon_url);

    // Build the tracker BEFORE constructing state so we can share it with
    // PeerStream and the on_inactive hook while also exposing it in AppState
    // for the /peers endpoint.
    let tracker = PeerTracker::new("howm.social.voice.1");
    let hook_tracker = tracker.clone();

    let state = AppState {
        rooms: RoomStore::new(voice_config),
        signal_hub: SignalHub::new(),
        bridge,
        notifier,
        tracker: tracker.clone(),
        daemon_port: config.daemon_port,
    };

    // ── Type-3 PeerStream: pre-built tracker shared with on_inactive guard ──

    let on_inactive: HookFn = {
        let state_for_hook = state.clone();
        Arc::new(move |peer_id: String| {
            let state = state_for_hook.clone();
            let tracker = hook_tracker.clone();
            Box::pin(async move {
                // Generation guard: peer flapped and reconnected before this
                // hook ran — skip teardown.
                if tracker.find_peer(&peer_id).await.is_some() {
                    tracing::debug!(
                        "voice: skipping teardown for {} — peer already reconnected",
                        &peer_id[..8.min(peer_id.len())]
                    );
                    return;
                }

                info!(
                    "voice: peer {} went offline, removing from rooms",
                    &peer_id[..8.min(peer_id.len())]
                );

                let rooms_affected = state.rooms.remove_peer_from_all(&peer_id);
                for (room_id, destroyed) in &rooms_affected {
                    if *destroyed {
                        info!("Room {} destroyed (last member went offline)", room_id);
                        state.signal_hub.close_room(room_id);
                    } else {
                        let msg = serde_json::to_string(&signal::SignalMessage {
                            msg_type: "peer-left".to_string(),
                            peer_id: Some(peer_id.clone()),
                            ..Default::default()
                        })
                        .unwrap_or_default();
                        state.signal_hub.broadcast_all(room_id, &msg);
                    }
                }
            })
        })
    };

    // PeerStream's SSE loop is detached internally — handle isn't needed
    // beyond construction.
    let _stream = PeerStream::drive_existing(
        tracker,
        format!(
            "http://127.0.0.1:{}/p2pcd/bridge/events?capability=howm.social.voice.1",
            config.daemon_port
        ),
        None, // no on_active hook needed
        Some(on_inactive),
    );

    // Background: room + invite cleanup loop (every 60s).
    let cleanup_fut = {
        let cleanup_state = state.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let removed = cleanup_state.rooms.cleanup_stale_rooms();
                if removed > 0 {
                    info!("Cleaned up {} stale room(s)", removed);
                }
                let expired = cleanup_state.rooms.cleanup_expired_invites();
                if expired > 0 {
                    info!("Expired {} stale invite(s)", expired);
                }
            }
        }
    };

    CapabilityApp::new("howm.social.voice.1", config.port, state)
        .with_ui(&UI_DIR)
        .with_inbound_handler(bridge::inbound_message)
        .with_routes(|router| {
            router
                .route("/rooms", post(api::create_room).get(api::list_rooms))
                .route(
                    "/rooms/{room_id}",
                    get(api::get_room).delete(api::close_room),
                )
                .route("/rooms/{room_id}/join", post(api::join_room))
                .route("/rooms/{room_id}/leave", post(api::leave_room))
                .route("/rooms/{room_id}/invite", post(api::invite_peers))
                .route("/rooms/{room_id}/mute", post(api::mute))
                .route("/quick-call", post(api::quick_call))
                .route("/peers", get(api::list_peers))
                .route("/rooms/{room_id}/signal", get(signal::signal_ws))
        })
        .spawn_task(cleanup_fut)
        .run()
        .await
}

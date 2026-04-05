use axum::{
    extract::Path as AxumPath,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use clap::Parser;
use include_dir::{include_dir, Dir};
use p2pcd::capability_sdk::{PeerStream, PeerTracker};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod bridge;
mod notifier;
mod signal;
mod state;

use notifier::VoiceNotifier;
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

    /// Base URL for the Howm daemon (used for push notifications).
    #[arg(long, default_value = "http://127.0.0.1:7000", env = "HOWM_DAEMON_URL")]
    daemon_url: String,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub rooms: RoomStore,
    pub signal_hub: SignalHub,
    pub bridge: p2pcd::bridge_client::BridgeClient,
    pub notifier: VoiceNotifier,
    #[allow(dead_code)]
    pub daemon_port: u16,
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

    let bridge = p2pcd::bridge_client::BridgeClient::new(config.daemon_port);
    let http_client = reqwest::Client::new();
    let notifier = VoiceNotifier::new(http_client, &config.daemon_url);

    let state = AppState {
        rooms: RoomStore::new(voice_config),
        signal_hub: SignalHub::new(),
        bridge,
        notifier,
        daemon_port: config.daemon_port,
    };

    // Background: room + invite cleanup loop
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
                let expired = cleanup_state.rooms.cleanup_expired_invites();
                if expired > 0 {
                    info!("Expired {} stale invite(s)", expired);
                }
            }
        });
    }

    // PeerStream: subscribe to daemon SSE peer events.
    //
    // Voice is Hook Type 3+4:
    //   on_active  = None  (no reaction needed when a peer comes online)
    //   on_inactive = generation-guarded room teardown
    //
    // Generation guard: the on_inactive hook fires AFTER the PeerStream's
    // SSE consumer has called tracker.on_peer_inactive (which removes the peer
    // from the tracker). If the peer reconnected immediately (session flap),
    // a peer-active event will have re-inserted them before the spawned hook
    // runs. Checking find_peer() at hook time catches this and skips teardown.
    // _stream must live at function scope so the SSE task stays alive for axum::serve.
    // Build the tracker first so the hook can capture a clone of it.
    let tracker = PeerTracker::new("howm.social.voice.1");
    let hook_tracker = tracker.clone(); // shares the inner Arc<RwLock<_>>

    let state_for_hook = state.clone();
    let on_inactive: p2pcd::capability_sdk::HookFn = Arc::new(move |peer_id: String| {
        let state = state_for_hook.clone();
        let tracker = hook_tracker.clone();
        Box::pin(async move {
                // Generation guard: if the peer flapped and came back before
                // this hook ran, they will already be re-present in the tracker.
                // Skip teardown to avoid destroying a room they're still in.
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
                        let msg = serde_json::to_string(&crate::signal::SignalMessage {
                            msg_type: "peer-left".to_string(),
                            peer_id: Some(peer_id.clone()),
                            ..Default::default()
                        })
                        .unwrap_or_default();
                        state.signal_hub.broadcast_all(room_id, &msg);
                    }
                }
            })
    });

    // Keep the stream alive for the process lifetime.
    // drive_existing: the hook captures the same tracker the SSE loop drives,
    // so find_peer() sees live updates from the stream.
    let _stream = PeerStream::drive_existing(
        tracker,
        format!(
            "http://127.0.0.1:{}/p2pcd/bridge/events?capability=howm.social.voice.1",
            config.daemon_port
        ),
        None,            // no on_active hook needed
        Some(on_inactive),
    );

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
        // Quick call
        .route("/quick-call", post(api::quick_call))
        // WebSocket signaling
        .route("/rooms/{room_id}/signal", get(signal::signal_ws))
        // P2P-CD inbound messages (lifecycle hooks now handled via PeerStream SSE)
        .route("/p2pcd/inbound", post(bridge::inbound_message))
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

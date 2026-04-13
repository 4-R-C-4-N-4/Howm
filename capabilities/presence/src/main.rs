use axum::routing::{get, post};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::sync::Arc;

use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{init_tracing, CapabilityApp, HookFn, PeerStream, PeerTracker};
use tracing::info;

mod api;
mod gossip;
mod peers;
mod state;

static UI_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui");

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
    init_tracing();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let bridge = BridgeClient::new(config.daemon_port);

    let app_state = state::AppState::new(
        bridge,
        config.idle_timeout,
        config.broadcast_interval,
        config.offline_timeout,
        config.gossip_port,
    );

    // ── Type-3 PeerStream: pre-built tracker so hooks can look up wg_address
    //    for the gossip sender/receiver address map ──
    let tracker = PeerTracker::new("howm.social.presence.1");

    let on_active: HookFn = {
        let peers_map = Arc::clone(&app_state.peers);
        let addr_map = Arc::clone(&app_state.peer_addresses);
        let hook_tracker = tracker.clone();
        let hook_bridge = app_state.bridge.clone();
        Arc::new(move |peer_id: String| {
            let peers = Arc::clone(&peers_map);
            let addrs = Arc::clone(&addr_map);
            let tracker = hook_tracker.clone();
            let bridge = hook_bridge.clone();
            Box::pin(async move {
                let now = state::now_secs();
                let mut peers_guard = peers.write().await;
                peers_guard
                    .entry(peer_id.clone())
                    .and_modify(|p| {
                        p.activity = state::Activity::Active;
                        p.updated_at = now;
                    })
                    .or_insert_with(|| peers::PeerPresence {
                        peer_id: peer_id.clone(),
                        activity: state::Activity::Active,
                        status: None,
                        emoji: None,
                        updated_at: now,
                        last_broadcast_received: now,
                    });
                // Populate wg_address for gossip. Try tracker first (live
                // peer-active events include it), fall back to bridge query
                // (needed for SSE snapshot where wg_address is null).
                let mut resolved_addr = String::new();
                if let Some(active_peer) = tracker.find_peer(&peer_id).await {
                    if !active_peer.wg_address.is_empty() {
                        resolved_addr = active_peer.wg_address;
                    }
                }
                if resolved_addr.is_empty() {
                    // Fallback: ask the bridge for the peer's WG address
                    if let Ok(peer_list) = bridge.list_peers(Some("howm.social.presence.1")).await {
                        if let Some(p) = peer_list.iter().find(|p| p.peer_id == peer_id) {
                            if let Some(ref addr) = p.wg_address {
                                resolved_addr = addr.clone();
                            }
                        }
                    }
                }
                if !resolved_addr.is_empty() {
                    info!(
                        "PeerStream: peer active {} wg_addr={}",
                        &peer_id[..8.min(peer_id.len())],
                        resolved_addr,
                    );
                    addrs.write().await.insert(peer_id.clone(), resolved_addr);
                } else {
                    info!(
                        "PeerStream: peer active {} (no wg_address resolved)",
                        &peer_id[..8.min(peer_id.len())],
                    );
                }
            })
        })
    };

    let on_inactive: HookFn = {
        let peers_map = Arc::clone(&app_state.peers);
        let addr_map = Arc::clone(&app_state.peer_addresses);
        Arc::new(move |peer_id: String| {
            let peers = Arc::clone(&peers_map);
            let addrs = Arc::clone(&addr_map);
            Box::pin(async move {
                addrs.write().await.remove(&peer_id);
                let now = state::now_secs();
                let mut peers_guard = peers.write().await;
                if let Some(p) = peers_guard.get_mut(&peer_id) {
                    p.activity = state::Activity::Away;
                    p.updated_at = now;
                }
                info!("PeerStream: peer inactive {}", peer_id);
            })
        })
    };

    // Drive the pre-built tracker with PeerStream SSE (Type-3 pattern).
    let _stream = PeerStream::drive_existing(
        tracker,
        format!(
            "http://127.0.0.1:{}/p2pcd/bridge/events?capability=howm.social.presence.1",
            config.daemon_port
        ),
        Some(on_active),
        Some(on_inactive),
    );

    // Gossip sender + receiver each spawn their own background tasks internally.
    gossip::start_gossip_sender(app_state.clone());
    gossip::start_gossip_receiver(app_state.clone());

    // Idle-timeout watcher — flips Active to Away after `idle_timeout` seconds
    // without a /heartbeat ping.
    let idle_fut = {
        let s = app_state.clone();
        let idle_timeout = config.idle_timeout;
        async move {
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
        }
    };

    CapabilityApp::new("howm.social.presence.1", config.port, app_state)
        .with_ui(&UI_DIR)
        .with_inbound_handler(api::inbound_message)
        .with_routes(|router| {
            router
                .route("/heartbeat", post(api::heartbeat))
                .route("/status", get(api::get_status).put(api::set_status))
                .route("/peers", get(api::list_peers))
                .route("/peers/{peer_id}", get(api::get_peer))
        })
        .spawn_task(idle_fut)
        .run()
        .await
}

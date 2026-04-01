use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use howm::api;
use howm::capabilities;
use howm::config;
use howm::executor;
use howm::identity;
use howm::lan_discovery;
use howm::matchmake;
use howm::net_detect;
use howm::p2pcd;
use howm::peers;
use howm::state;
use howm::wireguard;

use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    // Set up logging: file-based with optional stdout in debug mode
    let log_dir = config.data_dir.join("logs");
    std::fs::create_dir_all(&log_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&log_dir, std::fs::Permissions::from_mode(0o700));
    }
    let file_appender = tracing_appender::rolling::daily(&log_dir, "howm.log");
    let env_filter = EnvFilter::from_default_env().add_directive("info".parse()?);

    if config.debug {
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(std::io::stdout))
            .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
            .init();
        // Leak guard so it lives for the program duration
        std::mem::forget(_guard);
    } else {
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
            .init();
        std::mem::forget(_guard);
    }

    // Load or create identity
    let mut identity = identity::load_or_create(&config.data_dir, config.name.clone())?;
    info!("Node identity: {} ({})", identity.name, identity.node_id);

    // Detect IPv6 GUAs before WG init (informs endpoint selection)
    let ipv6_guas = net_detect::detect_ipv6_guas();

    // Find available WG port (falls back through range if preferred is busy)
    let actual_wg_port = net_detect::find_available_wg_port(config.wg_port);

    // Init WireGuard
    let wg_config = wireguard::WgConfig {
        enabled: config.wg_enabled(),
        port: actual_wg_port,
        endpoint: config.wg_endpoint.clone(),
        address: config.wg_address.clone(),
        data_dir: config.data_dir.clone(),
        node_id: identity.node_id.clone(),
    };

    let wg_state = wireguard::init(&wg_config).await?;

    if let Some(ref pubkey) = wg_state.public_key {
        identity.wg_pubkey = Some(pubkey.clone());
        identity.wg_address = wg_state.address.clone();
        identity.wg_endpoint = wg_state.endpoint.clone();
        identity.ipv6_guas = ipv6_guas.iter().map(|a| a.to_string()).collect();
        identity.wg_listen_port = Some(actual_wg_port);
        identity::write_identity(&config.data_dir, &identity)?;
        info!(
            "WG address: {}, listen port: {}",
            wg_state.address.as_deref().unwrap_or("none"),
            actual_wg_port,
        );
        if !ipv6_guas.is_empty() {
            info!("IPv6 GUAs available: {}", ipv6_guas.len());
        }
    }

    // Load persisted state
    let peers = peers::load(&config.data_dir)?;
    let mut capabilities = capabilities::load(&config.data_dir)?;

    // Restart capability processes that were running before daemon shutdown
    for cap in capabilities.iter_mut() {
        if matches!(cap.status, capabilities::CapStatus::Stopped) {
            continue;
        }
        // Check if the process is still alive
        let alive = cap.pid.map(executor::check_health).unwrap_or(false);
        if alive {
            info!(
                "Capability '{}' process still running (pid={:?})",
                cap.name, cap.pid
            );
            continue;
        }
        // Process is dead — restart from the binary
        info!(
            "Restarting capability '{}' from {}",
            cap.name, cap.binary_path
        );
        let data_dir = &cap.data_dir;
        std::fs::create_dir_all(data_dir)?;
        match executor::start_capability(
            &cap.binary_path,
            &cap.name,
            cap.port,
            data_dir,
            std::collections::HashMap::new(),
        )
        .await
        {
            Ok(new_pid) => {
                info!(
                    "Capability '{}' restarted on port {} (pid={})",
                    cap.name, cap.port, new_pid
                );
                cap.pid = Some(new_pid);
                cap.status = capabilities::CapStatus::Running;
            }
            Err(e) => {
                tracing::warn!("Failed to restart capability '{}': {}", cap.name, e);
                cap.status = capabilities::CapStatus::Error(format!("restart failed: {}", e));
                cap.pid = None;
            }
        }
    }
    capabilities::save(&config.data_dir, &capabilities)?;

    info!(
        "Loaded {} peers, {} capabilities",
        peers.len(),
        capabilities.len()
    );

    // Generate or load API bearer token (S2)
    let api_token = api::auth_layer::load_or_create_token(&config.data_dir)?;
    info!(
        "API bearer token loaded ({}…)",
        &api_token[..8.min(api_token.len())]
    );

    // Initialise access control database (group-based permissions)
    let access_db = {
        let db_path = config.data_dir.join("access.db");
        let db = howm_access::AccessDb::open(&db_path)
            .map_err(|e| anyhow::anyhow!("failed to open access.db: {}", e))?;

        // One-time migration: map peers.json TrustLevel → access.db groups
        migrate_trust_levels(&db, &peers);

        Arc::new(db)
    };
    info!("Access control database initialised");

    // Build app state
    let mut state = state::AppState::new(
        identity.clone(),
        peers.clone(),
        capabilities,
        config.clone(),
        api_token,
        Arc::clone(&access_db),
    );

    // Build capability notifier and register running capabilities
    let cap_notifier = p2pcd::cap_notify::CapabilityNotifier::new();
    {
        let caps = state.capabilities.read().await;
        for cap in caps.iter() {
            if matches!(cap.status, capabilities::CapStatus::Running) {
                cap_notifier.register(cap.name.clone(), cap.port).await;
                tracing::debug!("cap_notify: registered '{}' on port {}", cap.name, cap.port);
            }
        }
    }

    // Construct P2P-CD protocol engine (only when WG is active)
    let p2pcd_engine = if wg_state.tunnel_handle.is_some() {
        match build_p2pcd_engine(
            &config,
            &identity,
            &wg_state,
            Arc::clone(&cap_notifier),
            Arc::clone(&access_db),
        ) {
            Ok(engine) => {
                info!("P2P-CD engine initialised");
                Some(engine)
            }
            Err(e) => {
                tracing::warn!("P2P-CD engine init failed (continuing without): {}", e);
                None
            }
        }
    } else {
        info!("WG disabled — P2P-CD engine not started");
        None
    };
    state.p2pcd_engine = p2pcd_engine.clone();

    // Restore LAN transport hints from persisted peers so that P2P-CD can
    // reach LAN peers directly after a daemon restart without waiting for a
    // new scan or invite.  `lan_transport_hints` is in-memory only, so we
    // re-populate it here from the `lan_ip` field stored in peers.json.
    if let Some(ref engine) = p2pcd_engine {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let mut restored = 0u32;
        for peer in &peers {
            if let Some(ref lan_ip) = peer.lan_ip {
                if let Ok(bytes) = STANDARD.decode(&peer.wg_pubkey) {
                    if bytes.len() == 32 {
                        let mut peer_id = [0u8; 32];
                        peer_id.copy_from_slice(&bytes);
                        if let Ok(ip) = lan_ip.parse::<std::net::IpAddr>() {
                            let addr = std::net::SocketAddr::new(ip, 7654);
                            engine.set_lan_hint(peer_id, addr).await;
                            restored += 1;
                        }
                    }
                }
            }
        }
        if restored > 0 {
            info!(
                "Restored {} LAN transport hint(s) from peers.json",
                restored
            );
        }
    }

    // Store WG active state
    {
        let mut wg_active = state.wg_active.write().await;
        *wg_active = wg_state.tunnel_handle.is_some();
    }

    // Start LAN mDNS discovery if enabled
    if config.lan_discoverable {
        if let Some(ref pubkey) = identity.wg_pubkey {
            if let Some(lan_ip) = net_detect::detect_lan_ip() {
                let wg_port = identity.wg_listen_port.unwrap_or(config.wg_port);
                match lan_discovery::LanDiscovery::start(
                    &identity.name,
                    pubkey,
                    &lan_ip,
                    config.port,
                    wg_port,
                ) {
                    Ok(discovery) => {
                        *state.lan_discovery.write().await = Some(discovery);
                        info!("LAN discovery active on {}", lan_ip);
                    }
                    Err(e) => {
                        tracing::warn!("LAN discovery failed to start: {}", e);
                    }
                }
            } else {
                info!("LAN discovery: no LAN IP detected — skipping mDNS registration");
            }
        }
    } else {
        info!("LAN discovery disabled (lan_discoverable=false)");
    }

    // Build Axum router
    let router = api::build_router(state.clone(), config.ui_dir.clone());

    // Background: P2P-CD engine
    if let Some(ref engine) = p2pcd_engine {
        let engine_arc = Arc::clone(engine);
        tokio::spawn(async move {
            if let Err(e) = engine_arc.run().await {
                tracing::error!("P2P-CD engine exited with error: {}", e);
            }
        });
    }

    // Register matchmake circuit event handler
    if let Some(ref engine) = p2pcd_engine {
        if let Some(handler) = engine.cap_router().handler_by_name("core.network.relay.1") {
            if let Some(relay_handler) = handler
                .as_any()
                .downcast_ref::<::p2pcd::capabilities::relay::RelayHandler>()
            {
                let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                relay_handler.set_event_callback(tx).await;
                let mm_state = state.clone();
                let mm_counter = Arc::clone(&state.matchmake_counter);
                tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        if let ::p2pcd::capabilities::relay::CircuitEvent::Data {
                            circuit_id,
                            data,
                            ..
                        } = event
                        {
                            match matchmake::decode_message(&data) {
                                Ok(matchmake::MatchmakeMessage::Request(req)) => {
                                    let s = mm_state.clone();
                                    let c = Arc::clone(&mm_counter);
                                    tokio::spawn(async move {
                                        if let Err(e) = matchmake::handle_incoming_matchmake(
                                            &s, circuit_id, req, c,
                                        )
                                        .await
                                        {
                                            tracing::warn!("matchmake handler error: {}", e);
                                        }
                                    });
                                }
                                Ok(_) => {
                                    tracing::debug!(
                                        "matchmake: ignoring non-request on circuit {}",
                                        circuit_id
                                    );
                                }
                                Err(_) => {
                                    // Not a matchmake message — ignore
                                }
                            }
                        }
                    }
                    tracing::debug!("matchmake: circuit event channel closed");
                });
                info!("Matchmake circuit event handler registered");
            }
        }
    }

    // Background: capability health check loop (every 30s)
    {
        let health_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                let mut caps = health_state.capabilities.write().await;
                let mut any_changed = false;
                for cap in caps.iter_mut() {
                    if !matches!(cap.status, capabilities::CapStatus::Running) {
                        continue;
                    }
                    let alive = cap.pid.map(executor::check_health).unwrap_or(false);
                    if !alive {
                        tracing::warn!(
                            "Capability '{}' (pid={:?}) crashed — restarting",
                            cap.name,
                            cap.pid
                        );
                        let data_dir = &cap.data_dir;
                        match executor::start_capability(
                            &cap.binary_path,
                            &cap.name,
                            cap.port,
                            data_dir,
                            std::collections::HashMap::new(),
                        )
                        .await
                        {
                            Ok(new_pid) => {
                                tracing::info!(
                                    "Capability '{}' restarted (pid={})",
                                    cap.name,
                                    new_pid
                                );
                                cap.pid = Some(new_pid);
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to restart capability '{}': {}",
                                    cap.name,
                                    e
                                );
                                cap.status = capabilities::CapStatus::Error(format!(
                                    "restart failed: {}",
                                    e
                                ));
                                cap.pid = None;
                            }
                        }
                        any_changed = true;
                    }
                }
                if any_changed {
                    let caps_clone = caps.clone();
                    drop(caps);
                    let _ = capabilities::save(&health_state.config.data_dir, &caps_clone);
                }
            }
        });
        info!("Capability health check loop started (30s interval)");
    }

    // Start HTTP server with graceful shutdown
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    info!("Starting Howm daemon on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(wait_for_shutdown_signal())
    .await?;

    // Server has stopped accepting connections — clean up
    info!("Shutdown signal received — cleaning up...");
    do_shutdown(&state).await;

    Ok(())
}

// ── Shutdown helpers ──────────────────────────────────────────────────────────

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => info!("Received SIGTERM"),
            _ = sigint.recv()  => info!("Received SIGINT"),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("Ctrl+C handler");
    }
}

async fn do_shutdown(state: &state::AppState) {
    // Shut down LAN mDNS discovery
    if let Some(discovery) = state.lan_discovery.write().await.take() {
        discovery.shutdown();
    }
    // Gracefully close all P2P-CD sessions
    if let Some(ref engine) = state.p2pcd_engine {
        engine.shutdown().await;
    }
    // Stop all running capability processes
    let caps = state.capabilities.read().await.clone();
    for cap in &caps {
        if let Some(pid) = cap.pid {
            info!("Stopping capability process: {} (pid={})", cap.name, pid);
            let _ = executor::stop_capability(pid).await;
        }
    }
    // Shutdown WireGuard interface
    let wg_active = *state.wg_active.read().await;
    if wg_active {
        if let Err(e) = wireguard::shutdown().await {
            tracing::warn!("WG shutdown error: {}", e);
        }
    }
}

// ── P2P-CD engine builder ────────────────────────────────────────────────────

fn build_p2pcd_engine(
    config: &Config,
    identity: &identity::NodeIdentity,
    wg_state: &wireguard::WgState,
    notifier: Arc<p2pcd::cap_notify::CapabilityNotifier>,
    access_db: Arc<howm_access::AccessDb>,
) -> anyhow::Result<Arc<p2pcd::engine::ProtocolEngine>> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Derive PeerId from the WireGuard public key stored in identity
    let pubkey_b64 = identity
        .wg_pubkey
        .as_deref()
        .or(wg_state.public_key.as_deref())
        .ok_or_else(|| anyhow::anyhow!("No WireGuard public key available for P2P-CD engine"))?;

    let key_bytes = STANDARD
        .decode(pubkey_b64)
        .map_err(|e| anyhow::anyhow!("Failed to decode WG pubkey: {}", e))?;
    if key_bytes.len() != 32 {
        anyhow::bail!("WG public key is {} bytes, expected 32", key_bytes.len());
    }
    let mut peer_id = [0u8; 32];
    peer_id.copy_from_slice(&key_bytes);

    // Load or generate p2pcd-peer.toml
    let toml_path = config.data_dir.join("p2pcd-peer.toml");
    let peer_config = if toml_path.exists() {
        p2pcd_types::config::PeerConfig::load(&toml_path)
            .map_err(|e| anyhow::anyhow!("Failed to load p2pcd-peer.toml: {}", e))?
    } else {
        let default_cfg = p2pcd_types::config::PeerConfig::generate_default(&config.data_dir);
        // Write the default config for the user to inspect/modify
        if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
            let _ = std::fs::write(&toml_path, toml_str);
            info!(
                "Generated default p2pcd-peer.toml at {}",
                toml_path.display()
            );
        }
        default_cfg
    };

    Ok(Arc::new(p2pcd::engine::ProtocolEngine::new(
        peer_config,
        peer_id,
        notifier,
        config.data_dir.clone(),
        access_db,
    )))
}

/// One-time migration: map peers.json TrustLevel to access.db group memberships.
/// Runs only when the access database has no existing memberships (first startup).
fn migrate_trust_levels(db: &howm_access::AccessDb, peers: &[peers::Peer]) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use howm_access::{GROUP_DEFAULT, GROUP_FRIENDS};

    // Skip if there are already memberships (migration already ran)
    if peers.is_empty() {
        return;
    }

    // Check if any memberships exist
    let has_any = peers.iter().any(|p| {
        let peer_id = STANDARD.decode(&p.wg_pubkey).unwrap_or_default();
        db.peer_has_memberships(&peer_id).unwrap_or(false)
    });

    if has_any {
        return; // Already migrated
    }

    let mut migrated = 0u32;
    for peer in peers {
        let peer_id = match STANDARD.decode(&peer.wg_pubkey) {
            Ok(id) if id.len() == 32 => id,
            _ => {
                tracing::warn!(
                    "skipping migration for peer '{}': invalid WG pubkey",
                    peer.name
                );
                continue;
            }
        };

        let group = match peer.trust {
            peers::TrustLevel::Friend => GROUP_FRIENDS,
            peers::TrustLevel::Public | peers::TrustLevel::Restricted => GROUP_DEFAULT,
        };

        if let Err(e) = db.assign_peer_to_group(&peer_id, &group) {
            tracing::warn!("failed to migrate peer '{}' to access.db: {}", peer.name, e);
        } else {
            migrated += 1;
        }
    }

    if migrated > 0 {
        tracing::info!(
            "Migrated {} peers from peers.json trust levels to access.db groups",
            migrated
        );
    }
}

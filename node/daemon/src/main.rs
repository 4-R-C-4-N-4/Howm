use clap::Parser;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod capabilities;
mod p2pcd;
mod config;
mod discovery;
mod error;
mod executor;
mod health;
mod identity;
mod invite;
mod open_invite;
mod peers;
mod proxy;
mod prune;
mod state;
mod wireguard;

use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    // Set up logging: file-based with optional stdout in debug mode
    let log_dir = config.data_dir.join("logs");
    std::fs::create_dir_all(&log_dir)?;
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

    // Init WireGuard
    let wg_config = wireguard::WgConfig {
        enabled: config.wg_enabled(),
        port: config.wg_port,
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
        identity::write_identity(&config.data_dir, &identity)?;
        info!(
            "WG address: {}",
            wg_state.address.as_deref().unwrap_or("none")
        );
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
        let alive = cap
            .pid
            .map(|pid| executor::check_health(pid))
            .unwrap_or(false);
        if alive {
            info!("Capability '{}' process still running (pid={:?})", cap.name, cap.pid);
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
    let api_token=api::auth_layer::load_or_create_token(&config.data_dir)?;
    info!("API bearer token: {}", api_token);

    // Build app state
    let state = state::AppState::new(
        identity.clone(),
        peers,
        capabilities,
        config.clone(),
        api_token,
    );

    // Store WG active state
    {
        let mut wg_active = state.wg_active.write().await;
        *wg_active = wg_state.tunnel_handle.is_some();
    }

    // Build Axum router
    let router = api::build_router(state.clone(), config.ui_dir.clone());

    // Background: discovery loop
    let discovery_state = state.clone();
    tokio::spawn(async move {
        discovery::start_loop(discovery_state).await;
    });

    // Background: capability health check loop (S10)
    let health_state = state.clone();
    tokio::spawn(async move {
        health::start_loop(health_state).await;
    });

    // Background: prune stale public peers
    let prune_state = state.clone();
    tokio::spawn(async move {
        prune::start_loop(prune_state).await;
    });

    // Background: graceful shutdown handler (SIGTERM / SIGINT / Ctrl-C)
    let shutdown_app_state = state.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("Shutdown signal received — cleaning up...");
        do_shutdown(&shutdown_app_state).await;
        std::process::exit(0);
    });

    // Start HTTP server
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    info!("Starting Howm daemon on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

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

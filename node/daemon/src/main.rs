use clap::Parser;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod capabilities;
mod config;
mod discovery;
mod docker;
mod error;
mod health;
mod identity;
mod invite;
mod peers;
mod proxy;
mod state;
mod wireguard;

use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    // Load or create identity
    let mut identity = identity::load_or_create(&config.data_dir, config.name.clone())?;
    info!("Node identity: {} ({})", identity.name, identity.node_id);

    // Init WireGuard — manages howm-wg Docker container
    let wg_config = wireguard::WgConfig {
        enabled: config.wg_enabled,
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
        info!("WG address: {}", wg_state.address.as_deref().unwrap_or("none"));
    }

    // Load persisted state
    let peers = peers::load(&config.data_dir)?;
    let capabilities = capabilities::load(&config.data_dir)?;
    info!(
        "Loaded {} peers, {} capabilities",
        peers.len(),
        capabilities.len()
    );

    // Generate or load API bearer token (S2)
    let api_token = api::auth_layer::load_or_create_token(&config.data_dir)?;
    info!("API token: {}", api_token);

    // Build app state
    let state = state::AppState::new(identity.clone(), peers, capabilities, config.clone(), api_token);

    // Store WG container ID for graceful shutdown cleanup
    {
        let mut wg_id = state.wg_container_id.write().await;
        *wg_id = wg_state.container_id.clone();
    }

    // Build Axum routers
    let local_router = api::build_local_router(state.clone());

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

    // Background: graceful shutdown handler (SIGTERM / SIGINT / Ctrl-C)
    let shutdown_wg_id = wg_state.container_id.clone();
    let shutdown_app_state = state.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("Shutdown signal received — cleaning up...");
        do_shutdown(&shutdown_app_state, shutdown_wg_id.as_deref()).await;
        std::process::exit(0);
    });

    // Start local management listener (127.0.0.1 only)
    let local_addr: SocketAddr = format!("127.0.0.1:{}", config.port).parse()?;
    info!("Starting local management API on {}", local_addr);
    let local_listener = tokio::net::TcpListener::bind(local_addr).await?;

    // Start peer listener on WG address (if available)
    if let Some(ref wg_addr) = identity.wg_address {
        let peer_router = api::build_peer_router(state.clone());
        let peer_addr: SocketAddr = format!("{}:{}", wg_addr, config.port).parse()?;
        info!("Starting peer API on {}", peer_addr);
        tokio::spawn(async move {
            match tokio::net::TcpListener::bind(peer_addr).await {
                Ok(listener) => {
                    if let Err(e) = axum::serve(
                        listener,
                        peer_router.into_make_service_with_connect_info::<SocketAddr>(),
                    ).await {
                        tracing::error!("Peer listener error: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Could not bind peer listener on {} — WG interface may not be ready: {}",
                        peer_addr, e
                    );
                }
            }
        });
    }

    axum::serve(
        local_listener,
        local_router.into_make_service_with_connect_info::<SocketAddr>(),
    ).await?;

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

async fn do_shutdown(state: &state::AppState, wg_container_id: Option<&str>) {
    // Stop all running capability containers
    let caps = state.capabilities.read().await.clone();
    for cap in &caps {
        info!("Stopping capability container: {}", cap.name);
        let _ = docker::stop_capability(&cap.container_id).await;
    }

    // Stop WG container
    if let Some(id) = wg_container_id {
        if let Err(e) = wireguard::shutdown(id).await {
            tracing::warn!("WG shutdown error: {}", e);
        }
    }
}

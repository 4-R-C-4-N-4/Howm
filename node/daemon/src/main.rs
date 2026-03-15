use clap::Parser;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod auth;
mod capabilities;
mod config;
mod discovery;
mod docker;
mod error;
mod identity;
mod invite;
mod peers;
mod proxy;
mod state;
mod tailnet;

use config::Config;
use tailnet::TailnetConfig;

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

    // Init tailnet — manages howm-headscale + howm-tailscale Docker containers
    let tailnet_cfg = TailnetConfig {
        enabled: config.tailnet_enabled,
        coordination_url: config.coordination_url.clone(),
        authkey: config.tailscale_authkey.clone(),
        data_dir: config.data_dir.clone(),
        headscale_enabled: config.headscale,
        headscale_port: config.headscale_port,
    };

    let tailnet_state = tailnet::init(tailnet_cfg).await?;

    if let Some(ref ip) = tailnet_state.ip {
        identity.tailnet_ip = Some(ip.clone());
        identity.tailnet_name = tailnet_state.name.clone();
        identity::write_identity(&config.data_dir, &identity)?;
        info!("Tailnet IP: {}", ip);
    }

    // Load persisted state
    let peers = peers::load(&config.data_dir)?;
    let capabilities = capabilities::load(&config.data_dir)?;
    info!(
        "Loaded {} peers, {} capabilities",
        peers.len(),
        capabilities.len()
    );

    // Build app state
    let state = state::AppState::new(identity, peers, capabilities, config.clone());

    // Store tailnet container IDs for graceful shutdown cleanup
    {
        let mut tc = state.tailnet_containers.write().await;
        *tc = (
            tailnet_state.headscale_container_id.clone(),
            tailnet_state.tailscale_container_id.clone(),
        );
    }

    // Build Axum router
    let router = api::build_router(state.clone());

    // Background: discovery loop
    let discovery_state = state.clone();
    tokio::spawn(async move {
        discovery::start_loop(discovery_state).await;
    });

    // Background: graceful shutdown handler (SIGTERM / SIGINT / Ctrl-C)
    let shutdown_ts = tailnet::TailnetState {
        ip: tailnet_state.ip.clone(),
        name: tailnet_state.name.clone(),
        status: tailnet_state.status.clone(),
        headscale_container_id: tailnet_state.headscale_container_id.clone(),
        tailscale_container_id: tailnet_state.tailscale_container_id.clone(),
    };
    let shutdown_app_state = state.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("Shutdown signal received — cleaning up...");
        do_shutdown(&shutdown_app_state, &shutdown_ts).await;
        std::process::exit(0);
    });

    // Start HTTP server
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    info!("Starting Howm daemon on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

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

async fn do_shutdown(state: &state::AppState, tailnet: &tailnet::TailnetState) {
    // Stop all running capability containers
    let caps = state.capabilities.read().await.clone();
    for cap in &caps {
        info!("Stopping capability container: {}", cap.name);
        let _ = docker::stop_capability(&cap.container_id).await;
    }

    // Stop tailnet containers (howm-tailscale, howm-headscale)
    if let Err(e) = tailnet::shutdown(tailnet).await {
        tracing::warn!("Tailnet shutdown error: {}", e);
    }
}

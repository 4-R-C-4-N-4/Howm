use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod posts;

#[derive(Parser, Debug)]
#[command(name = "social-feed", about = "Howm social feed capability")]
struct Config {
    #[arg(long, default_value = "7001", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();

    // Ensure data directory exists
    std::fs::create_dir_all(&config.data_dir)?;

    let state = api::FeedState {
        data_dir: config.data_dir.clone(),
    };

    let app = Router::new()
        .route("/feed", get(api::get_feed))
        .route("/post", post(api::create_post))
        .route("/health", get(api::health))
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    info!("Social feed capability starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

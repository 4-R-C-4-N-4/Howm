#![allow(dead_code)]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

static UI_ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui");

mod api;
mod chain;
mod crypto;
mod db;

#[derive(Parser, Debug)]
#[command(name = "wallet", about = "Howm crypto wallet capability")]
struct Config {
    #[arg(long, default_value = "7006", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: PathBuf,

    /// Port the Howm daemon HTTP API listens on.
    #[arg(long, default_value = "7000", env = "HOWM_DAEMON_PORT")]
    daemon_port: u16,

    /// RPC URL for the EVM chain.
    #[arg(
        long,
        default_value = "https://sepolia.base.org",
        env = "CHAIN_RPC_URL"
    )]
    chain_rpc_url: String,

    /// Chain ID for the EVM chain.
    #[arg(long, default_value = "84532", env = "CHAIN_ID")]
    chain_id: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = Config::parse();
    std::fs::create_dir_all(&config.data_dir)?;

    let wallet_db = db::WalletDb::open(&config.data_dir)?;
    let secrets_db = db::SecretsDb::open(&config.data_dir)?;

    let backend: Arc<dyn chain::ChainBackend> = Arc::new(chain::evm::EvmBackend::new(
        config.chain_rpc_url.clone(),
        config.chain_id,
    ));

    let state = api::AppState {
        db: wallet_db,
        secrets: secrets_db,
        backend,
        rpc_url: config.chain_rpc_url,
        chain_id: config.chain_id,
    };

    // Background tasks
    let bg_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            // Expire old invoices
            if let Ok(n) = bg_state.db.expire_old_invoices(now) {
                if n > 0 {
                    tracing::debug!("Expired {} invoices", n);
                }
            }

            // Expire subscriptions
            if let Ok(n) = bg_state.db.expire_subscriptions(now) {
                if n > 0 {
                    tracing::debug!("Expired {} subscriptions", n);
                }
            }
        }
    });

    let app = Router::new()
        // Health
        .route("/health", get(api::health))
        // Wallets
        .route("/wallets", get(api::list_wallets).post(api::create_wallet))
        .route("/wallets/{id}", axum::routing::delete(api::delete_wallet))
        .route("/wallets/{id}/balance", get(api::get_balance))
        .route("/wallets/{id}/default", post(api::set_default))
        .route("/wallets/{id}/send", post(api::send_payment))
        // Transactions
        .route("/transactions", get(api::list_transactions))
        .route("/transactions/{id}", get(api::get_transaction))
        // Invoices
        .route(
            "/invoices",
            get(api::list_invoices).post(api::create_invoice),
        )
        .route("/invoices/{id}", get(api::get_invoice))
        .route("/invoices/{id}/check", post(api::check_invoice))
        // RPC (internal, capability-to-capability)
        .route("/rpc/create-invoice", post(api::rpc_create_invoice))
        .route("/rpc/verify-payment", post(api::rpc_verify_payment))
        // Subscriptions
        .route("/subscriptions", get(api::list_subscriptions))
        .route("/subscriptions/{id}/cancel", post(api::cancel_subscription))
        .route("/subscriptions/{id}/renew", post(api::renew_subscription))
        // Receipts
        .route("/receipts", get(api::list_receipts))
        .route("/receipts/{id}", get(api::get_receipt))
        .with_state(state)
        // Embedded UI
        .fallback(serve_ui);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    info!("Wallet capability listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Embedded UI ──────────────────────────────────────────────────────────────

async fn serve_ui(req: Request<Body>) -> Response {
    let path = req.uri().path();
    let rel = path.strip_prefix("/ui").unwrap_or(path);
    let rel = rel.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };

    match UI_ASSETS.get_file(rel) {
        Some(file) => (
            [(header::CONTENT_TYPE, ui_mime(rel))],
            Body::from(file.contents()),
        )
            .into_response(),
        None => {
            if path.starts_with("/ui") {
                match UI_ASSETS.get_file("index.html") {
                    Some(index) => (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        Body::from(index.contents()),
                    )
                        .into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

fn ui_mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

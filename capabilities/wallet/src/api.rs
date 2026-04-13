use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::chain::ChainBackend;
use crate::db::{Invoice, Receipt, SecretsDb, Transaction, WalletDb};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub db: WalletDb,
    pub secrets: SecretsDb,
    pub backend: Arc<dyn ChainBackend>,
    pub rpc_url: String,
    pub chain_id: u64,
}

// ── Request/response types ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateWalletRequest {
    pub label: Option<String>,
    pub import_key: Option<String>,
    pub passphrase: String,
}

#[derive(Deserialize)]
pub struct SendRequest {
    pub to_address: String,
    pub amount: String,
    pub token: String,
    pub passphrase: String,
    pub memo: Option<String>,
    pub peer_id: Option<String>,
    pub peer_name: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateInvoiceRequest {
    pub amount: String,
    pub token: String,
    pub chain: Option<String>,
    pub peer_id: Option<String>,
    pub resource: Option<String>,
    pub expires_in_secs: Option<i64>,
}

#[derive(Deserialize)]
pub struct RpcCreateInvoiceRequest {
    pub amount: String,
    pub token: String,
    pub chain: String,
    pub peer_id: String,
    pub resource: String,
}

#[derive(Deserialize)]
pub struct RpcVerifyPaymentRequest {
    pub invoice_id: String,
}

#[derive(Deserialize)]
pub struct ListParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<String>,
    pub direction: Option<String>,
    pub peer_id: Option<String>,
    pub resource_type: Option<String>,
}

#[derive(Serialize)]
pub struct ApiError {
    pub error: String,
}

type ApiResult = Result<Json<Value>, (axum::http::StatusCode, Json<ApiError>)>;

fn err(status: axum::http::StatusCode, msg: impl Into<String>) -> (axum::http::StatusCode, Json<ApiError>) {
    (status, Json(ApiError { error: msg.into() }))
}

fn bad_request(msg: impl Into<String>) -> (axum::http::StatusCode, Json<ApiError>) {
    err(axum::http::StatusCode::BAD_REQUEST, msg)
}

fn not_found(msg: impl Into<String>) -> (axum::http::StatusCode, Json<ApiError>) {
    err(axum::http::StatusCode::NOT_FOUND, msg)
}

fn internal(msg: impl Into<String>) -> (axum::http::StatusCode, Json<ApiError>) {
    err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg)
}

// ── Health ───────────────────────────────────────────────────────────────────

pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

// ── Wallets ──────────────────────────────────────────────────────────────────

pub async fn list_wallets(State(state): State<AppState>) -> ApiResult {
    let wallets = state.db.list_wallets().map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!(wallets)))
}

pub async fn create_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateWalletRequest>,
) -> ApiResult {
    if req.passphrase.is_empty() {
        return Err(bad_request("passphrase is required"));
    }

    let (address, encrypted) = if let Some(ref key) = req.import_key {
        state
            .backend
            .import_keypair(key, req.passphrase.as_bytes())
            .await
            .map_err(|e| bad_request(e.to_string()))?
    } else {
        state
            .backend
            .generate_keypair(req.passphrase.as_bytes())
            .await
            .map_err(|e| internal(e.to_string()))?
    };

    let wallet = crate::db::Wallet {
        id: Uuid::now_v7().to_string(),
        chain: state.backend.chain_name().to_string(),
        label: req.label,
        address,
        created_at: now(),
        is_default: false,
    };

    // If this is the first wallet, make it default
    let existing = state.db.list_wallets().map_err(|e| internal(e.to_string()))?;
    let is_first = existing.is_empty();

    state
        .db
        .insert_wallet(&wallet)
        .map_err(|e| internal(e.to_string()))?;

    if is_first {
        state
            .db
            .set_default_wallet(&wallet.id)
            .map_err(|e| internal(e.to_string()))?;
    }

    state
        .secrets
        .insert_secret(&wallet.id, &encrypted)
        .map_err(|e| internal(e.to_string()))?;

    // Re-fetch to get is_default
    let wallet = state
        .db
        .get_wallet(&wallet.id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| internal("wallet not found after insert"))?;

    Ok(Json(json!(wallet)))
}

pub async fn delete_wallet(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let deleted = state.db.delete_wallet(&id).map_err(|e| internal(e.to_string()))?;
    if !deleted {
        return Err(not_found("wallet not found"));
    }
    let _ = state.secrets.delete_secret(&id);
    Ok(Json(json!({"deleted": true})))
}

pub async fn get_balance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let wallet = state
        .db
        .get_wallet(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("wallet not found"))?;

    let balances = state
        .backend
        .get_balance(&wallet.address)
        .await
        .map_err(|e| internal(e.to_string()))?;

    Ok(Json(json!({
        "wallet_id": wallet.id,
        "address": wallet.address,
        "balances": balances,
    })))
}

pub async fn set_default(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let _ = state
        .db
        .get_wallet(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("wallet not found"))?;

    state
        .db
        .set_default_wallet(&id)
        .map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!({"default": true})))
}

// ── Send ─────────────────────────────────────────────────────────────────────

pub async fn send_payment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SendRequest>,
) -> ApiResult {
    let wallet = state
        .db
        .get_wallet(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("wallet not found"))?;

    let encrypted = state
        .secrets
        .get_secret(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| internal("secret not found for wallet"))?;

    let tx_hash = state
        .backend
        .send(&encrypted, req.passphrase.as_bytes(), &req.to_address, &req.amount, &req.token)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let tx_id = Uuid::now_v7().to_string();
    let tx = Transaction {
        id: tx_id.clone(),
        wallet_id: wallet.id.clone(),
        direction: "out".to_string(),
        chain_tx_id: Some(tx_hash.clone()),
        from_addr: Some(wallet.address.clone()),
        to_addr: Some(req.to_address.clone()),
        amount: req.amount.clone(),
        token: req.token.clone(),
        status: "pending".to_string(),
        created_at: now(),
        confirmed_at: None,
        invoice_id: None,
    };
    state
        .db
        .insert_transaction(&tx)
        .map_err(|e| internal(e.to_string()))?;

    // Create receipt
    let receipt = Receipt {
        id: Uuid::now_v7().to_string(),
        transaction_id: tx_id.clone(),
        invoice_id: None,
        peer_id: req.peer_id.unwrap_or_else(|| req.to_address.clone()),
        peer_name: req.peer_name,
        direction: "sent".to_string(),
        amount: req.amount,
        token: req.token,
        description: req.memo.or(Some("Direct payment".to_string())),
        resource_type: Some("direct".to_string()),
        resource_id: None,
        created_at: now(),
    };
    let _ = state.db.insert_receipt(&receipt);

    Ok(Json(json!({
        "transaction_id": tx_id,
        "tx_hash": tx_hash,
        "status": "pending",
    })))
}

// ── Transactions ─────────────────────────────────────────────────────────────

pub async fn list_transactions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult {
    let txs = state
        .db
        .list_transactions(
            params.direction.as_deref(),
            params.status.as_deref(),
            params.limit.unwrap_or(50),
            params.offset.unwrap_or(0),
        )
        .map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!(txs)))
}

pub async fn get_transaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let tx = state
        .db
        .get_transaction(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("transaction not found"))?;
    Ok(Json(json!(tx)))
}

// ── Invoices ─────────────────────────────────────────────────────────────────

pub async fn create_invoice(
    State(state): State<AppState>,
    Json(req): Json<CreateInvoiceRequest>,
) -> ApiResult {
    let wallet = state
        .db
        .get_default_wallet()
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| bad_request("no default wallet configured"))?;

    let expires_in = req.expires_in_secs.unwrap_or(3600);
    let invoice = Invoice {
        id: Uuid::now_v7().to_string(),
        wallet_id: wallet.id,
        amount: req.amount,
        token: req.token,
        status: "pending".to_string(),
        peer_id: req.peer_id,
        resource: req.resource,
        created_at: now(),
        expires_at: now() + expires_in,
        paid_tx_id: None,
    };

    state
        .db
        .insert_invoice(&invoice)
        .map_err(|e| internal(e.to_string()))?;

    Ok(Json(json!({
        "invoice_id": invoice.id,
        "address": wallet.address,
        "amount": invoice.amount,
        "token": invoice.token,
        "chain": format!("evm:{}", state.chain_id),
        "expiry": invoice.expires_at,
    })))
}

pub async fn list_invoices(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult {
    let invoices = state
        .db
        .list_invoices(
            params.status.as_deref(),
            params.limit.unwrap_or(50),
            params.offset.unwrap_or(0),
        )
        .map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!(invoices)))
}

pub async fn get_invoice(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let invoice = state
        .db
        .get_invoice(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("invoice not found"))?;
    Ok(Json(json!(invoice)))
}

pub async fn check_invoice(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let invoice = state
        .db
        .get_invoice(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("invoice not found"))?;

    if invoice.status == "paid" {
        return Ok(Json(json!({"paid": true, "tx_hash": invoice.paid_tx_id})));
    }
    if invoice.status == "expired" {
        return Ok(Json(json!({"paid": false, "expired": true})));
    }

    // Check on-chain
    let wallet = state
        .db
        .get_wallet(&invoice.wallet_id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| internal("wallet not found for invoice"))?;

    let confirmation = state
        .backend
        .check_payment(
            &wallet.address,
            &invoice.amount,
            &invoice.token,
            invoice.created_at as u64,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;

    if let Some(conf) = confirmation {
        state
            .db
            .update_invoice_status(&id, "paid", Some(&conf.tx_hash))
            .map_err(|e| internal(e.to_string()))?;
        Ok(Json(json!({"paid": true, "tx_hash": conf.tx_hash})))
    } else {
        // Check expiry
        if invoice.expires_at < now() {
            state
                .db
                .update_invoice_status(&id, "expired", None)
                .map_err(|e| internal(e.to_string()))?;
            Ok(Json(json!({"paid": false, "expired": true})))
        } else {
            Ok(Json(json!({"paid": false, "expired": false})))
        }
    }
}

// ── RPC (internal, capability-to-capability) ─────────────────────────────────

pub async fn rpc_create_invoice(
    State(state): State<AppState>,
    Json(req): Json<RpcCreateInvoiceRequest>,
) -> ApiResult {
    let wallet = state
        .db
        .get_default_wallet()
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| bad_request("no default wallet configured"))?;

    let invoice = Invoice {
        id: Uuid::now_v7().to_string(),
        wallet_id: wallet.id,
        amount: req.amount,
        token: req.token,
        status: "pending".to_string(),
        peer_id: Some(req.peer_id),
        resource: Some(req.resource),
        created_at: now(),
        expires_at: now() + 3600, // 1 hour default
        paid_tx_id: None,
    };

    state
        .db
        .insert_invoice(&invoice)
        .map_err(|e| internal(e.to_string()))?;

    Ok(Json(json!({
        "invoice_id": invoice.id,
        "address": wallet.address,
        "amount": invoice.amount,
        "token": invoice.token,
        "chain": req.chain,
        "expiry": invoice.expires_at,
    })))
}

pub async fn rpc_verify_payment(
    State(state): State<AppState>,
    Json(req): Json<RpcVerifyPaymentRequest>,
) -> ApiResult {
    let invoice = state
        .db
        .get_invoice(&req.invoice_id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("invoice not found"))?;

    if invoice.status == "paid" {
        return Ok(Json(json!({"paid": true, "tx_hash": invoice.paid_tx_id})));
    }

    // Check on-chain
    let wallet = state
        .db
        .get_wallet(&invoice.wallet_id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| internal("wallet not found"))?;

    let confirmation = state
        .backend
        .check_payment(
            &wallet.address,
            &invoice.amount,
            &invoice.token,
            invoice.created_at as u64,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;

    if let Some(conf) = confirmation {
        state
            .db
            .update_invoice_status(&req.invoice_id, "paid", Some(&conf.tx_hash))
            .map_err(|e| internal(e.to_string()))?;
        Ok(Json(json!({"paid": true, "tx_hash": conf.tx_hash})))
    } else {
        Ok(Json(json!({"paid": false})))
    }
}

// ── Subscriptions ────────────────────────────────────────────────────────────

pub async fn list_subscriptions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult {
    let subs = state
        .db
        .list_subscriptions(params.status.as_deref())
        .map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!(subs)))
}

pub async fn cancel_subscription(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let cancelled = state
        .db
        .cancel_subscription(&id, now())
        .map_err(|e| internal(e.to_string()))?;
    if !cancelled {
        return Err(bad_request(
            "subscription not found or not active",
        ));
    }
    let sub = state
        .db
        .get_subscription(&id)
        .map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!(sub)))
}

pub async fn renew_subscription(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let sub = state
        .db
        .get_subscription(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("subscription not found"))?;

    if sub.status == "active" {
        return Err(bad_request("subscription is already active"));
    }

    // For now, return the subscription info so the UI can trigger a payment flow.
    // The actual renewal happens when the payment completes and the grant is created.
    Ok(Json(json!({
        "subscription": sub,
        "action": "payment_required",
        "amount": sub.amount,
        "token": sub.token,
        "chain": sub.chain,
    })))
}

// ── Receipts ─────────────────────────────────────────────────────────────────

pub async fn list_receipts(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult {
    let receipts = state
        .db
        .list_receipts(
            params.peer_id.as_deref(),
            params.direction.as_deref(),
            params.resource_type.as_deref(),
            params.limit.unwrap_or(50),
            params.offset.unwrap_or(0),
        )
        .map_err(|e| internal(e.to_string()))?;
    Ok(Json(json!(receipts)))
}

pub async fn get_receipt(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let receipt = state
        .db
        .get_receipt(&id)
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| not_found("receipt not found"))?;
    Ok(Json(json!(receipt)))
}

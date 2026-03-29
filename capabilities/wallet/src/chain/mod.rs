pub mod evm;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Balance for a single token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBalance {
    pub token: String,
    pub amount: String,
    pub decimals: u8,
}

/// Confirmation of a received payment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentConfirmation {
    pub tx_hash: String,
    pub amount: String,
    pub confirmed_at: u64,
    pub block_number: u64,
}

/// Chain-agnostic wallet backend trait.
///
/// Each chain implementation (EVM, Monero, Lightning, etc.) implements this
/// trait. The wallet capability dispatches to the appropriate backend based
/// on the `chain` field in the wallet or invoice.
#[async_trait::async_trait]
pub trait ChainBackend: Send + Sync {
    /// Human-readable chain name (e.g. "evm", "monero", "lightning").
    fn chain_name(&self) -> &str;

    /// Generate a new keypair. Returns (address, encrypted_secret).
    async fn generate_keypair(&self, passphrase: &[u8]) -> Result<(String, Vec<u8>)>;

    /// Import from a private key hex string. Returns (address, encrypted_secret).
    async fn import_keypair(&self, secret: &str, passphrase: &[u8]) -> Result<(String, Vec<u8>)>;

    /// Fetch balances for an address. Returns a balance per token.
    async fn get_balance(&self, address: &str) -> Result<Vec<TokenBalance>>;

    /// Send a transaction. Returns the chain tx hash.
    async fn send(
        &self,
        encrypted_secret: &[u8],
        passphrase: &[u8],
        to_address: &str,
        amount: &str,
        token: &str,
    ) -> Result<String>;

    /// Check if a payment has been received at `address` matching the criteria.
    async fn check_payment(
        &self,
        address: &str,
        expected_amount: &str,
        token: &str,
        since_timestamp: u64,
    ) -> Result<Option<PaymentConfirmation>>;
}

/// Get the chain backend for a given chain name.
#[allow(dead_code)]
pub fn get_backend(chain: &str, rpc_url: &str, chain_id: u64) -> Option<Arc<dyn ChainBackend>> {
    match chain {
        "evm" => Some(Arc::new(evm::EvmBackend::new(
            rpc_url.to_string(),
            chain_id,
        ))),
        _ => None,
    }
}

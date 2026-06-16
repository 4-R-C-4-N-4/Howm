use alloy::{
    network::{EthereumWallet, TransactionBuilder},
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use anyhow::{anyhow, Result};

use super::{ChainBackend, PaymentConfirmation, TokenBalance};
use crate::crypto;

/// EVM-compatible chain backend (Ethereum, Base, Arbitrum, etc.).
pub struct EvmBackend {
    rpc_url: String,
    chain_id: u64,
}

impl EvmBackend {
    pub fn new(rpc_url: String, chain_id: u64) -> Self {
        Self { rpc_url, chain_id }
    }

    fn parse_eth_amount(amount: &str) -> Result<U256> {
        let parts: Vec<&str> = amount.split('.').collect();
        match parts.len() {
            1 => {
                let whole: u128 = parts[0]
                    .parse()
                    .map_err(|_| anyhow!("invalid amount: {}", amount))?;
                Ok(U256::from(whole) * U256::from(10u64.pow(18)))
            }
            2 => {
                let whole: u128 = if parts[0].is_empty() {
                    0
                } else {
                    parts[0]
                        .parse()
                        .map_err(|_| anyhow!("invalid amount: {}", amount))?
                };
                let decimals_str = parts[1];
                if decimals_str.len() > 18 {
                    return Err(anyhow!("too many decimal places: {}", amount));
                }
                let padded = format!("{:0<18}", decimals_str);
                let frac: u128 = padded
                    .parse()
                    .map_err(|_| anyhow!("invalid amount: {}", amount))?;
                Ok(U256::from(whole) * U256::from(10u64.pow(18)) + U256::from(frac))
            }
            _ => Err(anyhow!("invalid amount format: {}", amount)),
        }
    }

    fn wei_to_eth(wei: U256) -> String {
        let divisor = U256::from(10u64.pow(18));
        let whole = wei / divisor;
        let frac = wei % divisor;

        if frac.is_zero() {
            format!("{}", whole)
        } else {
            let frac_str = format!("{:018}", frac.to::<u128>());
            let trimmed = frac_str.trim_end_matches('0');
            format!("{}.{}", whole, trimmed)
        }
    }

    fn parse_rpc_url(&self) -> Result<reqwest::Url> {
        self.rpc_url.parse().map_err(|e| anyhow!("bad RPC URL: {}", e))
    }
}

#[async_trait::async_trait]
impl ChainBackend for EvmBackend {
    fn chain_name(&self) -> &str {
        "evm"
    }

    async fn generate_keypair(&self, passphrase: &[u8]) -> Result<(String, Vec<u8>)> {
        let signer = PrivateKeySigner::random();
        let address = format!("{:?}", signer.address());
        let key_bytes = signer.to_bytes();
        let encrypted = crypto::encrypt_secret(key_bytes.as_slice(), passphrase)?;
        Ok((address, encrypted))
    }

    async fn import_keypair(&self, secret: &str, passphrase: &[u8]) -> Result<(String, Vec<u8>)> {
        let secret_clean = secret.strip_prefix("0x").unwrap_or(secret);
        let key_bytes =
            hex::decode(secret_clean).map_err(|_| anyhow!("invalid hex private key"))?;
        if key_bytes.len() != 32 {
            return Err(anyhow!(
                "private key must be 32 bytes, got {}",
                key_bytes.len()
            ));
        }

        let signer: PrivateKeySigner = secret_clean
            .parse()
            .map_err(|e| anyhow!("invalid private key: {}", e))?;
        let address = format!("{:?}", signer.address());
        let encrypted = crypto::encrypt_secret(&key_bytes, passphrase)?;
        Ok((address, encrypted))
    }

    async fn get_balance(&self, address: &str) -> Result<Vec<TokenBalance>> {
        let provider = ProviderBuilder::new()
            .connect_http(self.parse_rpc_url()?);

        let addr: Address = address
            .parse()
            .map_err(|_| anyhow!("invalid address: {}", address))?;

        let balance = provider.get_balance(addr).await?;

        Ok(vec![TokenBalance {
            token: "ETH".to_string(),
            amount: Self::wei_to_eth(balance),
            decimals: 18,
        }])
    }

    async fn send(
        &self,
        encrypted_secret: &[u8],
        passphrase: &[u8],
        to_address: &str,
        amount: &str,
        token: &str,
    ) -> Result<String> {
        if token != "ETH" {
            return Err(anyhow!("ERC-20 transfers not yet implemented, only ETH"));
        }

        let key_bytes = crypto::decrypt_secret(encrypted_secret, passphrase)?;

        let signer: PrivateKeySigner = hex::encode(&key_bytes)
            .parse()
            .map_err(|e| anyhow!("invalid key: {}", e))?;
        let wallet = EthereumWallet::from(signer);

        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(self.parse_rpc_url()?);

        let to: Address = to_address
            .parse()
            .map_err(|_| anyhow!("invalid to_address: {}", to_address))?;

        let value = Self::parse_eth_amount(amount)?;

        let mut tx = alloy::rpc::types::TransactionRequest::default()
            .to(to)
            .value(value);
        tx.set_chain_id(self.chain_id);

        let tx_hash = provider.send_transaction(tx).await?.tx_hash().to_string();

        Ok(tx_hash)
    }

    async fn check_payment(
        &self,
        address: &str,
        _expected_amount: &str,
        _token: &str,
        _since_timestamp: u64,
    ) -> Result<Option<PaymentConfirmation>> {
        let _addr: Address = address
            .parse()
            .map_err(|_| anyhow!("invalid address: {}", address))?;

        // TODO: implement block scanning for incoming transfers.
        // V1 relies on the invoice flow: buyer pays, retries with invoice_id,
        // seller's wallet checks chain for incoming txs.
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_eth_amounts() {
        let wei = EvmBackend::parse_eth_amount("1").unwrap();
        assert_eq!(wei, U256::from(10u64.pow(18)));

        let wei = EvmBackend::parse_eth_amount("0.001").unwrap();
        assert_eq!(wei, U256::from(10u64.pow(15)));

        let wei = EvmBackend::parse_eth_amount("0.5").unwrap();
        assert_eq!(wei, U256::from(5u64 * 10u64.pow(17)));

        let wei = EvmBackend::parse_eth_amount("100").unwrap();
        assert_eq!(wei, U256::from(100u128 * 10u128.pow(18)));
    }

    #[test]
    fn wei_to_eth_display() {
        let one_eth = U256::from(10u64.pow(18));
        assert_eq!(EvmBackend::wei_to_eth(one_eth), "1");

        let half_eth = U256::from(5u64 * 10u64.pow(17));
        assert_eq!(EvmBackend::wei_to_eth(half_eth), "0.5");

        let milli_eth = U256::from(10u64.pow(15));
        assert_eq!(EvmBackend::wei_to_eth(milli_eth), "0.001");

        assert_eq!(EvmBackend::wei_to_eth(U256::ZERO), "0");
    }

    #[test]
    fn parse_roundtrip() {
        for amount in &["0.001", "1", "0.5", "100", "0.123456789"] {
            let wei = EvmBackend::parse_eth_amount(amount).unwrap();
            let back = EvmBackend::wei_to_eth(wei);
            let wei2 = EvmBackend::parse_eth_amount(&back).unwrap();
            assert_eq!(wei, wei2, "roundtrip failed for {}", amount);
        }
    }

    #[tokio::test]
    async fn generate_keypair_produces_valid_address() {
        let backend = EvmBackend::new("https://sepolia.base.org".to_string(), 84532);
        let (address, encrypted) = backend.generate_keypair(b"testpass").await.unwrap();

        assert!(address.starts_with("0x"));
        assert_eq!(address.len(), 42);

        let decrypted = crypto::decrypt_secret(&encrypted, b"testpass").unwrap();
        assert_eq!(decrypted.len(), 32);
    }

    #[tokio::test]
    async fn import_keypair_with_prefix() {
        let backend = EvmBackend::new("https://sepolia.base.org".to_string(), 84532);
        let test_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let (address, _) = backend.import_keypair(test_key, b"pass").await.unwrap();
        assert!(address.starts_with("0x"));
        assert_eq!(address.len(), 42);
    }

    #[tokio::test]
    async fn import_invalid_key_fails() {
        let backend = EvmBackend::new("https://sepolia.base.org".to_string(), 84532);
        assert!(backend.import_keypair("not-hex", b"pass").await.is_err());
        assert!(backend.import_keypair("aabb", b"pass").await.is_err());
    }
}

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::NodeIdentity;

type HmacSha256 = Hmac<Sha256>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenInviteConfig {
    pub enabled: bool,
    pub token: String,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub max_peers: u32,
    pub rate_limit_per_hour: u32,
    pub current_peer_count: u32,
    pub label: String,
}

/// Create a new open invite, persisting to disk. Returns the invite link.
pub fn create(
    data_dir: &Path,
    identity: &NodeIdentity,
    endpoint: Option<String>,
    daemon_port: u16,
    max_peers: u32,
    label: String,
    expires_at: Option<u64>,
) -> anyhow::Result<(OpenInviteConfig, String)> {
    let pubkey = identity
        .wg_pubkey
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("WG not initialized"))?;
    let ep = endpoint
        .or(identity.wg_endpoint.clone())
        .unwrap_or_else(|| "0.0.0.0:51820".to_string());

    // Refuse to create open invites with an unroutable endpoint.
    if ep.starts_with("0.0.0.0") {
        anyhow::bail!(
            "Cannot create open invite: WireGuard endpoint not configured. \
             Restart with --wg-endpoint <public-ip:port> or set HOWM_WG_ENDPOINT."
        );
    }

    let node_id = &identity.node_id;

    // Read WG private key for HMAC signing
    let priv_key_path = data_dir.join("wireguard").join("private_key");
    let priv_key = std::fs::read_to_string(&priv_key_path)?.trim().to_string();

    // Payload: node_id|wg_pubkey|endpoint|daemon_port
    let payload = format!("{}|{}|{}|{}", node_id, pubkey, ep, daemon_port);

    // Sign with HMAC-SHA256 using WG private key bytes
    let mut mac = HmacSha256::new_from_slice(priv_key.as_bytes())?;
    mac.update(payload.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig);

    // Token: base64url(node_id|wg_pubkey|endpoint|daemon_port|sig)
    let token_payload = format!("{}|{}", payload, sig_b64);
    let token = format!(
        "howm://open/{}",
        URL_SAFE_NO_PAD.encode(token_payload.as_bytes())
    );

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let config = OpenInviteConfig {
        enabled: true,
        token: token.clone(),
        created_at: now,
        expires_at,
        max_peers,
        rate_limit_per_hour: 10,
        current_peer_count: 0,
        label,
    };

    save(data_dir, &config)?;
    Ok((config, token))
}

/// Decode an open invite token into (node_id, wg_pubkey, endpoint, daemon_port, signature).
pub fn decode_open_invite(token: &str) -> anyhow::Result<(String, String, String, u16, String)> {
    let stripped = token
        .strip_prefix("howm://open/")
        .ok_or_else(|| anyhow::anyhow!("invalid open invite format"))?;
    let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
    let payload = String::from_utf8(bytes)?;
    let parts: Vec<&str> = payload.splitn(5, '|').collect();
    if parts.len() != 5 {
        return Err(anyhow::anyhow!(
            "invalid open invite payload — expected 5 fields, got {}",
            parts.len()
        ));
    }
    Ok((
        parts[0].to_string(), // node_id
        parts[1].to_string(), // wg_pubkey
        parts[2].to_string(), // endpoint
        parts[3].parse()?,    // daemon_port
        parts[4].to_string(), // signature
    ))
}

/// Validate an open invite token's HMAC signature using the host's WG private key.
pub fn validate_token(data_dir: &Path, token: &str) -> anyhow::Result<bool> {
    let (node_id, pubkey, endpoint, daemon_port, sig_b64) = decode_open_invite(token)?;

    let priv_key_path = data_dir.join("wireguard").join("private_key");
    let priv_key = std::fs::read_to_string(&priv_key_path)?.trim().to_string();

    let payload = format!("{}|{}|{}|{}", node_id, pubkey, endpoint, daemon_port);
    let mut mac = HmacSha256::new_from_slice(priv_key.as_bytes())?;
    mac.update(payload.as_bytes());

    let sig_bytes = URL_SAFE_NO_PAD.decode(&sig_b64)?;
    Ok(mac.verify_slice(&sig_bytes).is_ok())
}

pub fn load(data_dir: &Path) -> anyhow::Result<Option<OpenInviteConfig>> {
    let path = data_dir.join("open_invite.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&text)?))
}

pub fn save(data_dir: &Path, config: &OpenInviteConfig) -> anyhow::Result<()> {
    let path = data_dir.join("open_invite.json");
    let tmp = data_dir.join("open_invite.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(config)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn revoke(data_dir: &Path) -> anyhow::Result<()> {
    let path = data_dir.join("open_invite.json");
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

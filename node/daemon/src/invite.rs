use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::identity::NodeIdentity;
use crate::wireguard;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PendingInvite {
    pub psk: String,               // WireGuard pre-shared key
    pub assigned_ip: String,       // IP we assigned for the peer on our wg0
    pub our_pubkey: String,        // our WG public key
    pub our_endpoint: String,      // our public endpoint
    pub our_wg_address: String,    // our WG address
    pub expires_at: u64,
}

/// Decoded invite fields (from the invite code).
pub struct DecodedInvite {
    pub their_pubkey: String,
    pub their_endpoint: String,
    pub their_wg_address: String,
    pub psk: String,
    pub my_assigned_ip: String,
    pub expires_at: u64,
}

/// Generate a new invite code.
/// Format: howm://invite/<base64(our_pubkey:our_endpoint:our_wg_addr:psk:assigned_ip:expiry)>
pub fn generate(
    data_dir: &Path,
    identity: &NodeIdentity,
    endpoint_override: Option<String>,
    ttl_s: u64,
) -> anyhow::Result<String> {
    let our_pubkey = identity.wg_pubkey.as_deref()
        .ok_or_else(|| anyhow::anyhow!("WG not initialized — no public key"))?;
    let our_wg_address = identity.wg_address.as_deref()
        .ok_or_else(|| anyhow::anyhow!("WG not initialized — no address"))?;
    let our_endpoint = endpoint_override
        .or(identity.wg_endpoint.clone())
        .unwrap_or_else(|| "0.0.0.0:51820".to_string());

    let psk = wireguard::generate_psk();
    let assigned_ip = wireguard::assign_next_address(data_dir)?;
    let expires_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + ttl_s;

    let invite = PendingInvite {
        psk: psk.clone(),
        assigned_ip: assigned_ip.clone(),
        our_pubkey: our_pubkey.to_string(),
        our_endpoint: our_endpoint.clone(),
        our_wg_address: our_wg_address.to_string(),
        expires_at,
    };

    // Save to pending_invites.json
    let mut invites = load_pending(data_dir).unwrap_or_default();
    invites.push(invite);
    save_pending(data_dir, &invites)?;

    // Encode: our_pubkey:our_endpoint:our_wg_addr:psk:assigned_ip:expiry
    let payload = format!(
        "{}:{}:{}:{}:{}:{}",
        our_pubkey, our_endpoint, our_wg_address, psk, assigned_ip, expires_at
    );
    let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    Ok(format!("howm://invite/{}", encoded))
}

/// Decode an invite code into its constituent fields.
pub fn decode(invite_code: &str) -> anyhow::Result<DecodedInvite> {
    let stripped = invite_code
        .strip_prefix("howm://invite/")
        .ok_or_else(|| anyhow::anyhow!("invalid invite code format"))?;
    let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
    let payload = String::from_utf8(bytes)?;
    let parts: Vec<&str> = payload.splitn(6, ':').collect();
    if parts.len() != 6 {
        return Err(anyhow::anyhow!("invalid invite payload — expected 6 fields, got {}", parts.len()));
    }
    Ok(DecodedInvite {
        their_pubkey: parts[0].to_string(),
        their_endpoint: parts[1].to_string(),
        their_wg_address: parts[2].to_string(),
        psk: parts[3].to_string(),
        my_assigned_ip: parts[4].to_string(),
        expires_at: parts[5].parse()?,
    })
}

/// Consume a pending invite by matching its PSK. Returns the invite if valid.
pub fn consume_by_psk(data_dir: &Path, psk: &str) -> anyhow::Result<Option<PendingInvite>> {
    let mut invites = load_pending(data_dir).unwrap_or_default();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let pos = invites.iter().position(|i| i.psk == psk);
    match pos {
        None => Ok(None),
        Some(idx) => {
            let invite = invites.remove(idx);
            save_pending(data_dir, &invites)?;
            if invite.expires_at < now {
                Ok(None) // expired
            } else {
                Ok(Some(invite))
            }
        }
    }
}

fn load_pending(data_dir: &Path) -> anyhow::Result<Vec<PendingInvite>> {
    let path = data_dir.join("pending_invites.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

fn save_pending(data_dir: &Path, invites: &[PendingInvite]) -> anyhow::Result<()> {
    let path = data_dir.join("pending_invites.json");
    let tmp = data_dir.join("pending_invites.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(invites)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

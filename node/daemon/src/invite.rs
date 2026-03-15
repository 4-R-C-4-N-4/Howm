use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PendingInvite {
    pub token: String,        // 32-byte hex random token
    pub node_address: String, // our address
    pub node_port: u16,
    pub expires_at: u64,      // unix timestamp
}

pub fn generate(data_dir: &Path, node_address: &str, node_port: u16, ttl_s: u64) -> anyhow::Result<String> {
    let token = generate_token();
    let expires_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + ttl_s;

    let invite = PendingInvite {
        token: token.clone(),
        node_address: node_address.to_string(),
        node_port,
        expires_at,
    };

    // Save to pending_invites.json
    let mut invites = load_pending(data_dir).unwrap_or_default();
    invites.push(invite.clone());
    save_pending(data_dir, &invites)?;

    // Encode as base64: "address:port:token:expires_at"
    let payload = format!("{}:{}:{}:{}", node_address, node_port, token, expires_at);
    let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    Ok(format!("howm://invite/{}", encoded))
}

pub fn decode(invite_code: &str) -> anyhow::Result<(String, u16, String, u64)> {
    let stripped = invite_code
        .strip_prefix("howm://invite/")
        .ok_or_else(|| anyhow::anyhow!("invalid invite code format"))?;
    let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
    let payload = String::from_utf8(bytes)?;
    let parts: Vec<&str> = payload.splitn(4, ':').collect();
    if parts.len() != 4 {
        return Err(anyhow::anyhow!("invalid invite payload"));
    }
    let address = parts[0].to_string();
    let port: u16 = parts[1].parse()?;
    let token = parts[2].to_string();
    let expires_at: u64 = parts[3].parse()?;
    Ok((address, port, token, expires_at))
}

pub fn consume(data_dir: &Path, token: &str) -> anyhow::Result<Option<PendingInvite>> {
    let mut invites = load_pending(data_dir).unwrap_or_default();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let pos = invites.iter().position(|i| i.token == token);
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

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
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

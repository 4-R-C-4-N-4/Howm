use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use std::net::Ipv6Addr;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::NodeIdentity;
use crate::wireguard;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PendingInvite {
    pub psk: String,            // WireGuard pre-shared key
    pub assigned_ip: String,    // IP we assigned for the peer on our wg0
    pub our_pubkey: String,     // our WG public key
    pub our_endpoint: String,   // our public endpoint (IPv4)
    pub our_wg_address: String, // our WG address
    pub our_daemon_port: u16,   // our daemon API port
    pub expires_at: u64,
    #[serde(default)]
    pub our_ipv6_candidates: Vec<String>, // GUA IPv6 addresses
    #[serde(default)]
    pub our_wg_port: u16, // actual WG listen port
}

/// Decoded invite fields (from the invite code).
pub struct DecodedInvite {
    pub their_pubkey: String,
    pub their_endpoint: String, // WG endpoint (public IPv4 addr:port)
    pub their_wg_address: String,
    pub psk: String,
    pub my_assigned_ip: String,
    pub their_daemon_port: u16, // peer's daemon API port
    pub expires_at: u64,
    pub their_ipv6_candidates: Vec<Ipv6Addr>, // GUA IPv6 addresses
    pub their_wg_port: u16,                   // peer's WG listen port
}

/// Generate a new invite code.
///
/// Format v2 (pipe-delimited, base64url):
/// `howm://invite/<base64(pubkey|endpoint|wg_addr|psk|assigned_ip|daemon_port|expiry|ipv6_csv|wg_port)>`
///
/// Fields 8-9 are new (IPv6 candidates and WG port). Older parsers that split
/// on `|` and take the first 7 will still work — new fields are trailing.
pub fn generate(
    data_dir: &Path,
    identity: &NodeIdentity,
    endpoint_override: Option<String>,
    daemon_port: u16,
    ttl_s: u64,
    ipv6_guas: &[Ipv6Addr],
    wg_port: u16,
) -> anyhow::Result<String> {
    let our_pubkey = identity
        .wg_pubkey
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("WG not initialized — no public key"))?;
    let our_wg_address = identity
        .wg_address
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("WG not initialized — no address"))?;
    let our_endpoint = endpoint_override
        .or(identity.wg_endpoint.clone())
        .unwrap_or_else(|| "0.0.0.0:51820".to_string());

    // Refuse to create invites with an unroutable endpoint AND no IPv6
    if our_endpoint.starts_with("0.0.0.0") && ipv6_guas.is_empty() {
        anyhow::bail!(
            "Cannot create invite: no reachable endpoint. Either configure \
             --wg-endpoint <public-ip:port>, or ensure IPv6 is available."
        );
    }

    let psk = wireguard::generate_psk();
    let assigned_ip = wireguard::assign_next_address(data_dir)?;
    let expires_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + ttl_s;

    // Format IPv6 GUAs as comma-separated string (empty string if none)
    let ipv6_csv = ipv6_guas
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let invite = PendingInvite {
        psk: psk.clone(),
        assigned_ip: assigned_ip.clone(),
        our_pubkey: our_pubkey.to_string(),
        our_endpoint: our_endpoint.clone(),
        our_wg_address: our_wg_address.to_string(),
        our_daemon_port: daemon_port,
        expires_at,
        our_ipv6_candidates: ipv6_guas.iter().map(|a| a.to_string()).collect(),
        our_wg_port: wg_port,
    };

    // Save to pending_invites.json
    let mut invites = load_pending(data_dir).unwrap_or_default();
    invites.push(invite);
    save_pending(data_dir, &invites)?;

    // Encode with | delimiter (endpoints contain colons)
    // v2 format: 9 fields (appended ipv6_csv and wg_port)
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        our_pubkey,
        our_endpoint,
        our_wg_address,
        psk,
        assigned_ip,
        daemon_port,
        expires_at,
        ipv6_csv,
        wg_port,
    );
    let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    Ok(format!("howm://invite/{}", encoded))
}

/// Decode an invite code into its constituent fields.
/// Supports both v1 (7 fields) and v2 (9 fields) formats.
pub fn decode(invite_code: &str) -> anyhow::Result<DecodedInvite> {
    let stripped = invite_code
        .strip_prefix("howm://invite/")
        .ok_or_else(|| anyhow::anyhow!("invalid invite code format"))?;
    let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
    let payload = String::from_utf8(bytes)?;
    let parts: Vec<&str> = payload.splitn(9, '|').collect();
    if parts.len() < 7 {
        return Err(anyhow::anyhow!(
            "invalid invite payload — expected at least 7 fields, got {}",
            parts.len()
        ));
    }

    // Parse IPv6 candidates (field 8, index 7) — empty string = no IPv6
    let ipv6_candidates = if parts.len() > 7 && !parts[7].is_empty() {
        parts[7]
            .split(',')
            .filter_map(|s| s.parse::<Ipv6Addr>().ok())
            .collect()
    } else {
        vec![]
    };

    // Parse WG port (field 9, index 8) — default to parsed from endpoint if missing
    let wg_port = if parts.len() > 8 {
        parts[8].parse::<u16>().unwrap_or_else(|_| {
            // Fall back to port from endpoint
            extract_port_from_endpoint(parts[1]).unwrap_or(41641)
        })
    } else {
        extract_port_from_endpoint(parts[1]).unwrap_or(41641)
    };

    Ok(DecodedInvite {
        their_pubkey: parts[0].to_string(),
        their_endpoint: parts[1].to_string(),
        their_wg_address: parts[2].to_string(),
        psk: parts[3].to_string(),
        my_assigned_ip: parts[4].to_string(),
        their_daemon_port: parts[5].parse()?,
        expires_at: parts[6].parse()?,
        their_ipv6_candidates: ipv6_candidates,
        their_wg_port: wg_port,
    })
}

/// Extract the port number from an endpoint string like "1.2.3.4:51820"
/// or "[::1]:51820".
fn extract_port_from_endpoint(endpoint: &str) -> Option<u16> {
    endpoint.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
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

/// Build the best endpoint to attempt connecting to, preferring IPv6.
///
/// Returns a list of endpoints to try in priority order:
/// 1. IPv6 GUA addresses (with WG port)
/// 2. IPv4 endpoint (as-is from the invite)
pub fn connection_candidates(decoded: &DecodedInvite) -> Vec<String> {
    let mut candidates = Vec::new();

    // IPv6 first — globally routable, no NAT
    for addr in &decoded.their_ipv6_candidates {
        candidates.push(format!("[{}]:{}", addr, decoded.their_wg_port));
    }

    // IPv4 fallback
    if !decoded.their_endpoint.starts_with("0.0.0.0") {
        candidates.push(decoded.their_endpoint.clone());
    }

    candidates
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_v2_with_ipv6() {
        // Build a v2 payload manually
        let ipv6 = "2001:db8::1".parse::<Ipv6Addr>().unwrap();
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            "pubkey123",
            "1.2.3.4:41641",
            "100.222.0.1",
            "psk_value",
            "100.222.0.2",
            7000,
            9999999999u64,
            ipv6,
            41641,
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_pubkey, "pubkey123");
        assert_eq!(decoded.their_endpoint, "1.2.3.4:41641");
        assert_eq!(decoded.their_ipv6_candidates.len(), 1);
        assert_eq!(decoded.their_ipv6_candidates[0], ipv6);
        assert_eq!(decoded.their_wg_port, 41641);
    }

    #[test]
    fn test_decode_v1_compat() {
        // v1 format: only 7 fields, no IPv6 or wg_port
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            "pubkey123",
            "1.2.3.4:51820",
            "100.222.0.1",
            "psk_value",
            "100.222.0.2",
            7000,
            9999999999u64,
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_pubkey, "pubkey123");
        assert!(decoded.their_ipv6_candidates.is_empty());
        // Should extract port from endpoint
        assert_eq!(decoded.their_wg_port, 51820);
    }

    #[test]
    fn test_connection_candidates_ipv6_first() {
        let decoded = DecodedInvite {
            their_pubkey: "pk".to_string(),
            their_endpoint: "1.2.3.4:41641".to_string(),
            their_wg_address: "100.222.0.1".to_string(),
            psk: "psk".to_string(),
            my_assigned_ip: "100.222.0.2".to_string(),
            their_daemon_port: 7000,
            expires_at: u64::MAX,
            their_ipv6_candidates: vec!["2001:db8::1".parse().unwrap()],
            their_wg_port: 41641,
        };

        let candidates = connection_candidates(&decoded);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], "[2001:db8::1]:41641");
        assert_eq!(candidates[1], "1.2.3.4:41641");
    }

    #[test]
    fn test_connection_candidates_no_ipv6() {
        let decoded = DecodedInvite {
            their_pubkey: "pk".to_string(),
            their_endpoint: "1.2.3.4:41641".to_string(),
            their_wg_address: "100.222.0.1".to_string(),
            psk: "psk".to_string(),
            my_assigned_ip: "100.222.0.2".to_string(),
            their_daemon_port: 7000,
            expires_at: u64::MAX,
            their_ipv6_candidates: vec![],
            their_wg_port: 41641,
        };

        let candidates = connection_candidates(&decoded);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], "1.2.3.4:41641");
    }

    #[test]
    fn test_connection_candidates_skips_unroutable_ipv4() {
        let decoded = DecodedInvite {
            their_pubkey: "pk".to_string(),
            their_endpoint: "0.0.0.0:41641".to_string(),
            their_wg_address: "100.222.0.1".to_string(),
            psk: "psk".to_string(),
            my_assigned_ip: "100.222.0.2".to_string(),
            their_daemon_port: 7000,
            expires_at: u64::MAX,
            their_ipv6_candidates: vec!["2001:db8::1".parse().unwrap()],
            their_wg_port: 41641,
        };

        let candidates = connection_candidates(&decoded);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], "[2001:db8::1]:41641");
    }

    #[test]
    fn test_extract_port_from_endpoint() {
        assert_eq!(extract_port_from_endpoint("1.2.3.4:51820"), Some(51820));
        assert_eq!(extract_port_from_endpoint("[::1]:41641"), Some(41641));
        assert_eq!(extract_port_from_endpoint("garbage"), None);
    }
}

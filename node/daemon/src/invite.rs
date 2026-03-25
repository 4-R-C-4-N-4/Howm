use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use std::net::Ipv6Addr;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::NodeIdentity;
use crate::stun::{NatProfile, NatType};
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
#[allow(dead_code)]
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
    // v3 fields (NAT traversal)
    pub their_nat_type: Option<NatType>, // peer's NAT classification
    pub their_stride: i32,               // peer's port allocation stride
    pub their_relay_candidates: Vec<String>, // base64 WG pubkeys of relay-capable peers
}

/// Generate a new invite code.
///
/// Format v3 (pipe-delimited, base64url):
/// `howm://invite/<base64(pubkey|endpoint|wg_addr|psk|assigned_ip|daemon_port|expiry|ipv6_csv|wg_port|nat_type|stride|relay_csv)>`
///
/// Fields 8-9 added in v2 (IPv6, WG port). Fields 10-12 added in v3 (NAT info,
/// relay candidates). Older parsers that split on `|` and take fewer fields
/// will still work — new fields are trailing.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    data_dir: &Path,
    identity: &NodeIdentity,
    endpoint_override: Option<String>,
    daemon_port: u16,
    ttl_s: u64,
    ipv6_guas: &[Ipv6Addr],
    wg_port: u16,
    nat_profile: Option<&NatProfile>,
    relay_candidates: &[String],
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

    // v3 NAT fields (empty string if not applicable)
    let nat_type_str = nat_profile
        .map(|p| p.nat_type.to_string())
        .unwrap_or_default();
    let stride_str = nat_profile
        .map(|p| p.observed_stride.to_string())
        .unwrap_or_else(|| "0".to_string());
    let relay_csv = relay_candidates.join(",");

    // Encode with | delimiter (endpoints contain colons)
    // v3 format: 12 fields
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        our_pubkey,
        our_endpoint,
        our_wg_address,
        psk,
        assigned_ip,
        daemon_port,
        expires_at,
        ipv6_csv,
        wg_port,
        nat_type_str,
        stride_str,
        relay_csv,
    );
    let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    Ok(format!("howm://invite/{}", encoded))
}

/// Decode an invite code into its constituent fields.
/// Supports v1 (7 fields), v2 (9 fields), and v3 (12 fields) formats.
pub fn decode(invite_code: &str) -> anyhow::Result<DecodedInvite> {
    let stripped = invite_code
        .strip_prefix("howm://invite/")
        .ok_or_else(|| anyhow::anyhow!("invalid invite code format"))?;
    let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
    let payload = String::from_utf8(bytes)?;
    let parts: Vec<&str> = payload.splitn(12, '|').collect();
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
        parts[8]
            .parse::<u16>()
            .unwrap_or_else(|_| extract_port_from_endpoint(parts[1]).unwrap_or(41641))
    } else {
        extract_port_from_endpoint(parts[1]).unwrap_or(41641)
    };

    // v3 fields (index 9-11)
    let their_nat_type = if parts.len() > 9 && !parts[9].is_empty() {
        match parts[9] {
            "open" => Some(NatType::Open),
            "cone" => Some(NatType::Cone),
            "symmetric" => Some(NatType::Symmetric),
            "unknown" => Some(NatType::Unknown),
            _ => None,
        }
    } else {
        None
    };

    let their_stride = if parts.len() > 10 {
        parts[10].parse::<i32>().unwrap_or(0)
    } else {
        0
    };

    let their_relay_candidates = if parts.len() > 11 && !parts[11].is_empty() {
        parts[11].split(',').map(|s| s.to_string()).collect()
    } else {
        vec![]
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
        their_nat_type,
        their_stride,
        their_relay_candidates,
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
    Ok(serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!(
            "Failed to parse pending invites JSON, using empty list: {}",
            e
        );
        Vec::new()
    }))
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
            their_nat_type: None,
            their_stride: 0,
            their_relay_candidates: vec![],
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
            their_nat_type: None,
            their_stride: 0,
            their_relay_candidates: vec![],
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
            their_nat_type: None,
            their_stride: 0,
            their_relay_candidates: vec![],
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

    #[test]
    fn test_roundtrip_v3_full() {
        // Build a v3 payload with all 12 fields
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            "pubkey_v3",
            "203.0.113.5:41641",
            "100.222.0.1",
            "psk_v3",
            "100.222.0.2",
            7000,
            9999999999u64,
            "2001:db8::1,2001:db8::2",
            41641,
            "cone",
            4,
            "relay_peer_a,relay_peer_b",
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_pubkey, "pubkey_v3");
        assert_eq!(decoded.their_endpoint, "203.0.113.5:41641");
        assert_eq!(decoded.their_wg_address, "100.222.0.1");
        assert_eq!(decoded.psk, "psk_v3");
        assert_eq!(decoded.my_assigned_ip, "100.222.0.2");
        assert_eq!(decoded.their_daemon_port, 7000);
        assert_eq!(decoded.their_ipv6_candidates.len(), 2);
        assert_eq!(decoded.their_wg_port, 41641);
        assert_eq!(decoded.their_nat_type, Some(NatType::Cone));
        assert_eq!(decoded.their_stride, 4);
        assert_eq!(
            decoded.their_relay_candidates,
            vec!["relay_peer_a", "relay_peer_b"]
        );
    }

    #[test]
    fn test_v3_symmetric_nat_and_negative_stride() {
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            "sym_peer",
            "198.51.100.1:41641",
            "100.222.0.5",
            "psk_sym",
            "100.222.0.6",
            7000,
            9999999999u64,
            "",
            41641,
            "symmetric",
            -3,
            "relay_carol",
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_nat_type, Some(NatType::Symmetric));
        assert_eq!(decoded.their_stride, -3);
        assert_eq!(decoded.their_relay_candidates, vec!["relay_carol"]);
        assert!(decoded.their_ipv6_candidates.is_empty());
    }

    #[test]
    fn test_v3_no_nat_no_relay() {
        // v3 format but empty NAT and relay fields
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            "pk",
            "1.2.3.4:41641",
            "100.222.0.1",
            "psk",
            "100.222.0.2",
            7000,
            9999999999u64,
            "",
            41641,
            "", // no nat type
            "0",
            "", // no relay candidates
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_nat_type, None);
        assert_eq!(decoded.their_stride, 0);
        assert!(decoded.their_relay_candidates.is_empty());
    }

    #[test]
    fn test_v2_backward_compat_no_nat_fields() {
        // v2 format: only 9 fields (no NAT type, stride, or relay)
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            "pubkey_v2",
            "1.2.3.4:41641",
            "100.222.0.1",
            "psk_v2",
            "100.222.0.2",
            7000,
            9999999999u64,
            "2001:db8::1",
            41641,
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_pubkey, "pubkey_v2");
        assert_eq!(decoded.their_ipv6_candidates.len(), 1);
        assert_eq!(decoded.their_wg_port, 41641);
        // v3 fields should default gracefully
        assert_eq!(decoded.their_nat_type, None);
        assert_eq!(decoded.their_stride, 0);
        assert!(decoded.their_relay_candidates.is_empty());
    }

    #[test]
    fn test_v1_backward_compat_no_v3_fields() {
        // v1: 7 fields only — no IPv6, no WG port, no NAT
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            "pubkey_v1",
            "10.0.0.1:51820",
            "100.222.0.1",
            "psk_v1",
            "100.222.0.2",
            7000,
            9999999999u64,
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        let decoded = decode(&invite_code).unwrap();
        assert_eq!(decoded.their_pubkey, "pubkey_v1");
        assert!(decoded.their_ipv6_candidates.is_empty());
        assert_eq!(decoded.their_wg_port, 51820); // extracted from endpoint
        assert_eq!(decoded.their_nat_type, None);
        assert_eq!(decoded.their_stride, 0);
        assert!(decoded.their_relay_candidates.is_empty());
    }
}

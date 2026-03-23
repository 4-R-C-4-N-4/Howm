//! Accept token for two-way invite exchange (Tier 2 NAT traversal).
//!
//! When both peers are behind NAT, the one-way invite is insufficient.
//! The joiner creates an `howm://accept/` token and sends it back to the
//! inviter. Both peers then have each other's endpoint info and can
//! attempt a WG handshake hole punch.
//!
//! Format: `howm://accept/<base64url(inviter_pubkey|pubkey|ipv6_csv|external_ip|external_port|wg_port|nat_type|stride|psk)>`

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::net::Ipv6Addr;

use crate::stun::NatType;

/// Decoded accept token fields.
#[derive(Debug, Clone)]
pub struct DecodedAccept {
    /// Inviter's WG pubkey (binds this accept to a specific invite).
    pub inviter_pubkey: String,
    /// Joiner's WG public key.
    pub pubkey: String,
    /// Joiner's IPv6 GUA candidates.
    pub ipv6_candidates: Vec<Ipv6Addr>,
    /// Joiner's STUN-reflected external IP.
    pub external_ip: String,
    /// Joiner's STUN-reflected external port.
    pub external_port: u16,
    /// Joiner's actual WG listen port.
    pub wg_port: u16,
    /// Joiner's NAT classification.
    pub nat_type: NatType,
    /// Joiner's port allocation stride.
    pub observed_stride: i32,
    /// Pre-shared key (echoed from the invite).
    pub psk: String,
}

/// Generate an accept token.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    inviter_pubkey: &str,
    our_pubkey: &str,
    ipv6_guas: &[Ipv6Addr],
    external_ip: &str,
    external_port: u16,
    wg_port: u16,
    nat_type: NatType,
    stride: i32,
    psk: &str,
) -> String {
    let ipv6_csv = ipv6_guas
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        inviter_pubkey,
        our_pubkey,
        ipv6_csv,
        external_ip,
        external_port,
        wg_port,
        nat_type,
        stride,
        psk,
    );

    let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("howm://accept/{}", encoded)
}

/// Decode an accept token.
pub fn decode(token: &str) -> anyhow::Result<DecodedAccept> {
    let stripped = token
        .strip_prefix("howm://accept/")
        .ok_or_else(|| anyhow::anyhow!("invalid accept token format"))?;
    let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
    let payload = String::from_utf8(bytes)?;
    let parts: Vec<&str> = payload.splitn(9, '|').collect();
    if parts.len() != 9 {
        return Err(anyhow::anyhow!(
            "invalid accept payload — expected 9 fields, got {}",
            parts.len()
        ));
    }

    let ipv6_candidates = if parts[2].is_empty() {
        vec![]
    } else {
        parts[2]
            .split(',')
            .filter_map(|s| s.parse::<Ipv6Addr>().ok())
            .collect()
    };

    let nat_type = match parts[6] {
        "open" => NatType::Open,
        "cone" => NatType::Cone,
        "symmetric" => NatType::Symmetric,
        _ => NatType::Unknown,
    };

    Ok(DecodedAccept {
        inviter_pubkey: parts[0].to_string(),
        pubkey: parts[1].to_string(),
        ipv6_candidates,
        external_ip: parts[3].to_string(),
        external_port: parts[4].parse()?,
        wg_port: parts[5].parse()?,
        nat_type,
        observed_stride: parts[7].parse()?,
        psk: parts[8].to_string(),
    })
}

/// Build connection candidates from an accept token, IPv6 first.
pub fn connection_candidates(decoded: &DecodedAccept) -> Vec<String> {
    let mut candidates = Vec::new();

    // IPv6 first
    for addr in &decoded.ipv6_candidates {
        candidates.push(format!("[{}]:{}", addr, decoded.wg_port));
    }

    // IPv4 from STUN
    if !decoded.external_ip.is_empty() {
        candidates.push(format!("{}:{}", decoded.external_ip, decoded.external_port));
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accept_roundtrip() {
        let ipv6 = vec!["2001:db8::1".parse::<Ipv6Addr>().unwrap()];

        let token = generate(
            "alice_pubkey",
            "bob_pubkey",
            &ipv6,
            "203.0.113.5",
            41641,
            41641,
            NatType::Cone,
            0,
            "shared_psk_value",
        );

        assert!(token.starts_with("howm://accept/"));

        let decoded = decode(&token).unwrap();
        assert_eq!(decoded.inviter_pubkey, "alice_pubkey");
        assert_eq!(decoded.pubkey, "bob_pubkey");
        assert_eq!(decoded.ipv6_candidates.len(), 1);
        assert_eq!(decoded.ipv6_candidates[0].to_string(), "2001:db8::1");
        assert_eq!(decoded.external_ip, "203.0.113.5");
        assert_eq!(decoded.external_port, 41641);
        assert_eq!(decoded.wg_port, 41641);
        assert_eq!(decoded.nat_type, NatType::Cone);
        assert_eq!(decoded.observed_stride, 0);
        assert_eq!(decoded.psk, "shared_psk_value");
    }

    #[test]
    fn test_accept_no_ipv6() {
        let token = generate(
            "alice",
            "bob",
            &[],
            "10.0.0.1",
            41642,
            41641,
            NatType::Symmetric,
            4,
            "psk123",
        );

        let decoded = decode(&token).unwrap();
        assert!(decoded.ipv6_candidates.is_empty());
        assert_eq!(decoded.nat_type, NatType::Symmetric);
        assert_eq!(decoded.observed_stride, 4);
    }

    #[test]
    fn test_accept_connection_candidates() {
        let decoded = DecodedAccept {
            inviter_pubkey: "alice".to_string(),
            pubkey: "bob".to_string(),
            ipv6_candidates: vec!["2001:db8::1".parse().unwrap()],
            external_ip: "203.0.113.5".to_string(),
            external_port: 41641,
            wg_port: 41641,
            nat_type: NatType::Cone,
            observed_stride: 0,
            psk: "psk".to_string(),
        };

        let candidates = connection_candidates(&decoded);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], "[2001:db8::1]:41641");
        assert_eq!(candidates[1], "203.0.113.5:41641");
    }

    #[test]
    fn test_accept_invalid_format() {
        assert!(decode("howm://accept/garbage").is_err());
        assert!(decode("howm://invite/something").is_err());
        assert!(decode("not-a-token").is_err());
    }

    #[test]
    fn test_accept_multiple_ipv6() {
        let ipv6 = vec![
            "2001:db8::1".parse::<Ipv6Addr>().unwrap(),
            "2607:f8b0::1".parse::<Ipv6Addr>().unwrap(),
        ];

        let token = generate(
            "alice",
            "bob",
            &ipv6,
            "1.2.3.4",
            41641,
            41641,
            NatType::Cone,
            0,
            "psk",
        );

        let decoded = decode(&token).unwrap();
        assert_eq!(decoded.ipv6_candidates.len(), 2);
    }
}

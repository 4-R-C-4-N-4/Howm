//! Minimal STUN client for NAT characterization.
//!
//! Implements just enough of RFC 5389 to send Binding Requests and parse
//! Binding Responses. Used to discover our external IP:port mapping and
//! classify NAT type (cone vs symmetric).

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;
use tracing::{debug, warn};

// ── STUN constants ──────────────────────────────────────────────────────────

/// STUN magic cookie (RFC 5389 §6)
const MAGIC_COOKIE: u32 = 0x2112A442;

/// STUN message type: Binding Request
const BINDING_REQUEST: u16 = 0x0001;

/// STUN message type: Binding Success Response
const BINDING_RESPONSE: u16 = 0x0101;

/// STUN attribute: XOR-MAPPED-ADDRESS (RFC 5389 §15.2)
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// STUN attribute: MAPPED-ADDRESS (RFC 5389 §15.1, fallback)
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;

/// Address family: IPv4
const FAMILY_IPV4: u8 = 0x01;

/// Address family: IPv6
const FAMILY_IPV6: u8 = 0x02;

// ── Public types ────────────────────────────────────────────────────────────

/// Result of a single STUN binding request.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StunMapping {
    /// Our external IP as seen by the STUN server.
    pub external_ip: std::net::IpAddr,
    /// Our external port as seen by the STUN server.
    pub external_port: u16,
    /// The STUN server we queried.
    pub server: SocketAddr,
}

/// NAT classification based on STUN results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatType {
    /// No NAT — external IP matches local IP.
    Open,
    /// Port-preserving cone NAT — punchable.
    Cone,
    /// Symmetric NAT — port changes per destination, not directly punchable.
    Symmetric,
    /// Detection failed or not run.
    Unknown,
}

impl std::fmt::Display for NatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NatType::Open => write!(f, "open"),
            NatType::Cone => write!(f, "cone"),
            NatType::Symmetric => write!(f, "symmetric"),
            NatType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Cached NAT profile, stored to disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NatProfile {
    pub detected_at: u64,
    pub nat_type: NatType,
    pub external_ip: String,
    pub external_port: u16,
    pub observed_stride: i32,
}

// ── STUN wire protocol ─────────────────────────────────────────────────────

/// Build a STUN Binding Request (20 bytes header, no attributes).
fn build_binding_request(transaction_id: &[u8; 12]) -> [u8; 20] {
    let mut pkt = [0u8; 20];
    // Message type: Binding Request
    pkt[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    // Message length: 0 (no attributes)
    pkt[2..4].copy_from_slice(&0u16.to_be_bytes());
    // Magic cookie
    pkt[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    // Transaction ID (12 bytes)
    pkt[8..20].copy_from_slice(transaction_id);
    pkt
}

/// Parse a STUN Binding Response and extract the mapped address.
fn parse_binding_response(buf: &[u8], expected_txn: &[u8; 12]) -> Option<(std::net::IpAddr, u16)> {
    if buf.len() < 20 {
        return None;
    }

    // Verify message type
    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    if msg_type != BINDING_RESPONSE {
        debug!("STUN: unexpected message type 0x{:04x}", msg_type);
        return None;
    }

    // Verify magic cookie
    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if cookie != MAGIC_COOKIE {
        debug!("STUN: bad magic cookie");
        return None;
    }

    // Verify transaction ID
    if &buf[8..20] != expected_txn {
        debug!("STUN: transaction ID mismatch");
        return None;
    }

    let msg_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if buf.len() < 20 + msg_len {
        return None;
    }

    // Parse attributes
    let mut offset = 20;
    let end = 20 + msg_len;

    while offset + 4 <= end {
        let attr_type = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        let attr_len = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        offset += 4;

        if offset + attr_len > end {
            break;
        }

        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                return parse_xor_mapped_address(&buf[offset..offset + attr_len]);
            }
            ATTR_MAPPED_ADDRESS => {
                // Fallback if XOR-MAPPED-ADDRESS not present
                if let Some(result) = parse_mapped_address(&buf[offset..offset + attr_len]) {
                    return Some(result);
                }
            }
            _ => {}
        }

        // Attributes are padded to 4-byte boundaries
        let padded = (attr_len + 3) & !3;
        offset += padded;
    }

    None
}

/// Parse XOR-MAPPED-ADDRESS attribute value.
fn parse_xor_mapped_address(data: &[u8]) -> Option<(std::net::IpAddr, u16)> {
    if data.len() < 4 {
        return None;
    }

    let family = data[1];
    let xor_port = u16::from_be_bytes([data[2], data[3]]);
    let port = xor_port ^ (MAGIC_COOKIE >> 16) as u16;

    match family {
        FAMILY_IPV4 if data.len() >= 8 => {
            let xor_ip = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let ip = xor_ip ^ MAGIC_COOKIE;
            let addr = std::net::Ipv4Addr::from(ip);
            Some((std::net::IpAddr::V4(addr), port))
        }
        FAMILY_IPV6 if data.len() >= 20 => {
            // XOR with magic cookie + transaction ID (we don't store txn ID here,
            // so for IPv6 we'd need to pass it through. For now, IPv4 is sufficient
            // for NAT detection purposes.)
            debug!("STUN: IPv6 XOR-MAPPED-ADDRESS not implemented");
            None
        }
        _ => None,
    }
}

/// Parse MAPPED-ADDRESS attribute value (non-XOR, RFC 3489 compat).
fn parse_mapped_address(data: &[u8]) -> Option<(std::net::IpAddr, u16)> {
    if data.len() < 4 {
        return None;
    }

    let family = data[1];
    let port = u16::from_be_bytes([data[2], data[3]]);

    match family {
        FAMILY_IPV4 if data.len() >= 8 => {
            let addr = std::net::Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            Some((std::net::IpAddr::V4(addr), port))
        }
        FAMILY_IPV6 if data.len() >= 20 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data[4..20]);
            let addr = std::net::Ipv6Addr::from(octets);
            Some((std::net::IpAddr::V6(addr), port))
        }
        _ => None,
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Well-known public STUN servers.
pub const STUN_SERVERS: &[(&str, u16)] =
    &[("stun.l.google.com", 19302), ("stun.cloudflare.com", 3478)];

/// Perform a single STUN Binding Request from the given local socket.
///
/// Returns the external IP:port as seen by the server, or None on failure.
/// Uses a 3-second timeout with up to 2 retries.
pub fn stun_binding(socket: &UdpSocket, server: SocketAddr) -> Option<StunMapping> {
    use rand::RngCore;

    let mut txn_id = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut txn_id);
    let request = build_binding_request(&txn_id);

    // Set timeout for receive
    socket.set_read_timeout(Some(Duration::from_secs(3))).ok()?;

    for attempt in 0..2 {
        if attempt > 0 {
            debug!("STUN: retry {} to {}", attempt, server);
        }

        if let Err(e) = socket.send_to(&request, server) {
            debug!("STUN: send to {} failed: {}", server, e);
            continue;
        }

        let mut buf = [0u8; 576]; // STUN responses are small
        match socket.recv_from(&mut buf) {
            Ok((len, from)) => {
                if from.ip() != server.ip() {
                    debug!("STUN: response from unexpected source {}", from);
                    continue;
                }
                if let Some((ip, port)) = parse_binding_response(&buf[..len], &txn_id) {
                    return Some(StunMapping {
                        external_ip: ip,
                        external_port: port,
                        server,
                    });
                }
            }
            Err(e) => {
                debug!("STUN: recv from {} failed: {}", server, e);
            }
        }
    }

    None
}

/// Resolve a STUN server hostname to a SocketAddr.
pub fn resolve_stun_server(host: &str, port: u16) -> Option<SocketAddr> {
    use std::net::ToSocketAddrs;
    format!("{}:{}", host, port)
        .to_socket_addrs()
        .ok()?
        .find(|a| a.is_ipv4()) // Prefer IPv4 for NAT detection
}

/// Run the NAT characterization battery.
///
/// Sends STUN binding requests to two different servers and compares
/// the external port mappings to classify the NAT type.
///
/// `local_port` is the port to bind locally (typically the WG listen port).
pub fn characterize_nat(local_port: u16) -> NatProfile {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let default_profile = NatProfile {
        detected_at: now,
        nat_type: NatType::Unknown,
        external_ip: String::new(),
        external_port: 0,
        observed_stride: 0,
    };

    // Bind a UDP socket on the local port
    let bind_addr: SocketAddr = format!("0.0.0.0:{}", local_port).parse().unwrap();
    let socket = match UdpSocket::bind(bind_addr) {
        Ok(s) => s,
        Err(e) => {
            warn!("NAT detection: failed to bind port {}: {}", local_port, e);
            // Try binding to any port as fallback
            match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(e2) => {
                    warn!("NAT detection: fallback bind also failed: {}", e2);
                    return default_profile;
                }
            }
        }
    };

    // Resolve STUN servers
    let server_a = STUN_SERVERS
        .iter()
        .find_map(|(host, port)| resolve_stun_server(host, *port));
    let server_b = STUN_SERVERS
        .iter()
        .rev()
        .find_map(|(host, port)| resolve_stun_server(host, *port));

    let (server_a, server_b) = match (server_a, server_b) {
        (Some(a), Some(b)) if a != b => (a, b),
        _ => {
            warn!("NAT detection: could not resolve two distinct STUN servers");
            return default_profile;
        }
    };

    // Test 1: Baseline mapping from server A
    let mapping_a = match stun_binding(&socket, server_a) {
        Some(m) => m,
        None => {
            warn!("NAT detection: STUN server A ({}) failed", server_a);
            return default_profile;
        }
    };
    debug!(
        "NAT detection: server A reports {}:{}",
        mapping_a.external_ip, mapping_a.external_port
    );

    // Check for OPEN (no NAT) — external IP matches a local address
    let local_ip = socket.local_addr().map(|a| a.ip()).ok();
    if Some(mapping_a.external_ip) == local_ip {
        return NatProfile {
            detected_at: now,
            nat_type: NatType::Open,
            external_ip: mapping_a.external_ip.to_string(),
            external_port: mapping_a.external_port,
            observed_stride: 0,
        };
    }

    // Test 2: Symmetric check from server B
    let mapping_b = match stun_binding(&socket, server_b) {
        Some(m) => m,
        None => {
            warn!("NAT detection: STUN server B ({}) failed", server_b);
            // Got A but not B — assume cone (conservative)
            return NatProfile {
                detected_at: now,
                nat_type: NatType::Cone,
                external_ip: mapping_a.external_ip.to_string(),
                external_port: mapping_a.external_port,
                observed_stride: 0,
            };
        }
    };
    debug!(
        "NAT detection: server B reports {}:{}",
        mapping_b.external_ip, mapping_b.external_port
    );

    // Compare ports
    let stride = mapping_b.external_port as i32 - mapping_a.external_port as i32;

    if mapping_a.external_port == mapping_b.external_port {
        // Same external port for different destinations → cone NAT
        NatProfile {
            detected_at: now,
            nat_type: NatType::Cone,
            external_ip: mapping_a.external_ip.to_string(),
            external_port: mapping_a.external_port,
            observed_stride: 0,
        }
    } else {
        // Different external ports → symmetric NAT
        NatProfile {
            detected_at: now,
            nat_type: NatType::Symmetric,
            external_ip: mapping_a.external_ip.to_string(),
            external_port: mapping_a.external_port,
            observed_stride: stride,
        }
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Load cached NAT profile from disk.
pub fn load_nat_profile(data_dir: &std::path::Path) -> Option<NatProfile> {
    let path = data_dir.join("nat_profile.json");
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Save NAT profile to disk.
pub fn save_nat_profile(data_dir: &std::path::Path, profile: &NatProfile) -> anyhow::Result<()> {
    let path = data_dir.join("nat_profile.json");
    let tmp = data_dir.join("nat_profile.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(profile)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Run a fresh STUN binding to get the current external port mapping.
/// Uses cached NAT type but refreshes the mapping. Returns updated profile.
pub fn refresh_mapping(data_dir: &std::path::Path, local_port: u16) -> NatProfile {
    let existing = load_nat_profile(data_dir);
    let profile = characterize_nat(local_port);

    // If we had a previous profile, preserve the NAT type if the new detection
    // failed (UNKNOWN). The cached type is still useful.
    if profile.nat_type == NatType::Unknown {
        if let Some(existing) = existing {
            return existing;
        }
    }

    if let Err(e) = save_nat_profile(data_dir, &profile) {
        warn!("Failed to save NAT profile: {}", e);
    }
    profile
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_binding_request() {
        let txn = [1u8; 12];
        let pkt = build_binding_request(&txn);
        assert_eq!(pkt.len(), 20);
        // Message type
        assert_eq!(u16::from_be_bytes([pkt[0], pkt[1]]), BINDING_REQUEST);
        // Message length
        assert_eq!(u16::from_be_bytes([pkt[2], pkt[3]]), 0);
        // Magic cookie
        assert_eq!(
            u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]),
            MAGIC_COOKIE
        );
        // Transaction ID
        assert_eq!(&pkt[8..20], &txn);
    }

    #[test]
    fn test_parse_xor_mapped_address_ipv4() {
        // Build a fake XOR-MAPPED-ADDRESS for 203.0.113.5:41641
        let ip = std::net::Ipv4Addr::new(203, 0, 113, 5);
        let port: u16 = 41641;

        let xor_port = port ^ (MAGIC_COOKIE >> 16) as u16;
        let xor_ip = u32::from_be_bytes(ip.octets()) ^ MAGIC_COOKIE;

        let mut data = [0u8; 8];
        data[0] = 0; // reserved
        data[1] = FAMILY_IPV4;
        data[2..4].copy_from_slice(&xor_port.to_be_bytes());
        data[4..8].copy_from_slice(&xor_ip.to_be_bytes());

        let (parsed_ip, parsed_port) = parse_xor_mapped_address(&data).unwrap();
        assert_eq!(parsed_ip, std::net::IpAddr::V4(ip));
        assert_eq!(parsed_port, port);
    }

    #[test]
    fn test_parse_binding_response_full() {
        // Build a complete STUN Binding Response with XOR-MAPPED-ADDRESS
        let txn = [42u8; 12];
        let ip = std::net::Ipv4Addr::new(198, 51, 100, 1);
        let port: u16 = 12345;

        let xor_port = port ^ (MAGIC_COOKIE >> 16) as u16;
        let xor_ip = u32::from_be_bytes(ip.octets()) ^ MAGIC_COOKIE;

        // Attribute: XOR-MAPPED-ADDRESS
        let mut attr = Vec::new();
        attr.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes()); // type
        attr.extend_from_slice(&8u16.to_be_bytes()); // length
        attr.push(0); // reserved
        attr.push(FAMILY_IPV4);
        attr.extend_from_slice(&xor_port.to_be_bytes());
        attr.extend_from_slice(&xor_ip.to_be_bytes());

        // Header
        let mut pkt = Vec::new();
        pkt.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        pkt.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        pkt.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        pkt.extend_from_slice(&txn);
        pkt.extend_from_slice(&attr);

        let (parsed_ip, parsed_port) = parse_binding_response(&pkt, &txn).unwrap();
        assert_eq!(parsed_ip, std::net::IpAddr::V4(ip));
        assert_eq!(parsed_port, port);
    }

    #[test]
    fn test_parse_binding_response_wrong_txn() {
        let txn = [42u8; 12];
        let wrong_txn = [99u8; 12];

        let mut pkt = vec![0u8; 20];
        pkt[0..2].copy_from_slice(&BINDING_RESPONSE.to_be_bytes());
        pkt[2..4].copy_from_slice(&0u16.to_be_bytes());
        pkt[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
        pkt[8..20].copy_from_slice(&wrong_txn);

        assert!(parse_binding_response(&pkt, &txn).is_none());
    }

    #[test]
    fn test_parse_mapped_address_ipv4() {
        let mut data = [0u8; 8];
        data[1] = FAMILY_IPV4;
        data[2..4].copy_from_slice(&8080u16.to_be_bytes());
        data[4] = 10;
        data[5] = 0;
        data[6] = 0;
        data[7] = 1;

        let (ip, port) = parse_mapped_address(&data).unwrap();
        assert_eq!(
            ip,
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(port, 8080);
    }

    #[test]
    fn test_nat_type_display() {
        assert_eq!(NatType::Open.to_string(), "open");
        assert_eq!(NatType::Cone.to_string(), "cone");
        assert_eq!(NatType::Symmetric.to_string(), "symmetric");
        assert_eq!(NatType::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_nat_type_serde() {
        let json = serde_json::to_string(&NatType::Cone).unwrap();
        assert_eq!(json, "\"cone\"");
        let parsed: NatType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, NatType::Cone);
    }

    #[test]
    fn test_nat_profile_serde_roundtrip() {
        let profile = NatProfile {
            detected_at: 1700000000,
            nat_type: NatType::Cone,
            external_ip: "203.0.113.5".to_string(),
            external_port: 41641,
            observed_stride: 0,
        };
        let json = serde_json::to_string_pretty(&profile).unwrap();
        let parsed: NatProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nat_type, NatType::Cone);
        assert_eq!(parsed.external_port, 41641);
        assert_eq!(parsed.observed_stride, 0);
    }

    #[test]
    fn test_save_and_load_nat_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profile = NatProfile {
            detected_at: 1700000000,
            nat_type: NatType::Symmetric,
            external_ip: "198.51.100.1".to_string(),
            external_port: 41642,
            observed_stride: 2,
        };
        save_nat_profile(dir.path(), &profile).unwrap();

        let loaded = load_nat_profile(dir.path()).unwrap();
        assert_eq!(loaded.nat_type, NatType::Symmetric);
        assert_eq!(loaded.external_ip, "198.51.100.1");
        assert_eq!(loaded.external_port, 41642);
        assert_eq!(loaded.observed_stride, 2);
    }
}

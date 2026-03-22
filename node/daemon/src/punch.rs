//! NAT hole punching via WireGuard handshake endpoint rotation.
//!
//! When both peers are behind cone NAT, neither can receive unsolicited
//! inbound connections. The hole punch works by:
//! 1. Both peers exchange endpoint info out-of-band (howm://accept/ token)
//! 2. Both configure WG peers with best-guess endpoints
//! 3. Both rotate through candidate ports until WG handshake succeeds
//!
//! There is no custom probe protocol — WireGuard's handshake IS the probe.

use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::stun::NatType;

/// Configuration for a hole punch attempt.
#[derive(Debug, Clone)]
pub struct PunchConfig {
    /// Peer's WG public key (base64).
    pub peer_pubkey: String,
    /// Peer's external IP as reported by STUN.
    pub peer_external_ip: String,
    /// Peer's STUN-reflected external port.
    pub peer_external_port: u16,
    /// Peer's observed port allocation stride (0 = port-preserving).
    pub peer_stride: i32,
    /// Peer's actual WG listen port.
    pub peer_wg_port: u16,
    /// Peer's NAT type.
    pub peer_nat_type: NatType,
    /// Our NAT type.
    pub our_nat_type: NatType,
    /// PSK for the WG peer (optional).
    pub psk: Option<String>,
    /// WG address to assign to this peer.
    pub allowed_ip: String,
    /// Whether we should initiate first (true for cone vs symmetric).
    pub we_initiate: bool,
}

/// Result of a hole punch attempt.
#[derive(Debug)]
pub enum PunchResult {
    /// WG handshake succeeded on this endpoint.
    Success { endpoint: String, elapsed: Duration },
    /// All candidates exhausted or timeout reached.
    Timeout { elapsed: Duration },
    /// Error during punch attempt.
    Error(String),
}

/// Build the list of candidate endpoints to try, in priority order.
///
/// 1. STUN-reflected port (most likely for port-preserving NATs)
/// 2. Actual WG listen port (if different from STUN-reflected)
/// 3. Stride offsets: base ± stride, base ± 2*stride, ...
/// 4. Sequential neighbors: base ± 1..10
pub fn build_candidates(config: &PunchConfig) -> Vec<String> {
    let base = config.peer_external_port;
    let ip = &config.peer_external_ip;
    let stride = config.peer_stride;

    let mut ports: Vec<u16> = Vec::with_capacity(32);

    // 1. STUN-reflected port first (port preservation)
    ports.push(base);

    // 2. WG listen port if different
    if config.peer_wg_port != base {
        ports.push(config.peer_wg_port);
    }

    // 3. Stride offsets
    if stride != 0 {
        for i in 1..=5i32 {
            if let Some(p) = base.checked_add_signed(stride.wrapping_mul(i) as i16) {
                ports.push(p);
            }
            if let Some(p) = base.checked_sub((stride.wrapping_mul(i)).unsigned_abs() as u16) {
                ports.push(p);
            }
        }
    }

    // 4. Sequential neighbors ±1..10
    for offset in 1..=10u16 {
        if let Some(p) = base.checked_add(offset) {
            ports.push(p);
        }
        if let Some(p) = base.checked_sub(offset) {
            ports.push(p);
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    ports.retain(|p| seen.insert(*p));

    // Filter out port 0
    ports.retain(|p| *p > 0);

    // Format as IP:port endpoints
    ports.into_iter().map(|p| format!("{}:{}", ip, p)).collect()
}

/// Run the hole punch — rotate WG endpoint through candidates until
/// handshake succeeds or timeout.
///
/// This is the core loop: set endpoint → wait → check handshake → rotate.
///
/// `data_dir` is used for PSK temp file management.
/// `wg_iface` is the WireGuard interface name (e.g., "howm0").
pub async fn run_punch(
    config: &PunchConfig,
    data_dir: &std::path::Path,
    wg_iface: &str,
    timeout: Duration,
) -> PunchResult {
    let candidates = build_candidates(config);
    if candidates.is_empty() {
        return PunchResult::Error("no candidate endpoints".to_string());
    }

    info!(
        "Starting hole punch to {} ({} candidates, timeout {}s)",
        config.peer_pubkey[..8.min(config.peer_pubkey.len())].to_string(),
        candidates.len(),
        timeout.as_secs(),
    );

    // First, ensure the WG peer is configured (without endpoint — we'll set it in the loop)
    if let Err(e) = add_peer_for_punch(config, data_dir, wg_iface).await {
        return PunchResult::Error(format!("failed to add WG peer: {}", e));
    }

    let start = Instant::now();
    let interval = if config.we_initiate {
        Duration::from_secs(1) // Initiator: slower, keep mappings alive
    } else {
        Duration::from_millis(200) // Responder: fast rotation
    };

    // Cycle through candidates
    let mut attempt = 0;
    loop {
        let candidate = &candidates[attempt % candidates.len()];

        // Set the WG peer endpoint
        if let Err(e) = set_peer_endpoint(wg_iface, &config.peer_pubkey, candidate).await {
            debug!("Punch: failed to set endpoint {}: {}", candidate, e);
            // Continue to next candidate
        } else {
            debug!("Punch: trying endpoint {} (attempt {})", candidate, attempt);
        }

        // Wait for the interval
        tokio::time::sleep(interval).await;

        // Check if WG handshake has completed
        match check_handshake(wg_iface, &config.peer_pubkey).await {
            Ok(true) => {
                let elapsed = start.elapsed();
                info!(
                    "Hole punch succeeded on {} after {:.1}s ({} attempts)",
                    candidate,
                    elapsed.as_secs_f64(),
                    attempt + 1,
                );
                return PunchResult::Success {
                    endpoint: candidate.clone(),
                    elapsed,
                };
            }
            Ok(false) => {} // No handshake yet
            Err(e) => {
                debug!("Punch: handshake check failed: {}", e);
            }
        }

        // Check timeout
        if start.elapsed() >= timeout {
            return PunchResult::Timeout {
                elapsed: start.elapsed(),
            };
        }

        attempt += 1;
    }
}

/// Add a WG peer configured for punching (no endpoint initially, just pubkey + allowed-ips + psk).
async fn add_peer_for_punch(
    config: &PunchConfig,
    data_dir: &std::path::Path,
    wg_iface: &str,
) -> anyhow::Result<()> {
    let wg_dir = data_dir.join("wireguard");
    let mut args: Vec<String> = vec![
        "set".to_string(),
        wg_iface.to_string(),
        "peer".to_string(),
        config.peer_pubkey.clone(),
        "allowed-ips".to_string(),
        format!("{}/32", config.allowed_ip),
        "persistent-keepalive".to_string(),
        "25".to_string(),
    ];

    // Handle PSK
    let psk_file = if let Some(ref psk) = config.psk {
        let path = wg_dir.join("psk_punch.tmp");
        std::fs::create_dir_all(&wg_dir)?;
        std::fs::write(&path, psk)?;
        args.push("preshared-key".to_string());
        args.push(path.to_str().unwrap().to_string());
        Some(path)
    } else {
        None
    };

    let output = tokio::process::Command::new("wg")
        .args(&args)
        .output()
        .await?;

    if let Some(path) = psk_file {
        let _ = std::fs::remove_file(&path);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("wg set peer failed: {}", stderr.trim()));
    }

    Ok(())
}

/// Set the endpoint for a WG peer.
async fn set_peer_endpoint(wg_iface: &str, pubkey: &str, endpoint: &str) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("wg")
        .args(["set", wg_iface, "peer", pubkey, "endpoint", endpoint])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("wg set endpoint failed: {}", stderr.trim()));
    }

    Ok(())
}

/// Check if a WG handshake has completed for a given peer.
/// Returns true if there's a recent handshake (within last 180 seconds).
async fn check_handshake(wg_iface: &str, pubkey: &str) -> anyhow::Result<bool> {
    let output = tokio::process::Command::new("wg")
        .args(["show", wg_iface, "dump"])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("wg show failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for line in stdout.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 5 && fields[0] == pubkey {
            let handshake: u64 = fields[4].parse().unwrap_or(0);
            if handshake > 0 && now.saturating_sub(handshake) < 180 {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Public convenience wrapper: check if a WG handshake has completed for a peer.
/// Used by the accept redemption flow for IPv6 direct connection attempts.
pub async fn check_handshake_by_status(pubkey: &str) -> bool {
    check_handshake("howm0", pubkey).await.unwrap_or(false)
}

/// Determine if we should initiate first in a punch scenario.
/// Cone peer initiates against symmetric peer (cone's mapping is predictable).
pub fn should_we_initiate(our_nat: NatType, their_nat: NatType) -> bool {
    match (our_nat, their_nat) {
        (NatType::Cone, NatType::Symmetric) => true,
        (NatType::Symmetric, NatType::Cone) => false,
        _ => false, // Both cone: both probe simultaneously
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_candidates_port_preserving() {
        let config = PunchConfig {
            peer_pubkey: "test".to_string(),
            peer_external_ip: "203.0.113.5".to_string(),
            peer_external_port: 41641,
            peer_stride: 0,
            peer_wg_port: 41641,
            peer_nat_type: NatType::Cone,
            our_nat_type: NatType::Cone,
            psk: None,
            allowed_ip: "100.222.0.2".to_string(),
            we_initiate: false,
        };

        let candidates = build_candidates(&config);
        // First candidate should be the STUN-reflected port
        assert_eq!(candidates[0], "203.0.113.5:41641");
        // Should have sequential neighbors
        assert!(candidates.contains(&"203.0.113.5:41642".to_string()));
        assert!(candidates.contains(&"203.0.113.5:41640".to_string()));
        // No duplicates
        let unique: std::collections::HashSet<_> = candidates.iter().collect();
        assert_eq!(unique.len(), candidates.len());
    }

    #[test]
    fn test_build_candidates_with_stride() {
        let config = PunchConfig {
            peer_pubkey: "test".to_string(),
            peer_external_ip: "198.51.100.1".to_string(),
            peer_external_port: 41641,
            peer_stride: 4,
            peer_wg_port: 41641,
            peer_nat_type: NatType::Cone,
            our_nat_type: NatType::Cone,
            psk: None,
            allowed_ip: "100.222.0.2".to_string(),
            we_initiate: false,
        };

        let candidates = build_candidates(&config);
        assert_eq!(candidates[0], "198.51.100.1:41641");
        // Stride offsets should be present
        assert!(candidates.contains(&"198.51.100.1:41645".to_string())); // +4
        assert!(candidates.contains(&"198.51.100.1:41637".to_string())); // -4
    }

    #[test]
    fn test_build_candidates_different_wg_port() {
        let config = PunchConfig {
            peer_pubkey: "test".to_string(),
            peer_external_ip: "10.0.0.1".to_string(),
            peer_external_port: 41645, // STUN reflected
            peer_stride: 0,
            peer_wg_port: 41641, // Actual WG port (different due to NAT)
            peer_nat_type: NatType::Cone,
            our_nat_type: NatType::Cone,
            psk: None,
            allowed_ip: "100.222.0.2".to_string(),
            we_initiate: false,
        };

        let candidates = build_candidates(&config);
        // STUN port first, then WG port
        assert_eq!(candidates[0], "10.0.0.1:41645");
        assert_eq!(candidates[1], "10.0.0.1:41641");
    }

    #[test]
    fn test_should_we_initiate() {
        // Cone vs symmetric: cone initiates
        assert!(should_we_initiate(NatType::Cone, NatType::Symmetric));
        assert!(!should_we_initiate(NatType::Symmetric, NatType::Cone));

        // Cone vs cone: neither specifically initiates (both probe)
        assert!(!should_we_initiate(NatType::Cone, NatType::Cone));

        // Other combinations
        assert!(!should_we_initiate(NatType::Open, NatType::Cone));
        assert!(!should_we_initiate(NatType::Unknown, NatType::Unknown));
    }

    #[test]
    fn test_build_candidates_no_duplicates() {
        let config = PunchConfig {
            peer_pubkey: "test".to_string(),
            peer_external_ip: "1.2.3.4".to_string(),
            peer_external_port: 41641,
            peer_stride: 1, // stride of 1 overlaps with sequential neighbors
            peer_wg_port: 41641,
            peer_nat_type: NatType::Cone,
            our_nat_type: NatType::Cone,
            psk: None,
            allowed_ip: "100.222.0.2".to_string(),
            we_initiate: false,
        };

        let candidates = build_candidates(&config);
        let unique: std::collections::HashSet<_> = candidates.iter().collect();
        assert_eq!(
            unique.len(),
            candidates.len(),
            "candidates contain duplicates"
        );
    }
}

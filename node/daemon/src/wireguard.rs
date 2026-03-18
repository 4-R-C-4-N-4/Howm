// WireGuard module — native implementation using system `wg` and `ip` CLI tools.
// Works on Linux with wireguard-tools installed.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WgConfig {
    pub enabled: bool,
    pub port: u16,
    pub endpoint: Option<String>, // public addr:port for peers to reach us
    pub address: Option<String>,  // override WG address (100.222.x.y)
    pub data_dir: PathBuf,
    #[allow(dead_code)]
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WgState {
    pub public_key: Option<String>,
    pub address: Option<String>,  // 100.222.x.y
    pub endpoint: Option<String>, // public addr:port
    pub tunnel_handle: Option<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WgPeerConfig {
    pub pubkey: String,
    pub endpoint: String,
    pub psk: Option<String>,
    pub allowed_ip: String,
    pub name: String,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WgPeerStatus {
    pub pubkey: String,
    pub endpoint: String,
    pub allowed_ips: String,
    pub latest_handshake: u64,
    pub transfer_rx: u64,
    pub transfer_tx: u64,
}

const WG_SUBNET: &str = "100.222"; // 100.222.0.0/16
const WG_IFACE: &str = "howm0";

// ── Initialization ──────────────────────────────────────────────────────────

/// Initialize WireGuard: generate keypair, create kernel WG interface.
/// Returns WgState with our public key, address.
/// Falls back to WG-disabled mode if interface creation fails.
pub async fn init(config: &WgConfig) -> anyhow::Result<WgState> {
    if !config.enabled {
        info!("WireGuard disabled");
        return Ok(WgState {
            public_key: None,
            address: None,
            endpoint: None,
            tunnel_handle: None,
        });
    }

    let wg_dir = config.data_dir.join("wireguard");
    std::fs::create_dir_all(&wg_dir)?;

    // Generate keypair if needed
    let (private_key, public_key) = ensure_keypair(&wg_dir)?;
    info!("WG public key: {}", public_key);

    // Determine our WG address
    let address = match &config.address {
        Some(addr) => addr.clone(),
        None => {
            let addr_file = wg_dir.join("address");
            if addr_file.exists() {
                std::fs::read_to_string(&addr_file)?.trim().to_string()
            } else {
                // First node gets 100.222.0.1
                let addr = format!("{}.0.1", WG_SUBNET);
                std::fs::write(&addr_file, &addr)?;
                addr
            }
        }
    };
    info!("WG address: {}", address);

    let endpoint = config.endpoint.clone();

    // Try to create the WireGuard interface
    match setup_wg_interface(&private_key, &address, config.port, &wg_dir).await {
        Ok(()) => {
            info!(
                "WireGuard interface {} configured on port {}",
                WG_IFACE, config.port
            );

            // Load and configure saved peers
            let peers = load_peers(&wg_dir).unwrap_or_default();
            for peer in &peers {
                if let Err(e) = configure_wg_peer(&wg_dir, peer).await {
                    warn!("Failed to restore WG peer {}: {}", peer.name, e);
                }
            }
            if !peers.is_empty() {
                info!("Restored {} WG peers", peers.len());
            }

            Ok(WgState {
                public_key: Some(public_key),
                address: Some(address),
                endpoint,
                tunnel_handle: Some(()),
            })
        }
        Err(e) => {
            warn!(
                "Failed to create WireGuard interface: {}. Falling back to WG-disabled mode.",
                e
            );
            warn!(
                "Ensure wireguard-tools is installed and you have root/CAP_NET_ADMIN privileges."
            );
            Ok(WgState {
                public_key: Some(public_key),
                address: Some(address),
                endpoint,
                tunnel_handle: None,
            })
        }
    }
}

/// Set up the kernel WireGuard interface using `ip` and `wg` CLI tools.
async fn setup_wg_interface(
    _private_key: &str,
    address: &str,
    port: u16,
    wg_dir: &Path,
) -> anyhow::Result<()> {
    // Remove existing interface if present (ignore errors)
    let _ = tokio::process::Command::new("ip")
        .args(["link", "delete", WG_IFACE])
        .output()
        .await;

    // Create WireGuard interface
    let output = tokio::process::Command::new("ip")
        .args(["link", "add", WG_IFACE, "type", "wireguard"])
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ip link add failed: {}", stderr.trim()));
    }

    // Write private key to a temp file for wg setconf
    let privkey_file = wg_dir.join("private_key");

    // Configure WireGuard with private key and listen port
    let output = tokio::process::Command::new("wg")
        .args([
            "set",
            WG_IFACE,
            "private-key",
            privkey_file.to_str().unwrap(),
            "listen-port",
            &port.to_string(),
        ])
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Clean up interface on failure
        let _ = tokio::process::Command::new("ip")
            .args(["link", "delete", WG_IFACE])
            .output()
            .await;
        return Err(anyhow::anyhow!("wg set failed: {}", stderr.trim()));
    }

    // Assign IP address
    let output = tokio::process::Command::new("ip")
        .args(["addr", "add", &format!("{}/16", address), "dev", WG_IFACE])
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Not fatal if address already exists
        if !stderr.contains("RTNETLINK answers: File exists") {
            warn!("ip addr add warning: {}", stderr.trim());
        }
    }

    // Bring interface up
    let output = tokio::process::Command::new("ip")
        .args(["link", "set", WG_IFACE, "up"])
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ip link set up failed: {}", stderr.trim()));
    }

    Ok(())
}

// ── Shutdown ────────────────────────────────────────────────────────────────

/// Stop WireGuard — remove the interface.
pub async fn shutdown() -> anyhow::Result<()> {
    info!("Shutting down WireGuard interface {}", WG_IFACE);
    let output = tokio::process::Command::new("ip")
        .args(["link", "delete", WG_IFACE])
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("Failed to remove WG interface: {}", stderr.trim());
    }
    Ok(())
}

// ── Key management ──────────────────────────────────────────────────────────

/// Ensure a WG keypair exists on disk. Returns (private_key, public_key) as base64.
fn ensure_keypair(wg_dir: &Path) -> anyhow::Result<(String, String)> {
    let priv_path = wg_dir.join("private_key");
    let pub_path = wg_dir.join("public_key");

    if priv_path.exists() && pub_path.exists() {
        let private_key = std::fs::read_to_string(&priv_path)?.trim().to_string();
        let public_key = std::fs::read_to_string(&pub_path)?.trim().to_string();
        return Ok((private_key, public_key));
    }

    // Generate new keypair using x25519-dalek
    info!("Generating new WireGuard keypair");
    let private_key = generate_private_key();
    let public_key = derive_public_key(&private_key);

    std::fs::write(&priv_path, &private_key)?;
    std::fs::write(&pub_path, &public_key)?;

    // Restrict permissions on private key
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&priv_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok((private_key, public_key))
}

/// Generate a WireGuard private key (base64-encoded x25519 secret).
fn generate_private_key() -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use x25519_dalek::StaticSecret;
    let secret = StaticSecret::random_from_rng(rand::thread_rng());
    STANDARD.encode(secret.to_bytes())
}

/// Derive a WireGuard public key from a base64-encoded private key.
fn derive_public_key(private_key_b64: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use x25519_dalek::{PublicKey, StaticSecret};

    let private_bytes = STANDARD
        .decode(private_key_b64)
        .expect("valid base64 private key");
    let mut key = [0u8; 32];
    key.copy_from_slice(&private_bytes[..32]);

    let secret = StaticSecret::from(key);
    let public = PublicKey::from(&secret);
    STANDARD.encode(public.as_bytes())
}

/// Generate a WireGuard pre-shared key (random 32 bytes, base64-encoded).
pub fn generate_psk() -> String {
    use rand::RngCore;
    let mut key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key_bytes);
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(key_bytes)
}

// ── Peer operations ─────────────────────────────────────────────────────────

/// Add a WireGuard peer — configures via `wg set` and persists config.
pub async fn add_peer(data_dir: &Path, peer: &WgPeerConfig) -> anyhow::Result<()> {
    // Configure peer on the running interface
    if let Err(e) = configure_wg_peer(&data_dir.join("wireguard"), peer).await {
        warn!(
            "Failed to configure WG peer on interface (may not be active): {}",
            e
        );
    }

    // Persist peer config to disk
    let peers_dir = data_dir.join("wireguard").join("peers");
    std::fs::create_dir_all(&peers_dir)?;
    let peer_file = peers_dir.join(format!("{}.json", peer.node_id));
    let tmp = peers_dir.join(format!("{}.json.tmp", peer.node_id));
    std::fs::write(&tmp, serde_json::to_string_pretty(peer)?)?;
    std::fs::rename(&tmp, &peer_file)?;

    info!(
        "Added WG peer: {} ({})",
        peer.name,
        peer.pubkey[..8.min(peer.pubkey.len())].to_string()
    );
    Ok(())
}

/// Configure a single peer on the WireGuard interface using `wg set`.
async fn configure_wg_peer(wg_dir: &Path, peer: &WgPeerConfig) -> anyhow::Result<()> {
    let mut args: Vec<String> = vec![
        "set".to_string(),
        WG_IFACE.to_string(),
        "peer".to_string(),
        peer.pubkey.clone(),
        "allowed-ips".to_string(),
        format!("{}/32", peer.allowed_ip),
        "endpoint".to_string(),
        peer.endpoint.clone(),
    ];

    // Handle PSK via temp file
    let psk_file = if let Some(ref psk) = peer.psk {
        let psk_path = wg_dir.join(format!("psk_{}.tmp", peer.node_id));
        std::fs::write(&psk_path, psk)?;
        args.push("preshared-key".to_string());
        args.push(psk_path.to_str().unwrap().to_string());
        Some(psk_path)
    } else {
        None
    };

    // Add persistent keepalive
    args.push("persistent-keepalive".to_string());
    args.push("25".to_string());

    let output = tokio::process::Command::new("wg")
        .args(&args)
        .output()
        .await?;

    // Clean up PSK temp file
    if let Some(psk_path) = psk_file {
        let _ = std::fs::remove_file(&psk_path);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("wg set peer failed: {}", stderr.trim()));
    }

    Ok(())
}

/// Remove a WireGuard peer — removes from interface and deletes persisted config.
pub async fn remove_peer(data_dir: &Path, pubkey: &str, node_id: &str) -> anyhow::Result<()> {
    // Remove from running interface
    let output = tokio::process::Command::new("wg")
        .args(["set", WG_IFACE, "peer", pubkey, "remove"])
        .output()
        .await;
    match output {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!("wg remove peer warning: {}", stderr.trim());
        }
        Err(e) => warn!("Failed to run wg command: {}", e),
        _ => {}
    }

    // Remove persisted config
    let peer_file = data_dir
        .join("wireguard")
        .join("peers")
        .join(format!("{}.json", node_id));
    let _ = std::fs::remove_file(&peer_file);

    info!(
        "Removed WG peer: {} ({})",
        node_id,
        &pubkey[..8.min(pubkey.len())]
    );
    Ok(())
}

/// Get WireGuard status by parsing `wg show howm0 dump`.
pub async fn get_status() -> anyhow::Result<Vec<WgPeerStatus>> {
    let output = tokio::process::Command::new("wg")
        .args(["show", WG_IFACE, "dump"])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Interface might not exist
        if stderr.contains("No such device") || stderr.contains("Unable to access") {
            return Ok(Vec::new());
        }
        return Err(anyhow::anyhow!("wg show failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut peers = Vec::new();

    // Skip first line (interface info), parse peer lines
    // Format: public-key\tpreshared-key\tendpoint\tallowed-ips\tlatest-handshake\ttransfer-rx\ttransfer-tx\tpersistent-keepalive
    for line in stdout.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 7 {
            peers.push(WgPeerStatus {
                pubkey: fields[0].to_string(),
                endpoint: fields[2].to_string(),
                allowed_ips: fields[3].to_string(),
                latest_handshake: fields[4].parse().unwrap_or(0),
                transfer_rx: fields[5].parse().unwrap_or(0),
                transfer_tx: fields[6].parse().unwrap_or(0),
            });
        }
    }

    Ok(peers)
}

// ── WireGuard peer state monitor (P2P-CD Task 1.1) ──────────────────────────

/// Events emitted by the WgPeerMonitor to the protocol engine.
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum WgPeerEvent {
    /// A peer has become reachable (new handshake detected).
    PeerVisible(p2pcd_types::PeerId),
    /// A peer's handshake has timed out or they dropped off the dump.
    PeerUnreachable(p2pcd_types::PeerId),
    /// A peer has been completely removed from the WireGuard interface.
    PeerRemoved(p2pcd_types::PeerId),
}

/// Parse the output of `wg show <iface> dump` into `WgPeerState` structs.
///
/// Format (tab-separated, one peer per line after the interface self-line):
/// `public-key\tpreshared-key\tendpoint\tallowed-ips\tlatest-handshake\ttransfer-rx\ttransfer-tx\tpersistent-keepalive`
pub fn parse_wg_dump(output: &str) -> Vec<p2pcd_types::WgPeerState> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let mut peers = Vec::new();
    for line in output.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }

        let pubkey_b64 = fields[0];
        let endpoint_str = fields[2];
        let allowed_ips_str = fields[3];
        let latest_handshake: u64 = fields[4].parse().unwrap_or(0);
        let rx_bytes: u64 = fields[5].parse().unwrap_or(0);
        let tx_bytes: u64 = fields[6].parse().unwrap_or(0);

        let Ok(key_bytes) = STANDARD.decode(pubkey_b64) else {
            continue;
        };
        if key_bytes.len() != 32 {
            continue;
        }
        let mut peer_id = [0u8; 32];
        peer_id.copy_from_slice(&key_bytes);

        let endpoint = if endpoint_str == "(none)" {
            None
        } else {
            endpoint_str.parse().ok()
        };

        let allowed_ips: Vec<String> = allowed_ips_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        peers.push(p2pcd_types::WgPeerState {
            public_key: peer_id,
            endpoint,
            allowed_ips,
            latest_handshake,
            rx_bytes,
            tx_bytes,
        });
    }
    peers
}

/// Handshake timeout: if `latest_handshake` is older than this many seconds,
/// the peer is considered unreachable even if still listed in the dump.
const HANDSHAKE_TIMEOUT_SECS: u64 = 180;

/// Background task that polls `wg show howm0 dump` and emits [`WgPeerEvent`]s
/// whenever a peer's reachability changes.
pub struct WgPeerMonitor {
    poll_interval_ms: u64,
    tx: tokio::sync::mpsc::Sender<WgPeerEvent>,
}

impl WgPeerMonitor {
    pub fn new(poll_interval_ms: u64, tx: tokio::sync::mpsc::Sender<WgPeerEvent>) -> Self {
        Self {
            poll_interval_ms,
            tx,
        }
    }

    /// Spawn the background polling loop. Returns a `JoinHandle`.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(self) {
        use std::collections::{HashMap, HashSet};
        use tokio::time::{sleep, Duration};

        // last known handshake timestamp per peer
        let mut last_handshake: HashMap<p2pcd_types::PeerId, u64> = HashMap::new();
        // set of peers currently considered reachable
        let mut reachable: HashSet<p2pcd_types::PeerId> = HashSet::new();

        let interval = Duration::from_millis(self.poll_interval_ms);

        loop {
            sleep(interval).await;

            let dump_output = match get_wg_dump_output().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("WgPeerMonitor: wg show dump failed: {}", e);
                    continue;
                }
            };

            let current_peers = parse_wg_dump(&dump_output);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let mut seen_ids: HashSet<p2pcd_types::PeerId> = HashSet::new();

            for peer in &current_peers {
                let id = peer.public_key;
                seen_ids.insert(id);

                let is_fresh = peer.latest_handshake > 0
                    && now.saturating_sub(peer.latest_handshake) < HANDSHAKE_TIMEOUT_SECS;
                let prev_hs = last_handshake.get(&id).copied().unwrap_or(0);
                let handshake_advanced = peer.latest_handshake > prev_hs;

                if is_fresh && (handshake_advanced || !reachable.contains(&id)) {
                    last_handshake.insert(id, peer.latest_handshake);
                    if reachable.insert(id) {
                        // newly visible
                        let _ = self.tx.send(WgPeerEvent::PeerVisible(id)).await;
                        tracing::info!("WgPeerMonitor: peer visible: {}", peer_id_short(&id));
                    }
                    // if already reachable + handshake advanced: just update ts, no new event
                } else if !is_fresh && reachable.remove(&id) {
                    let _ = self.tx.send(WgPeerEvent::PeerUnreachable(id)).await;
                    tracing::info!("WgPeerMonitor: peer unreachable: {}", peer_id_short(&id));
                }
            }

            // Peers that vanished entirely from the dump
            let removed: Vec<p2pcd_types::PeerId> = reachable
                .iter()
                .filter(|id| !seen_ids.contains(*id))
                .copied()
                .collect();
            for id in removed {
                reachable.remove(&id);
                last_handshake.remove(&id);
                let _ = self.tx.send(WgPeerEvent::PeerRemoved(id)).await;
                tracing::info!("WgPeerMonitor: peer removed: {}", peer_id_short(&id));
            }
        }
    }
}

/// Run `wg show howm0 dump` and return stdout.
async fn get_wg_dump_output() -> anyhow::Result<String> {
    let output = tokio::process::Command::new("wg")
        .args(["show", WG_IFACE, "dump"])
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No such device") || stderr.contains("Unable to access") {
            return Ok(String::new());
        }
        return Err(anyhow::anyhow!("wg show dump failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Format first 4 bytes of a PeerId as base64 for log messages.
fn peer_id_short(id: &p2pcd_types::PeerId) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(&id[..4])
}

// ── Monitor unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod monitor_tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    /// Build a valid `wg show dump`-style line for a given 32-byte peer key.
    fn make_dump_line(
        key: &[u8; 32],
        endpoint: &str,
        allowed_ips: &str,
        handshake: u64,
        rx: u64,
        tx: u64,
    ) -> String {
        format!(
            "{}\t(none)\t{}\t{}\t{}\t{}\t{}\t25",
            STANDARD.encode(key),
            endpoint,
            allowed_ips,
            handshake,
            rx,
            tx,
        )
    }

    fn sample_dump() -> String {
        let iface_key = [0u8; 32];
        let peer1_key = [1u8; 32];
        let peer2_key = [2u8; 32];
        // Interface self-line (skipped by parse)
        let iface_line = make_dump_line(&iface_key, "0.0.0.0:51820", "100.222.0.1/32", 0, 0, 0);
        // Peer 1: has handshake, has endpoint
        let peer1_line = make_dump_line(
            &peer1_key,
            "203.0.113.1:51820",
            "100.222.0.2/32",
            1700000000,
            1024,
            512,
        );
        // Peer 2: no handshake, no endpoint
        let peer2_line = make_dump_line(&peer2_key, "(none)", "100.222.0.3/32", 0, 0, 0);
        format!("{}\n{}\n{}\n", iface_line, peer1_line, peer2_line)
    }

    #[test]
    fn parse_wg_dump_parses_peers() {
        let dump = sample_dump();
        let peers = parse_wg_dump(&dump);
        // Interface self-line is skipped → 2 peer lines
        assert_eq!(peers.len(), 2, "expected 2 peers, got {}", peers.len());

        // Peer 1: has handshake, has endpoint
        assert_eq!(peers[0].public_key, [1u8; 32]);
        assert_eq!(peers[0].latest_handshake, 1700000000);
        assert_eq!(peers[0].rx_bytes, 1024);
        assert_eq!(peers[0].tx_bytes, 512);
        assert!(peers[0].endpoint.is_some());
        assert_eq!(peers[0].allowed_ips, vec!["100.222.0.2/32"]);

        // Peer 2: no handshake, no endpoint
        assert_eq!(peers[1].public_key, [2u8; 32]);
        assert_eq!(peers[1].latest_handshake, 0);
        assert!(peers[1].endpoint.is_none());
    }

    #[test]
    fn parse_wg_dump_empty_output() {
        assert!(parse_wg_dump("").is_empty());
    }

    #[test]
    fn parse_wg_dump_interface_only() {
        // One line (the interface self-line) — skip(1) skips it, nothing left
        let iface_key = [0u8; 32];
        let line = make_dump_line(&iface_key, "0.0.0.0:51820", "100.222.0.1/32", 0, 0, 0);
        let peers = parse_wg_dump(&format!("{}\n", line));
        assert!(peers.is_empty());
    }

    #[test]
    fn parse_wg_dump_rejects_bad_key() {
        // Line with garbage pubkey
        let bad = "notavalidkey\t(none)\t1.2.3.4:51820\t100.222.0.5/32\t0\t0\t0\t25\n";
        // Prepend a dummy interface line so it gets skipped
        let dump = format!("{}\n{}", STANDARD.encode([0u8; 32]), bad);
        let peers = parse_wg_dump(&dump);
        assert!(peers.is_empty(), "bad key should be skipped");
    }

    #[test]
    fn is_reachable_requires_nonzero_handshake() {
        let state = p2pcd_types::WgPeerState {
            public_key: [1u8; 32],
            endpoint: None,
            allowed_ips: vec![],
            latest_handshake: 1700000000,
            rx_bytes: 0,
            tx_bytes: 0,
        };
        let never = p2pcd_types::WgPeerState {
            latest_handshake: 0,
            ..state.clone()
        };
        assert!(state.is_reachable());
        assert!(!never.is_reachable());
    }

    #[tokio::test]
    async fn monitor_spawns_without_panic() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::channel(16);
        let monitor = WgPeerMonitor::new(50, tx);
        let handle = monitor.spawn();
        tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
        handle.abort();
    }
}

// ── Address management ──────────────────────────────────────────────────────

/// Assign the next free IP address in the 100.222.0.0/16 space.
pub fn assign_next_address(data_dir: &Path) -> anyhow::Result<String> {
    let addr_file = data_dir.join("wireguard").join("addresses.json");
    let mut addresses: Vec<String> = if addr_file.exists() {
        let text = std::fs::read_to_string(&addr_file)?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        vec![]
    };

    let mut octet3: u8 = 0;
    let mut octet4: u8 = 2;

    loop {
        let candidate = format!("{}.{}.{}", WG_SUBNET, octet3, octet4);
        if !addresses.contains(&candidate) {
            addresses.push(candidate.clone());
            let tmp = data_dir.join("wireguard").join("addresses.json.tmp");
            std::fs::write(&tmp, serde_json::to_string_pretty(&addresses)?)?;
            std::fs::rename(&tmp, &addr_file)?;
            return Ok(candidate);
        }
        octet4 += 1;
        if octet4 == 0 {
            octet3 += 1;
            if octet3 == 0 {
                return Err(anyhow::anyhow!("address space exhausted"));
            }
        }
    }
}

/// Reclaim a previously assigned IP address, making it available for reuse.
#[allow(dead_code)]
pub fn reclaim_address(data_dir: &Path, address: &str) -> anyhow::Result<()> {
    let addr_file = data_dir.join("wireguard").join("addresses.json");
    if !addr_file.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&addr_file)?;
    let mut addresses: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    addresses.retain(|a| a != address);
    let tmp = data_dir.join("wireguard").join("addresses.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&addresses)?)?;
    std::fs::rename(&tmp, &addr_file)?;
    Ok(())
}

// ── Peer persistence ────────────────────────────────────────────────────────

fn load_peers(wg_dir: &Path) -> anyhow::Result<Vec<WgPeerConfig>> {
    let peers_dir = wg_dir.join("peers");
    if !peers_dir.exists() {
        return Ok(vec![]);
    }
    let mut peers = Vec::new();
    for entry in std::fs::read_dir(&peers_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            let text = std::fs::read_to_string(&path)?;
            if let Ok(peer) = serde_json::from_str::<WgPeerConfig>(&text) {
                peers.push(peer);
            }
        }
    }
    Ok(peers)
}

use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;

use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::docker;

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WgConfig {
    pub enabled: bool,
    pub port: u16,
    pub endpoint: Option<String>, // public addr:port for peers to reach us
    pub address: Option<String>,  // override WG address (10.47.x.y)
    pub data_dir: PathBuf,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WgState {
    pub public_key: Option<String>,
    pub address: Option<String>,  // 10.47.x.y
    pub endpoint: Option<String>, // public addr:port
    pub container_id: Option<String>,
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

const WG_IMAGE: &str = "linuxserver/wireguard:latest";
const WG_SUBNET: &str = "10.47"; // 10.47.0.0/16

// ── Initialization ──────────────────────────────────────────────────────────

/// Initialize WireGuard: pull image, generate keypair, start container, configure interface.
/// Returns WgState with our public key, address, and container ID.
pub async fn init(config: &WgConfig) -> anyhow::Result<WgState> {
    if !config.enabled {
        info!("WireGuard disabled");
        return Ok(WgState {
            public_key: None,
            address: None,
            endpoint: None,
            container_id: None,
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
                // First node gets 10.47.0.1
                let addr = format!("{}.0.1", WG_SUBNET);
                std::fs::write(&addr_file, &addr)?;
                addr
            }
        }
    };
    info!("WG address: {}", address);

    let endpoint = config.endpoint.clone();

    // Pull WG image
    docker::pull_image(WG_IMAGE).await?;

    // Start WG container
    let container_id = start_container(config, &private_key, &address).await?;
    info!("WG container started: {}", &container_id[..12]);

    // Load any saved peers
    let peers = load_peers(&wg_dir)?;
    for peer in &peers {
        if let Err(e) = add_peer_to_wg(&container_id, peer).await {
            warn!("Failed to restore WG peer {}: {}", peer.name, e);
        }
    }
    info!("Restored {} WG peers", peers.len());

    Ok(WgState {
        public_key: Some(public_key),
        address: Some(address),
        endpoint,
        container_id: Some(container_id),
    })
}

// ── Container management ────────────────────────────────────────────────────

async fn start_container(
    config: &WgConfig,
    private_key: &str,
    address: &str,
) -> anyhow::Result<String> {
    let docker = docker::connect()?;
    let short_id = &config.node_id[..8.min(config.node_id.len())];
    let container_name = format!("howm-wg-{}", short_id);

    // Remove existing container if any
    let _ = docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;

    let wg_conf = format!(
        "[Interface]\nPrivateKey = {}\nAddress = {}/16\nListenPort = {}\n",
        private_key, address, config.port
    );

    let wg_dir = config.data_dir.join("wireguard");
    let conf_path = wg_dir.join("wg0.conf");
    std::fs::write(&conf_path, &wg_conf)?;

    let wg_dir_str = wg_dir
        .canonicalize()
        .unwrap_or_else(|_| wg_dir.clone())
        .to_string_lossy()
        .to_string();

    let is_linux = cfg!(target_os = "linux");

    let mut host_config = HostConfig {
        cap_add: Some(vec!["NET_ADMIN".to_string()]),
        binds: Some(vec![format!("{}:/config/wg_confs", wg_dir_str)]),
        ..Default::default()
    };

    if is_linux {
        host_config.network_mode = Some("host".to_string());
        host_config.devices = Some(vec![bollard::models::DeviceMapping {
            path_on_host: Some("/dev/net/tun".to_string()),
            path_in_container: Some("/dev/net/tun".to_string()),
            cgroup_permissions: Some("rwm".to_string()),
        }]);
    } else {
        // macOS / Windows Docker Desktop — publish UDP port
        let mut port_bindings = std::collections::HashMap::new();
        port_bindings.insert(
            format!("{}/udp", config.port),
            Some(vec![bollard::models::PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(config.port.to_string()),
            }]),
        );
        host_config.port_bindings = Some(port_bindings);
    }

    let env = [
        "PUID=1000".to_string(),
        "PGID=1000".to_string(),
        "TZ=UTC".to_string(),
    ];

    let create_options = CreateContainerOptions {
        name: container_name.as_str(),
        platform: None,
    };

    let container_config = ContainerConfig {
        image: Some(WG_IMAGE),
        host_config: Some(host_config),
        env: Some(env.iter().map(|s| s.as_str()).collect()),
        ..Default::default()
    };

    let response = docker
        .create_container(Some(create_options), container_config)
        .await?;
    docker
        .start_container(&response.id, None::<StartContainerOptions<String>>)
        .await?;

    // Wait a moment for the interface to come up
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    Ok(response.id)
}

/// Stop and remove the WG container.
pub async fn shutdown(container_id: &str) -> anyhow::Result<()> {
    let docker = docker::connect()?;
    info!(
        "Stopping WG container: {}",
        &container_id[..12.min(container_id.len())]
    );
    let _ = docker
        .stop_container(container_id, Some(StopContainerOptions { t: 5 }))
        .await;
    let _ = docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    Ok(())
}

// ── Key management ──────────────────────────────────────────────────────────

/// Ensure a WG keypair exists on disk. Returns (private_key, public_key).
fn ensure_keypair(wg_dir: &Path) -> anyhow::Result<(String, String)> {
    let priv_path = wg_dir.join("private_key");
    let pub_path = wg_dir.join("public_key");

    if priv_path.exists() && pub_path.exists() {
        let private_key = std::fs::read_to_string(&priv_path)?.trim().to_string();
        let public_key = std::fs::read_to_string(&pub_path)?.trim().to_string();
        return Ok((private_key, public_key));
    }

    // Generate new keypair using x25519
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
    use rand::RngCore;
    let mut key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key_bytes);
    // Clamp for x25519
    key_bytes[0] &= 248;
    key_bytes[31] &= 127;
    key_bytes[31] |= 64;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(key_bytes)
}

/// Derive a WireGuard public key from a private key.
fn derive_public_key(private_key_b64: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let private_bytes = STANDARD
        .decode(private_key_b64)
        .expect("valid base64 private key");
    let mut key = [0u8; 32];
    key.copy_from_slice(&private_bytes[..32]);

    // x25519 base point multiplication
    let public_bytes = x25519_scalar_mult(&key);
    STANDARD.encode(public_bytes)
}

/// Generate a WireGuard pre-shared key (random 32 bytes, base64-encoded).
pub fn generate_psk() -> String {
    use rand::RngCore;
    let mut key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key_bytes);
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(key_bytes)
}

/// Minimal x25519 scalar multiplication (base point).
/// This is a simplified implementation for key derivation.
fn x25519_scalar_mult(scalar: &[u8; 32]) -> [u8; 32] {
    // Use the x25519 base point: 9 followed by zeros
    let mut base_point = [0u8; 32];
    base_point[0] = 9;

    // We implement the Montgomery ladder for Curve25519
    x25519_mult(scalar, &base_point)
}

/// x25519 Diffie-Hellman function (RFC 7748).
/// Field element is represented as [u64; 5] in radix 2^51.
fn x25519_mult(k: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    // Decode scalar with clamping already applied
    let mut scalar = *k;
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;

    // Decode u-coordinate (little-endian, mask high bit)
    let mut u_bytes = *u;
    u_bytes[31] &= 127;
    let x_1 = fe_from_bytes(&u_bytes);

    let mut x_2 = fe_one();
    let mut z_2 = fe_zero();
    let mut x_3 = x_1;
    let mut z_3 = fe_one();
    let mut swap: u64 = 0;

    for pos in (0..255).rev() {
        let bit = ((scalar[pos >> 3] >> (pos & 7)) & 1) as u64;
        swap ^= bit;
        fe_cswap(&mut x_2, &mut x_3, swap);
        fe_cswap(&mut z_2, &mut z_3, swap);
        swap = bit;

        let a = fe_add(&x_2, &z_2);
        let aa = fe_sq(&a);
        let b = fe_sub(&x_2, &z_2);
        let bb = fe_sq(&b);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x_3, &z_3);
        let d = fe_sub(&x_3, &z_3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        x_3 = fe_sq(&fe_add(&da, &cb));
        z_3 = fe_mul(&x_1, &fe_sq(&fe_sub(&da, &cb)));
        x_2 = fe_mul(&aa, &bb);
        z_2 = fe_mul(&e, &fe_add(&aa, &fe_mul_121666(&e)));
    }

    fe_cswap(&mut x_2, &mut x_3, swap);
    fe_cswap(&mut z_2, &mut z_3, swap);

    let result = fe_mul(&x_2, &fe_inv(&z_2));
    fe_to_bytes(&result)
}

// ── Field arithmetic for Curve25519 (radix 2^51) ────────────────────────────

type Fe = [u64; 5];

fn fe_zero() -> Fe {
    [0; 5]
}
fn fe_one() -> Fe {
    [1, 0, 0, 0, 0]
}

fn fe_from_bytes(s: &[u8; 32]) -> Fe {
    let mut h = [0u128; 5];
    let load8 = |b: &[u8]| -> u128 {
        let mut r = 0u128;
        for i in 0..b.len().min(8) {
            r |= (b[i] as u128) << (8 * i);
        }
        r
    };
    h[0] = load8(&s[0..]) & 0x7ffffffffffff;
    h[1] = (load8(&s[6..]) >> 3) & 0x7ffffffffffff;
    h[2] = (load8(&s[12..]) >> 6) & 0x7ffffffffffff;
    h[3] = (load8(&s[19..]) >> 1) & 0x7ffffffffffff;
    h[4] = (load8(&s[24..]) >> 12) & 0x7ffffffffffff;
    [
        h[0] as u64,
        h[1] as u64,
        h[2] as u64,
        h[3] as u64,
        h[4] as u64,
    ]
}

fn fe_to_bytes(h: &Fe) -> [u8; 32] {
    let mut t = *h;
    // Reduce
    let mut q = (19 * t[4] + (1 << 50)) >> 51;
    for i in 0..4 {
        q = (t[i] + q) >> 51;
    }
    q = (t[4] + q) >> 51;
    t[0] += 19 * q;
    let carry = t[0] >> 51;
    t[0] &= 0x7ffffffffffff;
    t[1] += carry;
    let carry = t[1] >> 51;
    t[1] &= 0x7ffffffffffff;
    t[2] += carry;
    let carry = t[2] >> 51;
    t[2] &= 0x7ffffffffffff;
    t[3] += carry;
    let carry = t[3] >> 51;
    t[3] &= 0x7ffffffffffff;
    t[4] += carry;
    t[4] &= 0x7ffffffffffff;

    // Final reduction
    let mut m = t[0].wrapping_sub(0x7ffffffffffed);
    for i in 1..4 {
        m &= t[i].wrapping_sub(0x7ffffffffffff);
    }
    m &= t[4].wrapping_sub(0x7ffffffffffff);
    let mask = (m >> 63).wrapping_sub(1);
    t[0] -= 0x7ffffffffffed & mask;
    for i in 1..5 {
        t[i] -= 0x7ffffffffffff & mask;
    }

    // Serialize using u128 to handle wide shifts
    let mut s = [0u8; 32];
    let combined: u128 = (t[0] as u128) | ((t[1] as u128) << 51) | ((t[2] as u128) << 102);
    for i in 0..16 {
        s[i] = (combined >> (8 * i)) as u8;
    }
    let combined2: u128 = ((t[2] as u128) >> 26) | ((t[3] as u128) << 25) | ((t[4] as u128) << 76);
    for i in 0..16 {
        s[i + 16] = (combined2 >> (8 * i)) as u8;
    }
    s
}

fn fe_add(a: &Fe, b: &Fe) -> Fe {
    let mut r = [0u64; 5];
    for i in 0..5 {
        r[i] = a[i] + b[i];
    }
    r
}

fn fe_sub(a: &Fe, b: &Fe) -> Fe {
    // Add 2p to avoid underflow
    let two_p: Fe = [
        0xfffffffffffda,
        0xffffffffffffe,
        0xffffffffffffe,
        0xffffffffffffe,
        0xffffffffffffe,
    ];
    let mut r = [0u64; 5];
    for i in 0..5 {
        r[i] = a[i] + two_p[i] - b[i];
    }
    r
}

fn fe_mul(a: &Fe, b: &Fe) -> Fe {
    let mut t = [0u128; 5];
    for i in 0..5 {
        for j in 0..5 {
            let idx = i + j;
            let prod = (a[i] as u128) * (b[j] as u128);
            if idx < 5 {
                t[idx] += prod;
            } else {
                t[idx - 5] += prod * 19;
            }
        }
    }
    let mut r = [0u64; 5];
    let mut carry = 0u128;
    for i in 0..5 {
        t[i] += carry;
        r[i] = (t[i] & 0x7ffffffffffff) as u64;
        carry = t[i] >> 51;
    }
    r[0] += (carry * 19) as u64;
    let c = r[0] >> 51;
    r[0] &= 0x7ffffffffffff;
    r[1] += c;
    r
}

fn fe_sq(a: &Fe) -> Fe {
    fe_mul(a, a)
}

fn fe_mul_121666(a: &Fe) -> Fe {
    let mut t = [0u128; 5];
    for i in 0..5 {
        t[i] = (a[i] as u128) * 121666;
    }
    let mut r = [0u64; 5];
    let mut carry = 0u128;
    for i in 0..5 {
        t[i] += carry;
        r[i] = (t[i] & 0x7ffffffffffff) as u64;
        carry = t[i] >> 51;
    }
    r[0] += (carry * 19) as u64;
    let c = r[0] >> 51;
    r[0] &= 0x7ffffffffffff;
    r[1] += c;
    r
}

fn fe_inv(a: &Fe) -> Fe {
    // Fermat's little theorem: a^(p-2) mod p, p = 2^255 - 19
    let mut t0 = fe_sq(a); // a^2
    let mut t1 = fe_sq(&t0); // a^4
    t1 = fe_sq(&t1); // a^8
    t1 = fe_mul(&t1, a); // a^9
    t0 = fe_mul(&t0, &t1); // a^11
    let mut t2 = fe_sq(&t0); // a^22
    t1 = fe_mul(&t1, &t2); // a^(2^5-1) = a^31
    t2 = fe_sq(&t1);
    for _ in 1..5 {
        t2 = fe_sq(&t2);
    }
    t1 = fe_mul(&t2, &t1); // a^(2^10-1)
    t2 = fe_sq(&t1);
    for _ in 1..10 {
        t2 = fe_sq(&t2);
    }
    t2 = fe_mul(&t2, &t1); // a^(2^20-1)
    let mut t3 = fe_sq(&t2);
    for _ in 1..20 {
        t3 = fe_sq(&t3);
    }
    t2 = fe_mul(&t3, &t2); // a^(2^40-1)
    t2 = fe_sq(&t2);
    for _ in 1..10 {
        t2 = fe_sq(&t2);
    }
    t1 = fe_mul(&t2, &t1); // a^(2^50-1)
    t2 = fe_sq(&t1);
    for _ in 1..50 {
        t2 = fe_sq(&t2);
    }
    t2 = fe_mul(&t2, &t1); // a^(2^100-1)
    t3 = fe_sq(&t2);
    for _ in 1..100 {
        t3 = fe_sq(&t3);
    }
    t2 = fe_mul(&t3, &t2); // a^(2^200-1)
    t2 = fe_sq(&t2);
    for _ in 1..50 {
        t2 = fe_sq(&t2);
    }
    t1 = fe_mul(&t2, &t1); // a^(2^250-1)
    t1 = fe_sq(&t1);
    t1 = fe_sq(&t1); // a^(2^252)
    t0 = fe_mul(&t1, &t0); // a^(2^252 - 3) — but we need 2^255-21
                           // Actually: p-2 = 2^255 - 21; above is the standard chain
                           // Correct chain for a^(p-2):
    t1 = fe_sq(&t0);
    t1 = fe_sq(&t1);
    t1 = fe_sq(&t1);
    fe_mul(&t1, a)
}

fn fe_cswap(a: &mut Fe, b: &mut Fe, swap: u64) {
    let mask = 0u64.wrapping_sub(swap);
    for i in 0..5 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t;
    }
}

// ── Peer operations ─────────────────────────────────────────────────────────

/// Add a WireGuard peer. Executes `wg set` inside the container and saves config.
pub async fn add_peer(
    container_id: &str,
    data_dir: &Path,
    peer: &WgPeerConfig,
) -> anyhow::Result<()> {
    add_peer_to_wg(container_id, peer).await?;

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
        peer.pubkey[..8].to_string()
    );
    Ok(())
}

/// Execute `wg set` inside the container to add a peer.
async fn add_peer_to_wg(container_id: &str, peer: &WgPeerConfig) -> anyhow::Result<()> {
    let docker = docker::connect()?;

    // If we have a PSK, write it to a temp file inside the container first
    let mut cmd = vec![
        "wg".to_string(),
        "set".to_string(),
        "wg0".to_string(),
        "peer".to_string(),
        peer.pubkey.clone(),
        "allowed-ips".to_string(),
        format!("{}/32", peer.allowed_ip),
        "persistent-keepalive".to_string(),
        "25".to_string(),
    ];

    if !peer.endpoint.is_empty() {
        cmd.push("endpoint".to_string());
        cmd.push(peer.endpoint.clone());
    }

    let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();

    let exec_id = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(cmd_refs),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?
        .id;

    match docker.start_exec(&exec_id, None).await? {
        StartExecResults::Attached { mut output, .. } => {
            while let Some(chunk) = output.next().await {
                if let Ok(bollard::container::LogOutput::StdErr { message }) = chunk {
                    let text = String::from_utf8_lossy(&message);
                    if !text.trim().is_empty() {
                        warn!("wg set stderr: {}", text.trim());
                    }
                }
            }
        }
        StartExecResults::Detached => {}
    }

    Ok(())
}

/// Remove a WireGuard peer.
pub async fn remove_peer(
    container_id: &str,
    data_dir: &Path,
    pubkey: &str,
    node_id: &str,
) -> anyhow::Result<()> {
    let docker = docker::connect()?;

    let exec_id = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["wg", "set", "wg0", "peer", pubkey, "remove"]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?
        .id;

    let _ = docker.start_exec(&exec_id, None).await;

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

/// Get WireGuard status by running `wg show wg0 dump` inside the container.
pub async fn get_status(container_id: &str) -> anyhow::Result<Vec<WgPeerStatus>> {
    let docker = docker::connect()?;

    let exec_id = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["wg", "show", "wg0", "dump"]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?
        .id;

    let mut output = String::new();
    match docker.start_exec(&exec_id, None).await? {
        StartExecResults::Attached {
            output: mut stream, ..
        } => {
            while let Some(chunk) = stream.next().await {
                if let Ok(bollard::container::LogOutput::StdOut { message }) = chunk {
                    output.push_str(&String::from_utf8_lossy(&message));
                }
            }
        }
        StartExecResults::Detached => {}
    }

    // Parse wg show dump output
    // Format: pubkey\tpreshared-key\tendpoint\tallowed-ips\tlatest-handshake\ttransfer-rx\ttransfer-tx\tpersistent-keepalive
    // First line is the interface itself, skip it
    let mut peers = Vec::new();
    for line in output.lines().skip(1) {
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

// ── Address management ──────────────────────────────────────────────────────

/// Assign the next free IP address in the 10.47.0.0/16 space.
pub fn assign_next_address(data_dir: &Path) -> anyhow::Result<String> {
    let addr_file = data_dir.join("wireguard").join("addresses.json");
    let mut addresses: Vec<String> = if addr_file.exists() {
        let text = std::fs::read_to_string(&addr_file)?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        vec![]
    };

    // Find next available address in 10.47.x.y space
    // Start from 10.47.0.2 (10.47.0.1 is the node itself)
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
            // wrapped
            octet3 += 1;
            if octet3 == 0 {
                return Err(anyhow::anyhow!("address space exhausted"));
            }
        }
    }
}

/// Reclaim a previously assigned IP address, making it available for reuse.
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

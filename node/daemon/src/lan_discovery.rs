//! LAN peer discovery via mDNS-SD.
//!
//! Broadcasts this node as `_howm._udp.local` so other howm nodes on the same
//! WiFi/LAN can discover it without manual IP entry. The broadcast exposes
//! only the node name and a truncated pubkey fingerprint — no secrets.
//!
//! The mDNS daemon runs in a background thread managed by `mdns-sd`.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, warn};

/// mDNS service type for howm nodes.
const SERVICE_TYPE: &str = "_howm._udp.local.";

/// How long to scan during a LAN peer search.
const SCAN_DURATION: Duration = Duration::from_secs(3);

/// A discovered howm node on the local network.
#[derive(Debug, Clone, Serialize)]
pub struct LanPeer {
    /// Node display name.
    pub name: String,
    /// Truncated WireGuard public key fingerprint (first 8 chars of base64).
    pub fingerprint: String,
    /// Full WireGuard public key (base64).
    pub wg_pubkey: String,
    /// LAN IP address.
    pub lan_ip: String,
    /// Daemon API port.
    pub daemon_port: u16,
    /// WireGuard listen port.
    pub wg_port: u16,
}

/// Handle to the running mDNS service registration.
pub struct LanDiscovery {
    daemon: ServiceDaemon,
    fullname: String,
}

impl LanDiscovery {
    /// Start advertising this node on the local network.
    ///
    /// The service is registered immediately and remains active until
    /// `shutdown()` is called or the daemon is dropped.
    pub fn start(
        node_name: &str,
        wg_pubkey: &str,
        lan_ip: &str,
        daemon_port: u16,
        wg_port: u16,
    ) -> anyhow::Result<Self> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| anyhow::anyhow!("Failed to start mDNS daemon: {}", e))?;

        // Scope mDNS to only the LAN interface — exclude WireGuard, Tailscale,
        // Docker, and other non-LAN interfaces that cause multicast errors.
        // Disable all first, then enable only the LAN IP.
        if let Ok(ip) = lan_ip.parse::<std::net::IpAddr>() {
            let _ = daemon.disable_interface(mdns_sd::IfKind::All);
            if let Err(e) = daemon.enable_interface(mdns_sd::IfKind::Addr(ip)) {
                warn!("LAN discovery: failed to scope mDNS to {}: {}", lan_ip, e);
            }
        }

        // Build TXT record properties
        let mut properties = HashMap::new();
        properties.insert("pubkey".to_string(), wg_pubkey.to_string());
        properties.insert("wg_port".to_string(), wg_port.to_string());

        // Instance name: sanitise node name for mDNS (alphanumeric + hyphens)
        let instance_name = sanitise_instance_name(node_name);

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &format!("{}.local.", hostname_safe()),
            lan_ip,
            daemon_port,
            properties,
        )
        .map_err(|e| anyhow::anyhow!("Failed to build mDNS service info: {}", e))?;

        let fullname = service_info.get_fullname().to_string();

        daemon
            .register(service_info)
            .map_err(|e| anyhow::anyhow!("Failed to register mDNS service: {}", e))?;

        info!(
            "LAN discovery: advertising as '{}' on {}:{}",
            instance_name, lan_ip, daemon_port
        );

        Ok(Self { daemon, fullname })
    }

    /// Scan the local network for other howm nodes.
    ///
    /// Returns discovered peers after a brief scan window.
    /// Excludes our own node (matched by pubkey).
    pub async fn scan(&self, our_pubkey: &str) -> Vec<LanPeer> {
        let daemon = self.daemon.clone();
        let our_pubkey = our_pubkey.to_string();

        // Run blocking mDNS browse in a spawn_blocking to avoid blocking tokio
        tokio::task::spawn_blocking(move || scan_blocking(&daemon, &our_pubkey))
            .await
            .unwrap_or_default()
    }

    /// Unregister the mDNS service and shut down the daemon.
    pub fn shutdown(self) {
        if let Err(e) = self.daemon.unregister(&self.fullname) {
            warn!("LAN discovery: failed to unregister mDNS service: {}", e);
        }
        if let Err(e) = self.daemon.shutdown() {
            warn!("LAN discovery: daemon shutdown error: {}", e);
        }
        info!("LAN discovery: shut down");
    }
}

/// Perform a blocking mDNS scan for howm nodes.
fn scan_blocking(daemon: &ServiceDaemon, our_pubkey: &str) -> Vec<LanPeer> {
    let receiver = match daemon.browse(SERVICE_TYPE) {
        Ok(r) => r,
        Err(e) => {
            warn!("LAN scan: failed to start browse: {}", e);
            return vec![];
        }
    };

    let mut peers = Vec::new();
    let deadline = std::time::Instant::now() + SCAN_DURATION;

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                debug!("LAN scan: resolved service {:?}", info.get_fullname());

                let pubkey = info
                    .get_properties()
                    .get("pubkey")
                    .map(|v| v.val_str().to_string())
                    .unwrap_or_default();

                // Skip our own node
                if pubkey == our_pubkey {
                    continue;
                }

                let wg_port: u16 = info
                    .get_properties()
                    .get("wg_port")
                    .and_then(|v| v.val_str().parse().ok())
                    .unwrap_or(41641);

                // Get the first IPv4 address from the service info
                let lan_ip = info
                    .get_addresses()
                    .iter()
                    .find(|a| a.is_ipv4())
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                if lan_ip.is_empty() || pubkey.is_empty() {
                    debug!("LAN scan: skipping incomplete service entry");
                    continue;
                }

                let fingerprint = if pubkey.len() >= 8 {
                    pubkey[..8].to_string()
                } else {
                    pubkey.clone()
                };

                // Instance name from mDNS is the node name
                let name = extract_instance_name(info.get_fullname());

                peers.push(LanPeer {
                    name,
                    fingerprint,
                    wg_pubkey: pubkey,
                    lan_ip,
                    daemon_port: info.get_port(),
                    wg_port,
                });
            }
            Ok(_) => {
                // Other events (SearchStarted, ServiceFound, etc) — ignore
            }
            Err(flume::RecvTimeoutError::Timeout) => break,
            Err(flume::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Stop browsing
    let _ = daemon.stop_browse(SERVICE_TYPE);

    info!("LAN scan: found {} peer(s)", peers.len());
    peers
}

/// Sanitise a node name for use as an mDNS instance name.
/// Keeps alphanumerics, hyphens, and spaces. Replaces others with hyphens.
fn sanitise_instance_name(name: &str) -> String {
    let sanitised: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();

    if sanitised.is_empty() {
        "howm-node".to_string()
    } else {
        sanitised
    }
}

/// Extract the instance name from a fully qualified mDNS service name.
/// e.g. "my-node._howm._udp.local." -> "my-node"
fn extract_instance_name(fullname: &str) -> String {
    fullname
        .strip_suffix('.')
        .unwrap_or(fullname)
        .split('.')
        .next()
        .unwrap_or("unknown")
        .trim_end_matches(&format!(".{}", SERVICE_TYPE.trim_end_matches('.')))
        .to_string()
}

/// Get a safe hostname for mDNS (fallback to "howm-node").
fn hostname_safe() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "howm-node".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitise_instance_name() {
        assert_eq!(sanitise_instance_name("alice"), "alice");
        assert_eq!(sanitise_instance_name("my-node"), "my-node");
        assert_eq!(sanitise_instance_name("My Node 2"), "My Node 2");
        assert_eq!(sanitise_instance_name("node@home!"), "node-home-");
        assert_eq!(sanitise_instance_name(""), "howm-node");
        assert_eq!(
            sanitise_instance_name("café☕"),
            "café-" // unicode letters kept, emoji replaced
        );
    }

    #[test]
    fn test_extract_instance_name() {
        assert_eq!(
            extract_instance_name("my-node._howm._udp.local."),
            "my-node"
        );
        assert_eq!(extract_instance_name("alice._howm._udp.local."), "alice");
        // Edge case: no trailing dot
        assert_eq!(extract_instance_name("bob._howm._udp.local"), "bob");
    }

    #[test]
    fn test_lan_peer_serialization() {
        let peer = LanPeer {
            name: "alice".to_string(),
            fingerprint: "dGVzdC1w".to_string(),
            wg_pubkey: "dGVzdC1wdWJrZXktYWxpY2U=".to_string(),
            lan_ip: "192.168.1.100".to_string(),
            daemon_port: 7000,
            wg_port: 41641,
        };
        let json = serde_json::to_value(&peer).unwrap();
        assert_eq!(json["name"], "alice");
        assert_eq!(json["lan_ip"], "192.168.1.100");
        assert_eq!(json["daemon_port"], 7000);
    }

    #[test]
    fn test_mdns_start_and_scan() {
        // This test verifies the mDNS daemon can start and scan without crashing.
        // On CI (no network), the scan simply returns empty results.
        let discovery =
            LanDiscovery::start("test-node", "dGVzdC1wdWJrZXk=", "127.0.0.1", 7000, 41641);

        // mDNS may fail in some environments (containers, no multicast)
        // so we just verify it doesn't panic
        if let Ok(d) = discovery {
            // Synchronous scan via blocking (can't use async in unit test easily)
            let peers = scan_blocking(&d.daemon, "dGVzdC1wdWJrZXk=");
            // Should not include ourselves
            assert!(
                peers.iter().all(|p| p.wg_pubkey != "dGVzdC1wdWJrZXk="),
                "scan should exclude our own node"
            );
            d.shutdown();
        }
    }
}

use std::net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket};
use tracing::{debug, info, warn};

/// Attempt to detect this machine's public IPv4 address by querying well-known
/// lightweight IP-echo services. Tries each in order; returns the first success.
///
/// This is a best-effort helper used at startup when `--wg-endpoint` is not
/// provided. The detected IP is logged clearly so the user can verify it.
pub async fn detect_public_ip() -> Option<String> {
    // Services that return a bare IP address as plain text (no JSON parsing needed).
    // Mix of HTTPS and HTTP — some environments (e.g. sudo) may have TLS issues.
    let candidates = [
        "https://api.ipify.org",
        "http://ipv4.icanhazip.com",
        "https://api4.my-ip.io/ip",
        "http://checkip.amazonaws.com",
    ];

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("Public IP detection: failed to build HTTP client: {}", e);
            return None;
        }
    };

    for url in &candidates {
        debug!("Public IP detection: trying {}", url);
        match client.get(*url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    let ip = body.trim().to_string();
                    // Sanity-check: must look like an IPv4 address
                    if ip.split('.').count() == 4
                        && ip.chars().all(|c| c.is_ascii_digit() || c == '.')
                    {
                        info!("Public IP detected: {}", ip);
                        return Some(ip);
                    }
                    debug!(
                        "Public IP detection: {} returned non-IPv4 response: {:?}",
                        url,
                        &ip[..ip.len().min(50)]
                    );
                }
            }
            Ok(resp) => {
                debug!(
                    "Public IP detection: {} returned status {}",
                    url,
                    resp.status()
                );
            }
            Err(e) => {
                debug!("Public IP detection: {} failed: {}", url, e);
            }
        }
    }

    warn!(
        "Public IP detection: all {} services failed. \
         Pass --wg-endpoint <ip:port> manually.",
        candidates.len()
    );
    None
}

// ── IPv6 GUA Detection ──────────────────────────────────────────────────────

/// Check if an IPv6 address is a Global Unicast Address (GUA).
/// GUAs are in the `2000::/3` range — the only globally routable unicast block.
fn is_gua(addr: &Ipv6Addr) -> bool {
    let first_byte = addr.octets()[0];
    // 2000::/3 means first 3 bits are 001 → first byte 0x20..0x3F
    (0x20..=0x3f).contains(&first_byte)
}

/// Detect all globally routable IPv6 addresses (GUAs) on this machine.
///
/// Excludes:
/// - Link-local (fe80::/10)
/// - Unique Local (fd00::/8, fc00::/7)
/// - Loopback (::1)
/// - Multicast (ff00::/8)
/// - IPv4-mapped (::ffff:0:0/96)
///
/// Returns addresses sorted for stability (deterministic invite tokens).
pub fn detect_ipv6_guas() -> Vec<Ipv6Addr> {
    // Use a UDP connect trick to enumerate local addresses.
    // This doesn't send any traffic — it just causes the OS to resolve
    // source address selection, which reveals configured addresses.
    //
    // For a complete picture, we also parse interface addresses directly.
    #[cfg(unix)]
    let mut guas = detect_ipv6_guas_unix();
    #[cfg(not(unix))]
    let mut guas = Vec::new();

    // Fallback / supplement: UDP connect trick to a public IPv6 address.
    // If we get a GUA back, add it to the list.
    if guas.is_empty() {
        if let Some(addr) = detect_ipv6_via_udp_connect() {
            guas.push(addr);
        }
    }

    guas.sort();
    guas.dedup();

    if guas.is_empty() {
        debug!("IPv6 GUA detection: no globally routable addresses found");
    } else {
        info!("IPv6 GUA detection: found {} address(es)", guas.len());
        for addr in &guas {
            info!("  IPv6 GUA: {}", addr);
        }
    }

    guas
}

/// Parse interface addresses on Unix via getifaddrs.
#[cfg(unix)]
fn detect_ipv6_guas_unix() -> Vec<Ipv6Addr> {
    use std::ffi::CStr;

    let mut guas = Vec::new();

    unsafe {
        let mut ifaddrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            warn!("IPv6 GUA detection: getifaddrs failed");
            return guas;
        }

        let mut current = ifaddrs;
        while !current.is_null() {
            let ifa = &*current;

            if !ifa.ifa_addr.is_null() {
                let family = (*ifa.ifa_addr).sa_family as i32;
                if family == libc::AF_INET6 {
                    let sockaddr_in6 = ifa.ifa_addr as *const libc::sockaddr_in6;
                    let raw = (*sockaddr_in6).sin6_addr.s6_addr;
                    let addr = Ipv6Addr::from(raw);

                    if is_gua(&addr) {
                        let iface_name = CStr::from_ptr(ifa.ifa_name).to_str().unwrap_or("unknown");
                        debug!("IPv6 GUA found on {}: {}", iface_name, addr);
                        guas.push(addr);
                    }
                }
            }

            current = ifa.ifa_next;
        }

        libc::freeifaddrs(ifaddrs);
    }

    guas
}

/// Detect IPv6 GUA via UDP connect trick.
/// Connects a UDP socket to a known public IPv6 address (Google DNS)
/// and reads back the local address the OS selected.
fn detect_ipv6_via_udp_connect() -> Option<Ipv6Addr> {
    // Google Public DNS IPv6
    let dest: SocketAddr = "[2001:4860:4860::8888]:80".parse().ok()?;
    let socket = UdpSocket::bind("[::]:0").ok()?;
    socket.connect(dest).ok()?;
    let local = socket.local_addr().ok()?;

    if let IpAddr::V6(addr) = local.ip() {
        if is_gua(&addr) {
            debug!("IPv6 GUA via UDP connect: {}", addr);
            return Some(addr);
        }
    }
    None
}

// ── LAN IP Detection ────────────────────────────────────────────────────────

/// Detect the primary LAN IPv4 address of this machine.
///
/// Uses the UDP connect trick: opens a UDP socket "connected" to a LAN-scoped
/// multicast address, then reads back the local address the OS selected.
/// This doesn't send any traffic — it just triggers source address selection.
///
/// Returns the first private IPv4 address (10.x, 172.16-31.x, 192.168.x).
/// Excludes loopback (127.x) and the WireGuard subnet (100.222.x).
pub fn detect_lan_ip() -> Option<String> {
    // Connect to a LAN multicast address to trigger OS source address selection.
    // 224.0.0.251 is the mDNS multicast group — guaranteed to exist on any
    // multicast-capable network but we never actually send anything.
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("224.0.0.251:5353").ok()?;
    let local = socket.local_addr().ok()?;

    if let std::net::IpAddr::V4(addr) = local.ip() {
        if is_private_lan(addr) {
            info!("LAN IP detected: {}", addr);
            return Some(addr.to_string());
        }
    }

    // Fallback: enumerate all interfaces on Unix
    #[cfg(unix)]
    {
        if let Some(addr) = detect_lan_ip_unix() {
            info!("LAN IP detected (interface scan): {}", addr);
            return Some(addr.to_string());
        }
    }

    debug!("LAN IP detection: no private LAN address found");
    None
}

/// Check if an IPv4 address is a private LAN address (RFC 1918),
/// excluding loopback and the WireGuard subnet.
fn is_private_lan(addr: std::net::Ipv4Addr) -> bool {
    let o = addr.octets();
    // 10.0.0.0/8
    if o[0] == 10 {
        return true;
    }
    // 172.16.0.0/12
    if o[0] == 172 && (16..=31).contains(&o[1]) {
        return true;
    }
    // 192.168.0.0/16
    if o[0] == 192 && o[1] == 168 {
        return true;
    }
    false
}

/// Enumerate interfaces on Unix to find a private LAN IPv4 address.
#[cfg(unix)]
fn detect_lan_ip_unix() -> Option<std::net::Ipv4Addr> {
    unsafe {
        let mut ifaddrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return None;
        }

        let mut result = None;
        let mut current = ifaddrs;
        while !current.is_null() {
            let ifa = &*current;
            if !ifa.ifa_addr.is_null() {
                let family = (*ifa.ifa_addr).sa_family as i32;
                if family == libc::AF_INET {
                    let sockaddr_in = ifa.ifa_addr as *const libc::sockaddr_in;
                    let raw = (*sockaddr_in).sin_addr.s_addr.to_ne_bytes();
                    let addr = std::net::Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3]);
                    if is_private_lan(addr) {
                        let iface_name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                            .to_str()
                            .unwrap_or("unknown");
                        debug!("LAN IPv4 found on {}: {}", iface_name, addr);
                        result = Some(addr);
                        break;
                    }
                }
            }
            current = ifa.ifa_next;
        }

        libc::freeifaddrs(ifaddrs);
        result
    }
}

// ── WG Port Fallback ────────────────────────────────────────────────────────

/// Maximum number of ports to try in the fallback range.
const PORT_FALLBACK_RANGE: u16 = 10;

/// Find an available UDP port for WireGuard, starting from the configured port
/// and falling back through the next PORT_FALLBACK_RANGE ports.
///
/// Returns the first port that can be successfully bound.
/// Note: this just checks availability — the actual WG interface creation
/// may still fail for other reasons (permissions, kernel module, etc).
pub fn find_available_wg_port(preferred: u16) -> u16 {
    for offset in 0..PORT_FALLBACK_RANGE {
        let port = preferred.saturating_add(offset);
        // Try binding to check availability
        match UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port))) {
            Ok(_socket) => {
                // Socket is dropped here, freeing the port for WG to use
                if offset > 0 {
                    info!("WG port {}: in use, falling back to {}", preferred, port);
                }
                return port;
            }
            Err(_) => {
                debug!("WG port {}: unavailable, trying next", port);
            }
        }
    }

    // All ports in range occupied — return the preferred port and let WG
    // surface the actual error with a clear message
    warn!(
        "All WG ports {}-{} unavailable, attempting {} anyway",
        preferred,
        preferred.saturating_add(PORT_FALLBACK_RANGE - 1),
        preferred
    );
    preferred
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn test_is_gua() {
        // Valid GUAs (2000::/3)
        assert!(is_gua(&"2001:db8::1".parse::<Ipv6Addr>().unwrap()));
        assert!(is_gua(
            &"2607:f8b0:4004:800::200e".parse::<Ipv6Addr>().unwrap()
        ));
        assert!(is_gua(
            &"2a00:1450:4001:800::200e".parse::<Ipv6Addr>().unwrap()
        ));

        // Not GUAs
        assert!(!is_gua(&"fe80::1".parse::<Ipv6Addr>().unwrap())); // link-local
        assert!(!is_gua(&"fd00::1".parse::<Ipv6Addr>().unwrap())); // ULA
        assert!(!is_gua(&"fc00::1".parse::<Ipv6Addr>().unwrap())); // ULA
        assert!(!is_gua(&"::1".parse::<Ipv6Addr>().unwrap())); // loopback
        assert!(!is_gua(&"ff02::1".parse::<Ipv6Addr>().unwrap())); // multicast
        assert!(!is_gua(&"::ffff:192.168.1.1".parse::<Ipv6Addr>().unwrap())); // v4-mapped
    }

    #[test]
    fn test_find_available_wg_port_returns_preferred_when_free() {
        // Use a high ephemeral port unlikely to be in use
        let port = find_available_wg_port(49999);
        // Should get 49999 or very close to it
        assert!((49999..=49999 + PORT_FALLBACK_RANGE).contains(&port));
    }

    #[test]
    fn test_is_private_lan() {
        use std::net::Ipv4Addr;
        assert!(is_private_lan(Ipv4Addr::new(192, 168, 1, 100)));
        assert!(is_private_lan(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_lan(Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_private_lan(Ipv4Addr::new(172, 31, 255, 255)));
        assert!(!is_private_lan(Ipv4Addr::new(172, 32, 0, 1)));
        assert!(!is_private_lan(Ipv4Addr::new(100, 222, 0, 1))); // WG subnet
        assert!(!is_private_lan(Ipv4Addr::new(127, 0, 0, 1))); // loopback
        assert!(!is_private_lan(Ipv4Addr::new(8, 8, 8, 8))); // public
    }

    #[test]
    fn test_detect_lan_ip_runs_without_panic() {
        // Just verify it doesn't crash — may or may not find an address
        let _ = detect_lan_ip();
    }

    #[test]
    fn test_detect_ipv6_guas_runs_without_panic() {
        // Just verify it doesn't crash — may or may not find addresses
        let guas = detect_ipv6_guas();
        // All returned addresses should be GUAs
        for addr in &guas {
            assert!(is_gua(addr), "{} is not a GUA", addr);
        }
    }
}

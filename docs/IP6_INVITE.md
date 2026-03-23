# IPv6 Invite Spec

Public IPv6 addresses are globally routable with no NAT. By requiring IPv6 for
the invite ceremony, peers can reach each other directly without port
forwarding or relay servers.

---

## Problem

The invite ceremony requires the joiner to make an HTTP call to the inviter's
daemon. On IPv4, most home users are behind NAT — the inviter's public IP maps
to a router, not their machine. Without manual port forwarding, the call fails.

IPv6 eliminates this: every device gets a globally unique address. The only
remaining barrier is the host firewall (most allow outbound; some block
unsolicited inbound UDP — the user may need to allow UDP on their WG port).

---

## Design Decisions

1. **IPv6 required for invite creation.** If the daemon cannot detect a public
   IPv6 address, invite creation returns an error with a clear message.
2. **IPv4 support is not removed.** The `--wg-endpoint` flag still accepts
   IPv4 for users with port forwarding or public IPs. IPv6 is the default
   auto-detected path.
3. **Endpoint format uses bracket notation.** IPv6 endpoints are encoded as
   `[addr]:port` (RFC 2732) throughout — invite tokens, WireGuard config,
   HTTP URLs. The pipe delimiter `|` in tokens is unaffected.
4. **WireGuard listens on IPv6.** The WG interface binds to `::` (dual-stack)
   so it accepts connections on both IPv4 and IPv6.
5. **The daemon HTTP server stays on `0.0.0.0`.** The invite ceremony routes
   (`/node/complete-invite`, `/node/open-join`) must be reachable over the
   public internet. The subnet middleware protects everything else.

---

## Changes by File

### 1. `src/net_detect.rs`

Replace the IPv4-only `detect_public_ip()` with two functions:

```rust
/// Detect public IPv6 address. Preferred for invites.
pub async fn detect_public_ipv6() -> Option<String>

/// Detect public IPv4 address. Fallback only.
pub async fn detect_public_ipv4() -> Option<String>
```

**IPv6 detection services** (return bare IPv6 address as plain text):
- `https://api6.ipify.org`
- `https://ipv6.icanhazip.com`
- `https://v6.ident.me`
- `https://api6.my-ip.io/ip`

**IPv6 validation:** The response must contain at least one `:` character and
only hex digits, colons, and optionally dots (for mapped addresses). Use
`std::net::Ipv6Addr::parse()` for validation.

**IPv4 detection** keeps the existing services:
- `https://api.ipify.org`
- `http://ipv4.icanhazip.com`
- `https://api4.my-ip.io/ip`
- `http://checkip.amazonaws.com`

**Top-level convenience function:**

```rust
/// Detect public IP, preferring IPv6. Returns (ip_string, is_v6).
pub async fn detect_public_ip() -> Option<(String, bool)> {
    if let Some(ip) = detect_public_ipv6().await {
        return Some((ip, true));
    }
    if let Some(ip) = detect_public_ipv4().await {
        return Some((ip, false));
    }
    None
}
```

### 2. `src/wireguard.rs` — init()

**Endpoint construction** (currently line 92-114):

Replace the current auto-detection block:

```rust
let endpoint = if let Some(ref ep) = config.endpoint {
    info!("WG endpoint configured: {}", ep);
    config.endpoint.clone()
} else {
    info!("WG endpoint not configured — attempting public IP auto-detection...");
    match crate::net_detect::detect_public_ip().await {
        Some((ip, is_v6)) => {
            let ep = if is_v6 {
                format!("[{}]:{}", ip, config.port)
            } else {
                format!("{}:{}", ip, config.port)
            };
            info!("Auto-detected public IP → WG endpoint: {}", ep);
            Some(ep)
        }
        None => {
            warn!(
                "Public IP detection failed — invites will be refused. \
                 Pass --wg-endpoint <ip:port> to set manually."
            );
            None
        }
    }
};
```

**WireGuard interface setup** — currently uses `listen-port` which accepts
connections on all interfaces. WireGuard kernel module already listens on both
IPv4 and IPv6 when the interface is up, so no change is needed for the WG
interface itself.

### 3. `src/invite.rs`

**Token format is unchanged.** The pipe-delimited payload already treats the
endpoint as an opaque string. An IPv6 endpoint `[2001:db8::1]:51820` encodes
safely — no conflict with the `|` delimiter.

```
{pubkey}|[2001:db8::1]:51820|{wg_address}|{psk}|{assigned_ip}|{daemon_port}|{expires_at}
```

**Endpoint validation** (currently line 54): Update the `0.0.0.0` check to
also reject `[::]`:

```rust
if our_endpoint.starts_with("0.0.0.0") || our_endpoint.starts_with("[::]") {
    anyhow::bail!(
        "Cannot create invite: WireGuard endpoint not configured. \
         Restart with --wg-endpoint or ensure IPv6 is available."
    );
}
```

**No other changes to invite.rs.** Decode already uses `splitn(7, '|')` and
treats the endpoint field as a string.

### 4. `src/open_invite.rs`

Same as invite.rs — token format is pipe-delimited, endpoint is an opaque
string field. The HMAC signature covers the endpoint, which works regardless of
format.

**Update the same `0.0.0.0` validation** (currently line 43):

```rust
if our_endpoint.starts_with("0.0.0.0") || our_endpoint.starts_with("[::]") {
    anyhow::bail!(
        "Cannot create open invite: WireGuard endpoint not configured."
    );
}
```

### 5. `src/api/node_routes.rs` — HTTP calls to remote peers

**`redeem_invite`** (line 179-182): Currently constructs the URL as:

```rust
let complete_url = format!(
    "http://{}:{}/node/complete-invite",
    their_host, decoded.their_daemon_port
);
```

Where `their_host` is extracted by splitting the endpoint on the last `:`.
This breaks for IPv6 bracket notation: `[2001:db8::1]:51820` split on last `:`
yields `[2001:db8::1]` and `51820`, which is correct — but the HTTP URL needs
the brackets stripped for the host and re-added for the URL.

**Add a helper function:**

```rust
/// Parse an endpoint string into (host, port) handling both IPv4 and IPv6.
/// IPv6 bracket notation: "[::1]:51820" → ("::1", 51820)
/// IPv4: "1.2.3.4:51820" → ("1.2.3.4", 51820)
fn parse_endpoint(endpoint: &str) -> Option<(&str, u16)> {
    if let Some(bracketed) = endpoint.strip_prefix('[') {
        // IPv6: [addr]:port
        let (addr, rest) = bracketed.split_once(']')?;
        let port_str = rest.strip_prefix(':')?;
        let port = port_str.parse().ok()?;
        Some((addr, port))
    } else {
        // IPv4: addr:port
        let (addr, port_str) = endpoint.rsplit_once(':')?;
        let port = port_str.parse().ok()?;
        Some((addr, port))
    }
}
```

**Update URL construction** in `redeem_invite`:

```rust
let (their_host, _wg_port) = parse_endpoint(&decoded.their_endpoint)
    .ok_or_else(|| AppError::BadRequest("invalid endpoint in invite".into()))?;

// Wrap IPv6 addresses in brackets for HTTP URLs
let host_for_url = if their_host.contains(':') {
    format!("[{}]", their_host)
} else {
    their_host.to_string()
};

let complete_url = format!(
    "http://{}:{}/node/complete-invite",
    host_for_url, decoded.their_daemon_port
);
```

**Same pattern for `redeem_open_invite`** (line 628-641): Replace the
`rsplit_once(':')` host extraction with `parse_endpoint()`.

### 6. `src/config.rs`

**`wg_endpoint` doc comment** — update to show IPv6 format:

```rust
#[arg(long, env = "HOWM_WG_ENDPOINT")]
pub wg_endpoint: Option<String>, // e.g. "[2001:db8::1]:51820" or "1.2.3.4:51820"
```

### 7. `src/main.rs` — daemon bind address

**Add IPv6 dual-stack binding.** Change:

```rust
let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
```

To:

```rust
let addr: SocketAddr = format!("[::]:{}", config.port).parse()?;
```

Binding to `[::]` on Linux accepts both IPv4 and IPv6 connections (dual-stack
socket). This allows the invite ceremony to work over IPv6 while keeping
localhost and IPv4 WG-subnet access working.

**Verify:** The `ConnectInfo<SocketAddr>` extractor will now return IPv6
addresses. The `is_local_or_wg()` helper in `api/mod.rs` must handle
IPv4-mapped IPv6 addresses (e.g., `::ffff:127.0.0.1`):

```rust
pub(crate) fn is_local_or_wg(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || is_wg_subnet(*v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            // Handle IPv4-mapped IPv6 addresses (::ffff:127.0.0.1)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || is_wg_subnet(v4);
            }
            false
        }
    }
}
```

### 8. UI — Startup diagnostic

**In the Dashboard WireGuard section**, if `wg_endpoint` is present and
contains `[`, display it as IPv6. If the endpoint is missing, the existing
warning banner already covers it.

**No new UI components needed.** The existing endpoint warning ("WireGuard
endpoint not set — invites will not work") is sufficient.

---

## Invite Token Examples

**Regular invite (IPv6):**
```
howm://invite/NDNoczM1U2svRGwwY0dQQU9pZHpLSlJFUkNHS0gxR09BaTdvTGduS2RpZz18WzI
wMDE6ZGI4OjoxXTo1MTgyMHwxMDAuMjIyLjAuMXxhYmNkZWYxMjM0NTZ8MTAwLjIyMi4wLjJ8NzAw
MHwxNzAwMDg2NDAw
```

Decoded payload:
```
43hs35Sk/Dl0cGPAOidzKJRERCGKH1GOAi7oLgnKdig=|[2001:db8::1]:51820|100.222.0.1|abcdef123456|100.222.0.2|7000|1700086400
```

**Open invite (IPv6):**
```
howm://open/NTg5NmU1ZGMtZDc0NS00ZWMyLTlkMTgtNjM2NDUwMWJmNTFhfDQzaHMzNVNrL0R
sMGNHUEFPaWR6S0pSRVJDR0tIMUdPQWk3b0xnbktkaWc9fFsyMDAxOmRiODo6MV06NTE4MjB8NzAwM
HxzaWduYXR1cmU=
```

---

## HTTP URL Format

When constructing HTTP URLs for the invite ceremony:

```
IPv4: http://1.2.3.4:7000/node/complete-invite
IPv6: http://[2001:db8::1]:7000/node/complete-invite
```

Standard RFC 2732 bracket notation. The `reqwest` client handles this natively.

---

## WireGuard Endpoint Format

WireGuard's `wg set` command accepts IPv6 endpoints with bracket notation:

```bash
wg set howm0 peer <pubkey> endpoint [2001:db8::1]:51820 ...
```

No changes needed to the `wg` CLI invocation in `wireguard.rs`.

---

## Startup Sequence

```
1. Load config (--wg-endpoint override, if any)
2. If no override:
   a. Try detect_public_ipv6()
      → success: endpoint = "[ip]:51820"
      → log: "Auto-detected IPv6 endpoint: [2001:db8::1]:51820"
   b. Fallback: try detect_public_ipv4()
      → success: endpoint = "ip:51820"
      → log: "No IPv6 found. Using IPv4: 203.0.113.1:51820"
      → log: "Note: IPv4 may require port forwarding for invites."
   c. Both fail:
      → log: "No public IP detected. Invites disabled."
      → endpoint = None
3. Initialize WireGuard with endpoint
4. Start daemon on [::]:7000 (dual-stack)
```

---

## Error Messages

**Invite creation with no endpoint:**
```
Cannot create invite: no public endpoint configured.
Run with --wg-endpoint [ip]:51820 or ensure IPv6 connectivity.
```

**Joiner cannot reach inviter:**
```
Could not reach the inviter at [2001:db8::1]:7000.
They may need to allow inbound UDP on port 51820 in their IPv6 firewall.
```

**No IPv6 detected (warning, not fatal):**
```
No public IPv6 address detected. Falling back to IPv4.
Invites may not work without port forwarding. Set --wg-endpoint manually
or check your IPv6 connectivity.
```

---

## Migration / Backward Compatibility

- Existing IPv4 invites continue to work. The decode functions treat the
  endpoint as an opaque string.
- Peers running old versions can still redeem IPv6 invites IF they can reach
  the IPv6 address (their OS and network must support IPv6).
- The `--wg-endpoint` flag accepts both IPv4 (`1.2.3.4:51820`) and IPv6
  (`[2001:db8::1]:51820`) formats.
- No token format version bump needed. The pipe delimiter and field positions
  are unchanged.

---

## Files Changed (Summary)

| File | Change |
|------|--------|
| `src/net_detect.rs` | Add `detect_public_ipv6()`, update `detect_public_ip()` to prefer v6 |
| `src/wireguard.rs` | Bracket-format IPv6 endpoints in init() |
| `src/invite.rs` | Update `0.0.0.0` validation to also reject `[::]` |
| `src/open_invite.rs` | Same validation update |
| `src/api/node_routes.rs` | Add `parse_endpoint()` helper, update URL construction for IPv6 |
| `src/api/mod.rs` | Update `is_local_or_wg()` for IPv4-mapped IPv6 addresses |
| `src/main.rs` | Bind daemon to `[::]:port` (dual-stack) |
| `src/config.rs` | Update doc comment for `wg_endpoint` |

---

## Out of Scope

- **NAT traversal for IPv4-only users.** They must use `--wg-endpoint` with
  port forwarding or run on a host with a public IPv4.
- **STUN/TURN/relay.** Against project principles.
- **IPv6 firewall auto-configuration.** Out of scope; surface clear error
  messages instead.
- **Dual-stack invites** (encoding both v4 and v6 in one token). Keep it
  simple — one endpoint per invite.

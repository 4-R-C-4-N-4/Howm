# WireGuard P2P Connectivity
## Zero-Infrastructure NAT Traversal — Implementation Specification

---

## Table of Contents

1. [Overview](#1-overview)
2. [Connectivity Priority Ladder](#2-connectivity-priority-ladder)
3. [NAT Self-Characterisation](#3-nat-self-characterisation)
4. [Invite Gating and UX Behaviour](#4-invite-gating-and-ux-behaviour)
5. [Invite Flows by Tier](#5-invite-flows-by-tier)
6. [Out-of-Band (OOB) Exchange Payload](#6-out-of-band-oob-exchange-payload)
7. [Smart Port Selection](#7-smart-port-selection)
8. [Probe Protocol](#8-probe-protocol)
9. [Failure Handling](#9-failure-handling)
10. [Implementation Checklist](#10-implementation-checklist)

---

## 1. Overview

This document specifies how two peers establish a WireGuard tunnel without any
dedicated relay, STUN, or signalling infrastructure. Connectivity is attempted
in priority order: direct IPv6, reachable IPv4, then UDP hole-punching.

### Core Principles

- **No relay servers.** No TURN, no signalling service, no single point of
  failure. Every connection is direct between two peers.
- **IPv6 is the happy path.** Globally routable, no NAT. The current one-way
  invite flow works unmodified.
- **STUN is opt-in and offline.** Used only for NAT characterisation at install
  time (or on user request). Results cached to disk. Not part of the
  connection path.
- **Two-way OOB exchange is the worst-case fallback.** Only required when both
  peers are behind NAT with no IPv6. Tiers 1 and 2 use the existing one-way
  invite flow.

### Probe Port

All probe traffic uses **UDP port 4725**. This is a dedicated, fixed port for
Howm peer probing — separate from the WireGuard listen port (default 51820).

Rationale:
- Kernel WireGuard owns the WG UDP socket; userspace cannot demux on it.
- A separate port avoids collisions with Tailscale or other WireGuard users
  on the same network who share port 51820.
- Port 4725 is in the IANA registered range (1024-49151), unassigned, and
  unlikely to conflict with any running service.
- Fixed across all installations — both peers know it without exchanging
  anything.
- Overridable via `--probe-port` flag for rare conflicts.

---

## 2. Connectivity Priority Ladder

Attempt methods in strict order. Move to the next tier only after the previous
is confirmed unreachable or times out.

### Tier 1 — Direct IPv6

Both peers have a globally routable IPv6 address. The current one-way invite
flow works as-is: Alice creates a token, Bob calls Alice's daemon directly
over IPv6. No NAT traversal required.

**Invite model:** One-way (existing `howm://invite/` flow).
**Requirement:** Inviter's IPv6 firewall allows inbound UDP on WG port.

### Tier 2 — Reachable IPv4

At least one peer has a reachable public IPv4 (port forwarding, static IP,
open/full-cone NAT). The reachable peer acts as the inviter; the joiner
connects directly.

**Invite model:** One-way (existing `howm://invite/` flow).
**Requirement:** Inviter has port forwarding or open NAT.

### Tier 3 — UDP hole-punch (both behind NAT, no IPv6)

Both peers are behind NAT with no IPv6 available. Neither can receive
unsolicited inbound connections. Requires:
- NAT characterisation (from cached STUN results)
- Two-way OOB exchange (both peers share their connection info)
- Simultaneous bidirectional probing on port 4725

**Invite model:** Two-way OOB exchange.
**Requirement:** At least one peer is NOT behind symmetric NAT.

### Unreachable — surface failure to user

Symmetric NAT on both peers with no IPv6. Or strict firewall blocking all UDP.
Report `UNREACHABLE` with a clear explanation. Do not retry silently.

---

## 3. NAT Self-Characterisation

NAT characterisation is **opt-in** and runs **once at install time** (or when
the user explicitly requests it). Results are cached to disk, associated with
the current network identity.

### 3.1 When to Run

- **First install:** Prompt the user: "Would you like to detect your network
  configuration? This helps with connections when both peers are behind NAT."
- **On user request:** Settings page "Re-detect network" button.
- **Never automatically on startup.** The cached result is used until the
  network changes or the user re-runs detection.

### 3.2 Network Identity

Cache results keyed by network identity to invalidate when the user changes
networks:

```json
{
  "network_id": "<gateway_mac_or_bssid>",
  "network_label": "Home",
  "detected_at": 1742320000,
  "nat_type": "port_restricted",
  "external_ip": "203.0.113.5",
  "external_port": 4725,
  "observed_stride": 4,
  "mapping_lifetime_ms": 45000,
  "stun_servers_used": ["stun.l.google.com:19302", "stun.cloudflare.com:3478"]
}
```

On daemon start, compare the current gateway MAC/BSSID with the cached
`network_id`. If they differ, warn: "Network changed — NAT detection results
may be stale. Re-run detection from Settings."

### 3.3 Known Networks

Store multiple network profiles. The user can label them ("Home", "Office",
"Mobile hotspot"). When reconnecting to a known network, load the cached
profile automatically. In the future, known network profiles can be shared
with trusted peers so they know what to expect.

### 3.4 Public STUN Servers

Use at least two providers with distinct IP addresses for multi-destination
comparison.

| Provider | Host | Port | CHANGE-REQUEST |
|---|---|---|---|
| Google (primary) | `stun.l.google.com` | 19302 | No (RFC 5389) |
| Google (alt) | `stun1.l.google.com` | 19302 | No |
| Cloudflare | `stun.cloudflare.com` | 3478 | No |
| Twilio | `global.stun.twilio.com` | 3478 | No |
| stunprotocol.org | `stun.stunprotocol.org` | 3478 | Yes (RFC 3489) |

> Community-maintained fallback list:
> [pradt2/always-online-stun](https://github.com/pradt2/always-online-stun)

### 3.5 Test Battery

All tests use the **same local UDP socket bound to port 4725** so the NAT
mapping is consistent across measurements.

1. **Test 1 — baseline mapping.** Send STUN binding request to Server A.
   Record reflected external `(ext_ip_A, ext_port_A)`.

2. **Test 2 — IP restriction check.** Send CHANGE-REQUEST (change IP) to
   Server A. Success = full-cone. Timeout = at least address-restricted.
   (Requires RFC 3489 server; skip if unavailable.)

3. **Test 3 — port restriction check.** Send CHANGE-REQUEST (change port) to
   Server A. Success = address-restricted. Timeout = port-restricted.
   (Requires RFC 3489 server; skip if unavailable.)

4. **Test 4 — symmetric check.** Send binding request to Server B (different
   IP). Record `(ext_ip_B, ext_port_B)`. If `ext_port_A != ext_port_B`, NAT
   is symmetric.

5. **Test 5 — stride measurement.** Compute `stride = ext_port_B - ext_port_A`.
   If stable across multiple pairs, cache as `observed_stride`.

6. **Test 6 — mapping lifetime.** Re-send Test 1 after 30s, 60s, 120s without
   keepalive traffic. Record when the external port changes. Cache as
   `mapping_lifetime_ms`. Run in background; does not block the UI.

> **Note:** If only RFC 5389 servers are available (Google, Cloudflare),
> derive classification from Test 4 alone. Port mismatch = symmetric;
> port match = cone (type unknown; assume port-restricted as conservative
> default).

### 3.6 NAT Classification Logic

```
if ext_ip == local_ip:           -> OPEN (no NAT)
if test2_success:                -> FULL_CONE
if test3_success:                -> ADDRESS_RESTRICTED
if test4_port != test1_port:     -> SYMMETRIC
else:                            -> PORT_RESTRICTED
if all tests timeout:            -> UNKNOWN
```

### 3.7 NAT Classification Reference

| NAT type | Cone? | Open invite | Tier 3 punch | Notes |
|---|---|---|---|---|
| Open / no NAT | N/A | Always works | N/A | No NAT to traverse |
| Full cone | Yes | Reliable | Reliable | Any source can reach mapped port |
| Address-restricted | Yes | Ephemeral | Works | Must send to peer first |
| Port-restricted | Yes | Ephemeral | Works | Must send to peer's exact port |
| Symmetric | No | Disabled | Fails | Port changes per destination |
| Unknown | ? | Ephemeral | Best-effort | Conservative: treat as port-restricted |

### 3.8 Peer-to-Peer Connectivity Matrix

| Peer A \ Peer B | Open | Full cone | Addr-restr | Port-restr | Symmetric |
|---|---|---|---|---|---|
| **Open** | Tier 2 | Tier 2 | Tier 2 | Tier 2 | Tier 2 |
| **Full cone** | Tier 2 | Tier 2 | Tier 2 | Tier 2 | Tier 2 |
| **Addr-restr** | Tier 2 | Tier 2 | Tier 3 | Tier 3 | Tier 3 |
| **Port-restr** | Tier 2 | Tier 2 | Tier 3 | Tier 3 | UNREACHABLE |
| **Symmetric** | Tier 2 | Tier 2 | Tier 3 | UNREACHABLE | UNREACHABLE |

> When at least one peer is Open or Full cone, the reachable peer should be
> the inviter (Tier 2 one-way flow). Tier 3 two-way exchange is only needed
> when neither peer is directly reachable.

---

## 4. Invite Gating and UX Behaviour

### 4.1 Open Invite Feature Gate

Open invites encode a standing endpoint. Only valid for NAT types with a
stable reachable address.

| NAT type | Open invite | UX action |
|---|---|---|
| Open / no NAT | Enabled | No warning |
| Full cone | Enabled | No warning |
| Address-restricted | Enabled (ephemeral) | Show ephemeral warning |
| Port-restricted | Enabled (ephemeral) | Show ephemeral warning |
| Symmetric | **Disabled** | Hide feature; show tooltip |
| Unknown (no STUN run) | Enabled | Show "run NAT detection" suggestion |

### 4.2 Ephemeral Invite Warning

Display at invite creation time for restricted NAT types:

> **Your invite is time-sensitive**
>
> Your network reserves a port for incoming connections, but that reservation
> expires after approximately `{mapping_lifetime_human}`. Share this invite
> quickly.
>
> If your internet connection resets, your public IP may change and this
> invite will stop working.

### 4.3 Fresh STUN at Invite Creation

When creating an invite on a restricted NAT, re-run a single STUN binding
request to capture the freshest external port. The cached NAT characterisation
provides the type and stride; the fresh binding provides the current mapping.

Include both `external_port` and `observed_stride` in the invite so the
consuming peer can probe the preserved port first, then stride-predicted
ports.

---

## 5. Invite Flows by Tier

### 5.1 Tier 1 & 2 — One-Way Invite (Existing Flow)

No changes to the current `howm://invite/` and `howm://open/` token formats.
The existing pipe-delimited payload works for both IPv4 and IPv6 endpoints.

```
Alice creates invite -> token to Bob out-of-band
Bob redeems -> calls Alice's daemon directly (IPv6 or reachable IPv4)
Both configure WireGuard -> tunnel up
```

See `docs/IP6_INVITE.md` for IPv6-specific changes.

### 5.2 Tier 3 — Two-Way OOB Exchange

When both peers are behind NAT with no IPv6, the one-way invite is
insufficient because neither peer's daemon is reachable. The flow becomes:

```
Alice                                    Bob
  |                                        |
  | 1. Create invite (includes NAT info)   |
  | --- howm://invite/<alice_payload> ---> |
  |     (out-of-band: chat, QR, email)     |
  |                                        |
  |           2. Bob accepts, creates      |
  |              response with his info    |
  | <--- howm://accept/<bob_payload> ---   |
  |     (same out-of-band channel)         |
  |                                        |
  | 3. Both configure WireGuard locally    |
  |    (pubkeys, PSK, assigned IPs)        |
  |                                        |
  | 4. Both open UDP socket on port 4725   |
  |    Both begin simultaneous probing     |
  |    (see Section 8: Probe Protocol)     |
  |                                        |
  | 5. Probe succeeds — endpoint learned   |
  |    Update WG peer endpoint             |
  |    WireGuard handshake completes       |
  |                                        |
  | <========= WG tunnel up ============> |
  |                                        |
  | 6. Confirm via /node/info over WG      |
```

The `howm://accept/` token is a NEW token type. It contains Bob's connection
info (pubkey, candidates, NAT hints) and references Alice's invite (by nonce
or pubkey) so Alice can match it.

### 5.3 Accept Token Format

Same encoding as invite tokens (base64url, pipe-delimited) but with the
`howm://accept/` scheme prefix:

```
howm://accept/<base64url(payload)>
```

Payload fields:

| Field | Description |
|---|---|
| `inviter_pubkey` | Alice's WG pubkey (binds this accept to a specific invite) |
| `pubkey` | Bob's WG pubkey |
| `candidates` | Compact candidate list (see 6.1) |
| `probe_port` | 4725 (included for forward-compat) |
| `nat_type_hint` | Bob's NAT classification |
| `observed_stride` | Bob's port allocation stride |
| `session_mac_key` | 32-byte random key for probe authentication |
| `psk` | Pre-shared key (generated by Bob, or echoed from Alice's invite) |

### 5.4 Tier Selection Logic

At invite creation time, determine which tier applies:

```
if ipv6_endpoint is Some:
    -> Tier 1 (one-way invite, direct IPv6)
elif nat_type in [OPEN, FULL_CONE]:
    -> Tier 2 (one-way invite, reachable IPv4)
elif nat_type in [ADDRESS_RESTRICTED, PORT_RESTRICTED, UNKNOWN]:
    -> Tier 3 (two-way OOB exchange required)
    -> Include NAT metadata in invite token
    -> UI shows: "Your peer will need to send you an accept link back"
elif nat_type == SYMMETRIC:
    -> Block invite creation
    -> UI shows: "Your NAT type prevents direct connections. Enable IPv6
       or use a network with a different NAT type."
```

---

## 6. Out-of-Band (OOB) Exchange Payload

The OOB exchange provides shared ground truth before probing begins.

### 6.1 Compact Candidate Encoding

For the invite and accept tokens to remain shareable (chat, QR), candidates
are encoded compactly rather than as full JSON:

```
candidates = ip1:port:family[,ip2:port:family,...]
```

Example:
```
2001:db8::1:4725:6,203.0.113.5:4725:4,192.168.1.42:4725:4p
```

Where `4p` suffix = family 4, scope private. Keeps the token under ~300 bytes.

### 6.2 Candidate Ordering

Probe in this order:

1. IPv6 GUAs (all, one attempt each)
2. Public IPv4 (STUN-reflected or known)
3. RFC1918 private addresses (same-LAN hairpin detection)

Exclude: link-local (`fe80::`), loopback, multicast.

### 6.3 session_mac_key

For v1, use Option A: fresh 32-byte random key included in the accept token.
The OOB channel is already trusted (the user is manually copying links).

### 6.4 Timing Coordination

There is no `probe_start_at` timestamp. Both peers cannot coordinate a time
without a communication channel.

Instead, use **trigger-on-config**: both peers begin probing immediately when
they have each other's full config:

- **Alice** begins probing the moment she processes Bob's `howm://accept/`
  token.
- **Bob** begins probing the moment he sends his accept token (he already has
  Alice's info from the invite).

This means Bob starts first. Bob's outbound probes create NAT mappings. Alice
starts seconds/minutes later when she processes the accept token. Alice's
probes hit Bob's NAT mappings (which Bob keeps alive with periodic sends).

To handle the timing gap:
- The **first peer to start** sends probe packets at 1/second continuously
  until the other peer responds or a 5-minute timeout expires.
- The **second peer** sends probes at 200ms for rapid convergence once both
  are active.

---

## 7. Smart Port Selection

### 7.1 NAT Allocation Patterns

**Port preservation** — most common on home NAT. Internal port == external
port. Always probe the preserved port (4725) first.

**Sequential allocation** — when 4725 is taken, router increments by a fixed
stride. Probe `4725 + stride`, `4725 + 2*stride`, etc.

**Pseudorandom** — symmetric NATs. Unpredictable. Accept failure.

### 7.2 Probe Tier Schedule

All probes target the peer's public IPv4 address on varying ports:

| Tier | Ports to probe | Start after | Rationale |
|---|---|---|---|
| 1 | 4725 (preserved); OOB candidate ports | Immediately | Preservation is common |
| 2 | `4725 +/- 1..10`; `4725 + stride` | 0.5 s | Sequential allocation |
| 3 | 443, 4500, 3478, 8080 | 1.5 s | Firewall-friendly ports |
| 4 | Random sample from 1024-49151 | 3 s | Last resort |

> Send probes at 150-250 ms intervals within each tier. Not as a burst.
> Stop all tiers when a working endpoint is confirmed or hard timeout (15s).

### 7.3 Per-Peer Cache

Store NAT observations per peer for faster reconnection:

```json
{
  "peer_pubkey": "base64...",
  "last_external_port": 4725,
  "observed_stride": 4,
  "nat_behavior": "sequential",
  "mapping_lifetime_ms": 45000,
  "last_seen": 1742320000
}
```

On reconnection to the same peer, start tier 1 at `last_external_port +
observed_stride`. Cache expires after 24 hours or on network change.

---

## 8. Probe Protocol

### 8.1 Dedicated Probe Socket

The probe protocol runs on a **separate UDP socket** bound to port 4725. This
is independent of the WireGuard kernel socket. No demuxing required — all
traffic on port 4725 is probe traffic.

The daemon opens this socket at startup if Tier 3 NAT traversal is configured
or if a two-way invite exchange is in progress.

### 8.2 Packet Format

```
Probe packet (90 bytes):

  [0x00]      magic byte = 0xFE
  [0x01]      type: 0x01 = PROBE, 0x02 = PROBE_RESPONSE
  [0x02-0x11] session_nonce (16 bytes, from OOB exchange)
  [0x12-0x13] sequence number (uint16, big-endian)
  [0x14-0x27] from_pubkey_hash (20 bytes, SHA-1 of sender's WG pubkey)
  [0x28-0x37] observed_src_ip (16 bytes, IPv6 or IPv4-mapped; zeroed in PROBE)
  [0x38-0x39] observed_src_port (uint16, big-endian; zeroed in PROBE)
  [0x3A-0x59] MAC (32 bytes, HMAC-SHA256 over bytes 0x00-0x39,
                    key = session_mac_key)
```

### 8.3 Probe Phase Sequence

Both peers execute simultaneously (see 6.4 for timing):

1. **Same-LAN check.** Compare private IP candidates. If both share a
   RFC1918 /24, try direct LAN connection on port 4725. Skip NAT punch.

2. **IPv6 attempt.** If any IPv6 candidates exist, try them first. On
   success, skip IPv4 probing entirely.

3. **Bidirectional probing.** Send `PROBE` packets to the peer's public IPv4
   across all port tiers (Section 7.2). `observed_src` fields are zeroed.

4. **Reflection.** On receiving a verified `PROBE`, reply with
   `PROBE_RESPONSE` with `observed_src` set to the packet's source IP:port.

5. **Endpoint learning.** On receiving a verified `PROBE_RESPONSE`, extract
   `observed_src`. This is our own external mapping as seen by the peer.
   Both peers now know each other's external endpoint.

6. **Convergence.** Focus probes on confirmed endpoints. Continue at 200ms
   until WireGuard handshake completes.

7. **WireGuard handshake.** Update `wg set howm0 peer <pubkey> endpoint
   <confirmed_ip:wg_port>`. WireGuard initiates handshake. Continue probe
   keepalives to maintain NAT mapping.

### 8.4 Reflection Authentication

All packets are HMAC-SHA256 verified using `session_mac_key`. Drop anything
that fails verification. This prevents on-path injection of false
`observed_src` values.

### 8.5 Keepalive

After WireGuard tunnel is established, NAT mapping must be maintained:

- WireGuard `persistent-keepalive` = `max(10, (mapping_lifetime_ms / 1000) - 10)`
- Default 25 seconds if `mapping_lifetime_ms` is unknown.
- Probe keepalives on port 4725 every 25 seconds as a secondary channel.

---

## 9. Failure Handling

### 9.1 Failure Modes

| Failure | Detection | Action |
|---|---|---|
| Symmetric NAT (local) | STUN test 4: port differs | Block Tier 3 at invite creation |
| Symmetric NAT (remote) | OOB `nat_type_hint` = symmetric | Warn at accept time |
| Both symmetric, no IPv6 | Both hints symmetric | UNREACHABLE |
| Strict firewall | All probe tiers timeout | UNREACHABLE after 15s |
| Mapping expired during handshake | WG handshake timeout | Retry probe phase once |
| Accept token never received | 5-minute probe timeout | Surface "waiting for peer" |

### 9.2 UNREACHABLE State

Surface to the user with:
- Which tiers were attempted
- Local NAT type
- Remote NAT type hint (if available)
- Specific failure reason

```
Connection failed: both peers behind port-restricted NAT, no IPv6.
Tried: direct IPv6 (no address), IPv4 direct (NAT), UDP hole-punch (15s timeout).
Your NAT: port_restricted | Peer NAT: symmetric

Suggestion: Enable IPv6, or connect from a different network.
```

### 9.3 Retry Policy

- Do not auto-retry a failed punch without user action.
- On network change (detected via gateway MAC), invalidate cache and suggest
  re-running NAT detection.
- If peer cache shows previous success, try cached port first on reconnect.

---

## 10. Implementation Checklist

### NAT Detection (opt-in, cached)

- [ ] Settings UI: "Detect network configuration" button
- [ ] Prompt on first install: "Would you like to detect NAT type?"
- [ ] STUN battery runs on port 4725 UDP socket
- [ ] Results cached to `{data_dir}/nat_profile.json` keyed by network ID
- [ ] Network ID derived from gateway MAC or BSSID
- [ ] Multiple network profiles supported ("Home", "Office", etc.)
- [ ] Mapping lifetime test (Test 6) runs in background, does not block UI
- [ ] NAT type included in OOB payload as `nat_type_hint`

### Invite Flow

- [ ] Tier selection logic at invite creation (IPv6 > reachable IPv4 > Tier 3)
- [ ] `howm://accept/` token type implemented for Tier 3 two-way exchange
- [ ] Accept token includes: pubkey, candidates, probe_port, nat_type_hint,
      observed_stride, session_mac_key, inviter_pubkey reference
- [ ] Open invite disabled (hidden) for symmetric NAT
- [ ] Ephemeral warning shown for restricted NAT types
- [ ] Fresh STUN binding at invite creation for restricted NAT

### Probe Protocol

- [ ] Dedicated UDP socket on port 4725 (separate from WireGuard)
- [ ] `--probe-port` flag for override (default 4725)
- [ ] Probe packet format: 90 bytes, HMAC-SHA256 authenticated
- [ ] Same-LAN detection before NAT punch
- [ ] IPv6 attempted before IPv4 punch
- [ ] Port probe tiers execute in order (7.2) with defined timeouts
- [ ] Probe reflection: `PROBE_RESPONSE` echoes observed source
- [ ] Endpoint learning: update WG peer endpoint from confirmed probe
- [ ] Timing: first peer probes at 1/s; both at 200ms once active

### Post-Connection

- [ ] Per-peer NAT cache written after successful Tier 3 connection
- [ ] `persistent-keepalive` set from `mapping_lifetime_ms`
- [ ] Probe keepalives on port 4725 every 25s after tunnel up
- [ ] `UNREACHABLE` surfaced with tier attempts and failure reason within 15s
- [ ] No silent auto-retry after failure

### Config

- [ ] `probe_port: u16` in Config (default 4725)
- [ ] `nat_profile.json` schema and read/write
- [ ] `peer_nat_cache.json` schema and read/write

---

## Appendix: Port 4725

- IANA status: unassigned (as of 2026-03)
- Range: registered (1024-49151) — not ephemeral, not well-known
- No known conflicts with Tailscale, WireGuard, or major services
- Fixed across all Howm installations — no coordination needed

---

*End of specification*

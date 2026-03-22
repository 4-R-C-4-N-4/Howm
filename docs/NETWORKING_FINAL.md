# Howm Networking — Peer Connection Strategy
## Final Implementation Plan

---

## 1. Design Philosophy

Howm is a social mesh. Every connection is direct between two peers over
WireGuard. There are no Howm-operated servers — no STUN infrastructure, no
DERP relays, no signalling service.

The network only needs **one peer with a public inbound path** (IPv6, port
forwarding, open NAT) to make both directions of the exchange trivial. That's
the happy path and covers the majority of real-world connections. Everything
else in this document handles the shrinking-but-real case where neither peer
has a public inbound path.

### Principles

- **IPv6 is the future, design for it now.** Global adoption is ~45% and
  climbing. Every Howm node should detect and prefer IPv6 automatically.
- **One reachable peer is enough.** If either side can receive inbound UDP,
  use the existing one-way invite flow. No complexity needed.
- **Hole punching is the backup, not the default.** Only invoked when both
  peers are behind NAT with no IPv6. Keep the implementation lean.
- **Peer relay is the last resort.** Already implemented in p2pcd core. A
  mutual peer with connectivity bridges two peers who can't punch through.
  No external infrastructure, no DERP — just the mesh helping itself.
- **Fail clearly, never silently.** If connection isn't possible, tell the
  user exactly why and what they can do about it.

---

## 2. Connectivity Tiers

Attempt in strict order. Fall through only on confirmed failure or timeout.

### Tier 1 — Direct (one peer has public inbound)

Covers: IPv6 on either side, port-forwarded IPv4, static IP, open/full-cone
NAT. This is the existing `howm://invite/` one-way flow. No changes needed
to the invite protocol itself.

**How it works:**
```
Alice (reachable) creates invite → token to Bob out-of-band
Bob redeems → calls Alice's daemon directly
Both configure WireGuard → tunnel up
```

**Detection:** At invite creation, the node checks:
1. Do I have a globally routable IPv6 address?
2. Did STUN report my NAT as OPEN or FULL_CONE?
3. Is my external IP:port reachable (quick self-probe)?

If any are true → Tier 1. One-way invite. Done.

**IPv6 considerations:**
- Detect GUA (Global Unicast Address, `2000::/3`) on all interfaces.
- Exclude link-local (`fe80::`), ULA (`fd00::/8`), loopback.
- Prefer IPv6 over IPv4 when both are available — include both in the
  invite token, let the joiner try IPv6 first.
- Note: some ISPs firwall inbound UDP on IPv6 by default. The self-probe
  catches this — if the probe fails, don't advertise IPv6 as reachable.

**Estimated scope:** ~300 lines. IPv6 detection, self-probe, tier selection
logic at invite creation.

### Tier 2 — UDP Hole Punch (both behind NAT, no public inbound)

Covers: Both peers behind cone NAT (address-restricted or port-restricted)
with no IPv6. Neither can receive unsolicited inbound. Requires a two-way
out-of-band exchange so both peers know each other's connection info before
probing begins.

**How it works:**
```
Alice                                    Bob
  |                                        |
  | 1. Create invite (includes NAT info)   |
  | --- howm://invite/<alice_payload> --->  |
  |                                         |
  |           2. Bob accepts, creates       |
  |              response with his info     |
  | <--- howm://accept/<bob_payload> ---    |
  |                                         |
  | 3. Both configure WireGuard locally     |
  |    (pubkeys, PSK, assigned IPs)         |
  |                                         |
  | 4. Both begin probing on UDP 4725      |
  |    Bob starts immediately on send       |
  |    Alice starts on accept processing    |
  |                                         |
  | 5. Probe succeeds — endpoint learned    |
  |    Update WG peer endpoint              |
  |    WireGuard handshake completes        |
  |                                         |
  | <========= WG tunnel up ===========>   |
```

**New token type:** `howm://accept/<base64url(payload)>` — Bob's response
containing his pubkey, candidates, NAT hint, and session MAC key. References
Alice's invite by her pubkey.

**Probe protocol (simplified):**
- Dedicated UDP socket on port 4725 (separate from WireGuard).
- 90-byte packet: magic, type (PROBE/PROBE_RESPONSE), session nonce,
  sequence number, sender pubkey hash, observed source IP:port (zeroed in
  PROBE, filled in RESPONSE), HMAC-SHA256 over the whole thing.
- Probing targets the peer's public IP on port 4725 first (port preservation
  is the most common NAT behavior), then 4725 ± stride if stride was
  observed during STUN, then 4725 ± 1..10 for sequential allocators.
- That's it. No firewall-friendly port guessing, no random sampling. If
  the above doesn't work within 15 seconds, it's not going to.
- Timing: first peer (Bob) probes at 1/s. Second peer (Alice) probes at
  200ms once active. Both stop on confirmed response or 15s timeout.

**Estimated scope:** ~800 lines. Accept token, probe socket, probe
send/receive loop, HMAC auth, endpoint learning, WG endpoint update.

### Tier 3 — Peer Relay (hole punch failed or symmetric NAT)

Covers: Both peers behind symmetric NAT, or hole punch timed out, or any
other case where direct connectivity failed. Uses p2pcd's existing relay
capability — a mutually connected peer forwards traffic.

**How it works:**
- When Tier 2 fails (or is known to be impossible because one/both peers
  report symmetric NAT), the node checks its existing peer list for a
  mutual contact who is connected to both Alice and Bob.
- The relay peer forwards WireGuard UDP packets between the two. Traffic
  is still end-to-end encrypted by WireGuard — the relay sees only opaque
  UDP payloads.
- Relay is transparent to the application layer. From the perspective of
  capabilities and the bridge, it's just another peer connection.

**Relay discovery:**
- At invite creation time, if the node detects it's behind symmetric NAT,
  include a `relay_candidates` field in the invite: a list of peer pubkeys
  the inviter is currently connected to.
- The joiner checks the list against their own peer list. Any overlap is a
  potential relay.
- If no overlap, surface this to the user: "No direct path found and no
  mutual peers to relay through. You'll need to connect through a mutual
  friend first, or one of you needs to enable IPv6 / port forwarding."

**This is already mostly built.** The relay forwarding logic is in p2pcd
core. What's new is the discovery mechanism (relay_candidates in the invite)
and the fallback logic that triggers it.

**Estimated scope:** ~400 lines. Relay candidate discovery, fallback
trigger from failed Tier 2, relay negotiation messages.

### Unreachable — Clear Failure

Both behind symmetric NAT, no IPv6, no mutual peers for relay. Surface to
the user with:
- Which tiers were attempted and why they failed
- Local NAT type
- Remote NAT type (if known from OOB exchange)
- Actionable suggestions: "Enable IPv6", "Port forward UDP 51820",
  "Connect to a mutual friend first who can relay"

---

## 3. NAT Self-Characterization

Lightweight STUN-based detection. Runs once on user request (prompted at
first install). Results cached. Not part of any connection path — purely
informational to gate invite behavior and inform the user.

### When to Run

- **First install:** "Would you like to detect your network type? This
  helps Howm choose the best connection method."
- **On user request:** Settings → "Re-detect network"
- **At invite creation (restricted NAT only):** Single fresh STUN binding
  to get current external port. Uses cached NAT type, just refreshes the
  mapping.

### Test Battery

All tests use a single UDP socket bound to port 4725.

1. **Baseline mapping.** STUN binding request to Server A. Record
   `(ext_ip_A, ext_port_A)`.

2. **Symmetric check.** STUN binding request to Server B (different IP).
   Record `(ext_ip_B, ext_port_B)`. If ports differ → symmetric NAT.

3. **Stride.** `stride = ext_port_B - ext_port_A`. If stable across
   repeated tests, cache it.

That's the minimum viable battery. Two STUN requests to two different
servers. Classifies into:

```
if ext_ip == local_ip        → OPEN (no NAT)
if ext_port_A == ext_port_B  → CONE (port-preserving, punchable)
if ext_port_A != ext_port_B  → SYMMETRIC (not punchable)
if all tests timeout         → UNKNOWN (treat as cone, attempt punch)
```

We don't need to distinguish full-cone vs address-restricted vs
port-restricted for our purposes. What matters is: **cone (punchable) vs
symmetric (not punchable).** The probe protocol handles both restricted
subtypes the same way.

### STUN Servers

Use public servers. No Howm infrastructure.

| Provider | Host | Port |
|---|---|---|
| Google | `stun.l.google.com` | 19302 |
| Cloudflare | `stun.cloudflare.com` | 3478 |

Two servers with distinct IPs is sufficient for the symmetric check.

### Cache

```json
{
  "detected_at": 1742320000,
  "nat_type": "cone",
  "external_ip": "203.0.113.5",
  "external_port": 4725,
  "observed_stride": 0,
  "stun_servers": ["stun.l.google.com:19302", "stun.cloudflare.com:3478"]
}
```

Stored at `{data_dir}/nat_profile.json`. Single profile. No network
identity tracking, no multi-profile — that's premature. If the user
changes networks and things break, they re-run detection.

**Estimated scope:** ~400 lines. Minimal STUN client (binding request
only — not a full RFC 5389 impl), classification logic, cache read/write.

---

## 4. Invite Token Changes

### Existing: `howm://invite/`

Add optional fields for Tier 2+ scenarios:

| Field | When included | Purpose |
|---|---|---|
| `ipv6_candidates` | Always (if available) | GUA addresses for Tier 1 |
| `nat_type` | When cone or symmetric | Tells joiner what to expect |
| `external_port` | When behind NAT | Fresh STUN-reflected port |
| `observed_stride` | When stride != 0 | Helps joiner's probe targeting |
| `relay_candidates` | When symmetric NAT | Pubkeys of connected peers |
| `probe_port` | Always (for forward-compat) | Default 4725 |

Existing fields (pubkey, endpoint, PSK, HMAC, etc.) unchanged.

### New: `howm://accept/`

Tier 2 two-way exchange only. Same encoding (base64url, pipe-delimited).

| Field | Purpose |
|---|---|
| `inviter_pubkey` | Binds accept to specific invite |
| `pubkey` | Bob's WG pubkey |
| `candidates` | Bob's IP:port candidates (compact) |
| `nat_type` | Bob's NAT classification |
| `external_port` | Bob's STUN-reflected port |
| `observed_stride` | Bob's allocation stride |
| `session_mac_key` | 32-byte random, for probe HMAC |
| `psk` | Pre-shared key (echoed from invite) |
| `probe_port` | 4725 |

### Candidate Encoding

Compact, for shareability:
```
2001:db8::1:4725:6,203.0.113.5:4725:4,192.168.1.42:4725:4p
```
Where `4p` = IPv4 private. Keeps tokens under ~300 bytes.

---

## 5. UX Behavior

### Invite Creation

```
if has_public_inbound (IPv6 GUA, or STUN says OPEN/CONE with self-probe OK):
    → Standard one-way invite. No special UI.

elif nat_type == CONE:
    → Two-way invite required.
    → UI: "Your peer will need to send you a response link back.
           Share your invite, then paste their response here."

elif nat_type == SYMMETRIC:
    → Check relay_candidates (connected peers).
    → If candidates exist:
        → Include them in invite. UI: "Direct connection isn't possible
          from your network. Connection will be relayed through a mutual
          peer if available."
    → If no candidates:
        → UI: "Your network type prevents direct connections and you have
          no connected peers to relay through. Enable IPv6 or connect
          from a different network."

elif nat_type == UNKNOWN (no detection run):
    → Allow invite creation (assume cone, attempt Tier 2).
    → UI: "Tip: Run network detection in Settings for better connection
          reliability."
```

### Invite Redemption (Joiner Side)

```
1. Parse invite token.
2. Try IPv6 candidates first (if any). Direct connect → done.
3. Try IPv4 endpoint directly. If reachable → done.
4. If invite includes nat_type, enter Tier 2:
   → Generate accept token.
   → UI: "This peer is behind NAT. Copy this response link back to them:
          howm://accept/..."
   → Begin probing on send.
5. If Tier 2 times out and relay_candidates present:
   → Check for mutual peer. Negotiate relay. → done.
6. If all fail → UNREACHABLE with explanation.
```

---

## 6. Implementation Order

### Phase 1: IPv6 Detection + Preference (~300 lines)

Everything needed so Tier 1 works reliably for IPv6 users.

- [ ] Enumerate GUA IPv6 addresses on all interfaces
- [ ] Filter out link-local, ULA, loopback
- [ ] Include IPv6 candidates in invite tokens
- [ ] Joiner tries IPv6 candidates before IPv4
- [ ] Self-probe: send a UDP packet to own IPv6 via external path to
      verify inbound works (or use a simple probe to the invite endpoint
      after binding)

### Phase 2: NAT Characterization (~400 lines)

Inform the user and gate invite behavior. No connection logic yet.

- [ ] Minimal STUN client (RFC 5389 binding request/response only)
- [ ] Two-server symmetric check (Google + Cloudflare)
- [ ] Stride measurement
- [ ] Classification: OPEN / CONE / SYMMETRIC / UNKNOWN
- [ ] Cache to `nat_profile.json`
- [ ] Fresh STUN binding at invite creation (for restricted NAT)
- [ ] Settings UI hook: "Detect network type"
- [ ] First-install prompt

### Phase 3: Two-Way Exchange + Hole Punch (~800 lines)

The core of Tier 2.

- [ ] `howm://accept/` token type: generate, parse, validate
- [ ] Accept token payload: all fields from Section 4
- [ ] Invite token: add nat_type, external_port, stride, probe_port
- [ ] Dedicated UDP probe socket on port 4725
- [ ] `--probe-port` CLI flag
- [ ] Probe packet format (90 bytes, HMAC-SHA256)
- [ ] Probe send loop: target preserved port, then stride offsets, then
      ± 1..10
- [ ] Probe receive + HMAC verification
- [ ] PROBE_RESPONSE reflection (echo observed source)
- [ ] Endpoint learning from PROBE_RESPONSE
- [ ] `wg set` endpoint update on confirmed probe
- [ ] Timing: first peer 1/s, second peer 200ms, 15s hard timeout
- [ ] Same-LAN shortcut: if both have RFC1918 in same /24, try direct
      on 4725 first

### Phase 4: Peer Relay Fallback (~400 lines)

Wire up existing p2pcd relay as Tier 3.

- [ ] `relay_candidates` field in invite token (list of connected peer
      pubkeys)
- [ ] Joiner checks relay_candidates against own peer list for overlap
- [ ] Relay negotiation: ask mutual peer to forward for a session
- [ ] Fallback trigger: Tier 2 timeout or symmetric NAT detected
- [ ] Relay teardown when direct connection later becomes possible
      (e.g., network change)

### Phase 5: UX Polish

- [ ] Invite creation flow with tier-appropriate messaging
- [ ] Joiner flow with step-by-step guidance for two-way exchange
- [ ] UNREACHABLE error display with attempted tiers and suggestions
- [ ] NAT type display in settings/node info

---

## 7. What We're NOT Building (and why)

| Feature | Reason to skip |
|---|---|
| DERP relay servers | External infrastructure. Against core principles. Peer relay covers this. |
| Smart port selection (firewall-friendly ports, random sampling) | Diminishing returns. Preserved + stride covers the realistic cases. |
| Known network profiles (Home, Office, etc.) | Premature. Single cached profile is sufficient until proven otherwise. |
| Per-peer NAT cache | Optimization. Build after we have real users and real reconnection data. |
| Mapping lifetime measurement | Complex background test for a keepalive optimization. Use 25s default. |
| CHANGE-REQUEST tests (RFC 3489) | Requires specific STUN servers. Symmetric check from two servers is enough. |
| Network identity (gateway MAC/BSSID) | Complexity for auto-invalidation. User re-runs detection manually. |

---

## 8. Open Questions

1. **Self-probe mechanism.** How do we verify our own IPv6 is reachable
   for inbound UDP without a dedicated test server? Options: (a) probe
   own address from a different socket — won't cross the NAT/firewall,
   (b) ask an existing peer to send a test packet back, (c) just try and
   let the joiner fall through to Tier 2 if IPv6 fails. Leaning toward
   (c) — keep it simple, let the tier ladder handle it.

2. **Accept token delivery UX.** The two-way exchange requires the joiner
   to send a link BACK to the inviter. On mobile this is fine (copy/paste
   in the same chat). On desktop it's clunkier. Should we explore
   QR-code-based exchange for in-person scenarios? Camera → scan →
   auto-accept could make Tier 2 feel seamless.

3. **Relay consent.** Should the relay peer be asked before being included
   as a relay_candidate? Options: (a) opt-in setting "allow relay through
   me", (b) always allow but rate-limit, (c) ask at relay negotiation
   time. Leaning toward (a) with default on — relay is lightweight and
   pro-social, but users should be able to opt out.

4. **Probe port conflicts.** 4725 is unassigned but could conflict with
   something on a user's machine. The `--probe-port` flag handles this,
   but if the port is overridden, both peers need to know. Should the
   probe port always be in the token even when default? (Current answer:
   yes, for forward-compat.)

5. **Symmetric-to-cone punchability.** The original spec marks
   symmetric + port-restricted as UNREACHABLE. In practice, if the cone
   side sends first, the symmetric peer's response WILL come back to the
   cone peer's mapped port. This is a known technique (the cone peer
   "opens" for the symmetric peer). Should we attempt this before falling
   to relay? It would expand the punchable matrix at the cost of ~50
   lines in the probe logic.

6. **Invite token backward compatibility.** Adding fields to the invite
   token means older nodes can't parse newer tokens. Strategy: (a) version
   byte at the start of the payload, (b) older nodes ignore unknown
   trailing fields, (c) new fields are all optional and absence means
   "Tier 1 only." Leaning toward (c).

7. **WireGuard port vs probe port.** After hole punch succeeds on 4725,
   we learn the peer's external IP. But WireGuard listens on 51820 (or
   configured port). The NAT mapping for 4725 doesn't help with 51820.
   Options: (a) run WireGuard on 4725 too (avoids the problem but limits
   to one WG interface), (b) after probe success, both peers send a burst
   to each other's WG port to open that mapping too, (c) use the probe
   channel to relay the first few WG handshake packets until the NAT
   mapping for the WG port is established. This needs a clear answer
   before Phase 3 implementation.

8. **Relay bandwidth / abuse.** Relayed traffic is full WireGuard
   throughput through a third peer's connection. For text/social-feed
   this is negligible, but video or file transfer could be significant.
   Should relay connections have a bandwidth cap or be limited to
   signaling-only (help establish the punch, then drop)?

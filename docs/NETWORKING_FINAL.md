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
- **WireGuard IS the probe.** No dedicated probe port or protocol. The WG
  handshake itself punches the NAT hole. STUN gives us the external IP,
  the OOB exchange gives us port candidates, and WG does the rest.
- **Peer relay is signaling only.** A mutual peer helps exchange endpoint
  info (like a STUN server on the mesh). No sustained traffic forwarding.
  Relay is opt-in, off by default.
- **Fail clearly, never silently.** If connection isn't possible, tell the
  user exactly why and what they can do about it.

---

## 2. WireGuard Port

Howm uses a default WG listen port of **41641** — distinct from the standard
WireGuard default (51820) to avoid conflicts with Tailscale or other WG
instances on the same machine.

- If 41641 is occupied at startup, Howm tries 41642, 41643, ... up to 41650
  before failing with a clear error.
- The actually-bound port is what goes into invite tokens. If the user
  configures a custom port, that's used instead.
- Config: `wireguard.listen_port` (default 41641).

There is no separate probe port. All NAT traversal happens on the WG port
itself — the WG handshake IS the hole punch.

---

## 3. Connectivity Tiers

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
2. Did STUN report my NAT as OPEN or CONE?

If either is true → Tier 1. One-way invite. Done.

**IPv6 considerations:**
- Detect GUA (Global Unicast Address, `2000::/3`) on all interfaces.
- Exclude link-local (`fe80::`), ULA (`fd00::/8`), loopback.
- Prefer IPv6 over IPv4 when both are available — include both in the
  invite token, let the joiner try IPv6 first.
- If IPv6 inbound fails at redemption time, the joiner falls through to
  Tier 2 automatically. No self-probe needed.

**Estimated scope:** ~300 lines. IPv6 detection, tier selection logic at
invite creation.

### Tier 2 — UDP Hole Punch (both behind NAT, no public inbound)

Covers: Both peers behind cone NAT (address-restricted or port-restricted)
with no IPv6. Neither can receive unsolicited inbound. Also covers
**symmetric + cone** pairs — the cone peer sends first, creating a NAT
mapping that the symmetric peer's response routes back through.

Requires a two-way out-of-band exchange so both peers know each other's
connection info before punching.

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
  | 4. Both set WG peer endpoint to         |
  |    best-guess ext_ip:wg_port            |
  |    WG handshake attempts punch NAT      |
  |                                         |
  | 5. If preserved port fails, rotate      |
  |    through stride offsets               |
  |                                         |
  | <========= WG tunnel up ===========>   |
```

**No dedicated probe port or protocol.** The WG handshake itself is the
hole punch:

- STUN tells each peer their external IP.
- The OOB exchange shares external IP + WG listen port + stride.
- Both peers do `wg set peer <pubkey> endpoint <ext_ip>:<port>`.
- WireGuard's handshake initiation packets punch the NAT holes.
- For port-preserving NATs (the common case), this works on the first try.
- For sequential allocators, rotate the endpoint through stride offsets
  every 500ms: base, base+stride, base-stride, base+2*stride, etc.
- Hard timeout: 15 seconds. If no WG handshake completes, fall to Tier 3.

**Symmetric + cone pairs:** When one peer reports symmetric NAT in the OOB
exchange, the cone peer sends first. The cone peer's NAT mapping is
predictable (port-preserving), so the symmetric peer's WG response routes
back through it. The cone peer just needs to set its endpoint to the
symmetric peer's STUN-reflected IP:port and initiate. ~50 lines of extra
logic in the endpoint rotation: "if peer is symmetric, I initiate and wait
rather than both trying simultaneously."

**Timing:**
- Bob begins WG handshake attempts immediately after sending his accept
  token (he already has Alice's info from the invite).
- Alice begins when she processes Bob's accept token.
- Bob probes at 1/s (he's early, keeping NAT mappings alive). Alice probes
  at 200ms once active (rapid convergence). Both stop on successful WG
  handshake or 15s timeout.

**New token type:** `howm://accept/<base64url(payload)>` — Bob's response
containing his pubkey, candidates, NAT hint. References Alice's invite by
her pubkey. Shared via the same OOB channel as the original invite (chat,
email — whatever the users are already talking on).

**Estimated scope:** ~500 lines. Accept token, endpoint rotation loop,
symmetric-cone logic, WG handshake monitoring.

### Tier 3 — Peer Relay Signaling (hole punch failed or both symmetric)

Covers: Both peers behind symmetric NAT, or hole punch timed out. Uses a
mutually connected peer as a **signaling relay** — NOT a traffic relay.

The relay peer acts as a matchmaker: it passes endpoint info between the
two peers so they can attempt a direct WG connection. This is STUN-over-mesh.

**How it works:**
```
Alice                     Relay (Carol)                    Bob
  |                            |                            |
  | "I want to reach Bob.      |                            |
  |  My ext IP:port is X:Y"    |                            |
  | -------- signal --------> |                            |
  |                            | "Alice wants to reach you. |
  |                            |  Her ext IP:port is X:Y"   |
  |                            | -------- signal --------> |
  |                            |                            |
  |                            | <------- signal --------- |
  |                            | "Bob's ext IP:port is A:B" |
  | <------- signal ---------- |                            |
  |                            |                            |
  | Both set WG endpoints and attempt direct handshake      |
  | <=============== WG tunnel up ===================>     |
```

**No sustained traffic.** Carol exchanges a few small messages (< 1KB
total) and she's done. The actual WG connection is direct between Alice
and Bob. Carol never sees their traffic.

**Relay discovery:**
- At invite creation, if the node detects it's behind symmetric NAT (or
  hole punch just failed), include `relay_candidates` in the invite: a
  list of peer pubkeys the inviter is currently connected to.
- The joiner checks the list against their own peer list. Any overlap is
  a potential relay.
- If no overlap: "No direct path found and no mutual peers to help.
  Connect to a mutual friend first, or enable IPv6 / port forwarding."

**Relay consent:** Relay is **off by default**. Users opt in via config:
`network.allow_relay: true`. This is a conscious decision — relaying means
your node is doing work on behalf of others, and users should understand
what that means before enabling it. The setting is surfaced in the UI
with a clear explanation.

**When it doesn't work:** Symmetric + symmetric with no mutual peer for
signaling. Even with signaling, symmetric + symmetric is genuinely
UNREACHABLE — both sides have unpredictable port mappings, so even
knowing each other's external IP doesn't help. This is a small and
shrinking demographic (both on CGNAT, no IPv6, no mutual friends).
Let it be UNREACHABLE with a clear explanation.

**Estimated scope:** ~300 lines. Relay candidate discovery, signaling
message types (3 messages: request, offer, exchange), relay negotiation
over existing p2pcd channels, fallback trigger.

### Unreachable — Clear Failure

Both behind symmetric NAT, no IPv6, no mutual peers for relay signaling.
Surface to the user with:
- Which tiers were attempted and why they failed
- Local NAT type
- Remote NAT type (if known from OOB exchange)
- Actionable suggestions: "Enable IPv6", "Port forward UDP 41641",
  "Connect to a mutual friend first who can relay signaling"

---

## 4. NAT Self-Characterization

Lightweight STUN-based detection. Runs once on user request (prompted at
first install). Results cached. Not part of any connection path — purely
informational to gate invite behavior and inform the user.

### When to Run

- **First install:** "Would you like to detect your network type? This
  helps Howm choose the best connection method."
- **On user request:** Settings → "Re-detect network"
- **At invite creation (behind NAT):** Single fresh STUN binding to get
  current external IP:port. Uses cached NAT type, just refreshes the
  mapping.

### Test Battery

All tests use the WG listen socket (no separate port needed).

1. **Baseline mapping.** STUN binding request to Server A from the WG
   port. Record `(ext_ip_A, ext_port_A)`.

2. **Symmetric check.** STUN binding request to Server B (different IP)
   from the same socket. Record `(ext_ip_B, ext_port_B)`. If ports
   differ → symmetric NAT.

3. **Stride.** `stride = ext_port_B - ext_port_A`. If stable across
   repeated tests, cache it.

Two STUN requests. Classifies into:

```
if ext_ip == local_ip        → OPEN (no NAT)
if ext_port_A == ext_port_B  → CONE (port-preserving, punchable)
if ext_port_A != ext_port_B  → SYMMETRIC (not directly punchable)
if all tests timeout         → UNKNOWN (treat as cone, attempt punch)
```

We don't need to distinguish full-cone vs address-restricted vs
port-restricted. What matters is: **cone (punchable) vs symmetric (not
punchable).** The WG handshake handles restricted subtypes the same way.

**Important:** STUN requests go out from the WG UDP socket. This means
the STUN server sees the same NAT mapping that WG traffic will use. No
separate probe socket needed. The kernel WG module doesn't own the socket
until `wg set` configures a peer — before that, or between peers, the
socket is available for STUN. For implementations where the kernel owns
the socket, send STUN from a socket bound to WG port + 1 and adjust
stride accordingly.

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
  "external_port": 41641,
  "observed_stride": 0
}
```

Stored at `{data_dir}/nat_profile.json`. Single profile. If the user
changes networks and things break, they re-run detection.

**Estimated scope:** ~400 lines. Minimal STUN client (binding request
only — not a full RFC 5389 impl), classification logic, cache read/write.

---

## 5. Invite Token Changes

### Existing: `howm://invite/`

Add optional fields for Tier 2+ scenarios:

| Field | When included | Purpose |
|---|---|---|
| `ipv6_candidates` | Always (if available) | GUA addresses for Tier 1 |
| `nat_type` | When behind NAT | Tells joiner what to expect |
| `external_ip` | When behind NAT | STUN-reflected public IP |
| `external_port` | When behind NAT | STUN-reflected port (= WG port mapping) |
| `observed_stride` | When stride != 0 | Helps joiner's endpoint rotation |
| `wg_port` | Always | Actual bound WG listen port |
| `relay_candidates` | When symmetric NAT | Pubkeys of connected peers for signaling |

Existing fields (pubkey, endpoint, PSK, HMAC, etc.) unchanged.

### New: `howm://accept/`

Tier 2 two-way exchange only. Same encoding (base64url, pipe-delimited).

| Field | Purpose |
|---|---|
| `inviter_pubkey` | Binds accept to specific invite |
| `pubkey` | Bob's WG pubkey |
| `ipv6_candidates` | Bob's GUA addresses (if any) |
| `external_ip` | Bob's STUN-reflected public IP |
| `external_port` | Bob's STUN-reflected port |
| `wg_port` | Bob's actual WG listen port |
| `nat_type` | Bob's NAT classification |
| `observed_stride` | Bob's allocation stride |
| `psk` | Pre-shared key (echoed from invite) |

### Candidate Encoding

Compact, for shareability:
```
2001:db8::1:6,203.0.113.5:41641:4
```
Port is the WG listen port. Family suffix: `6` for IPv6, `4` for IPv4.
Keeps tokens short.

---

## 6. UX Behavior

### Invite Creation

```
if has_public_inbound (IPv6 GUA, or STUN says OPEN/CONE):
    → Standard one-way invite. No special UI.

elif nat_type == CONE:
    → Two-way invite required.
    → UI: "Your peer will need to send you a response link back.
           Share your invite, then paste their response here."

elif nat_type == SYMMETRIC:
    → Check relay_candidates (connected peers with allow_relay).
    → If candidates exist:
        → Include them in invite.
        → UI: "Direct connection isn't possible from your network.
          If you share a mutual peer, they can help negotiate the
          connection."
    → If no candidates:
        → UI: "Your network type prevents direct connections and you
          have no connected peers to help. Enable IPv6 or connect
          from a different network."

elif nat_type == UNKNOWN (no detection run):
    → Allow invite creation (assume cone, attempt Tier 2).
    → UI: "Tip: Run network detection in Settings for better
          connection reliability."
```

### Invite Redemption (Joiner Side)

```
1. Parse invite token.
2. Try IPv6 candidates first (if any). Direct connect → done.
3. Try IPv4 endpoint directly. If reachable → done.
4. If invite includes nat_type, enter Tier 2:
   → Generate accept token.
   → UI: "This peer is behind NAT. Send this response link back
          to them: howm://accept/..."
   → Begin WG handshake attempts on send.
5. If Tier 2 times out and relay_candidates present:
   → Check for mutual peer. Relay signaling exchange. → done.
6. If all fail → UNREACHABLE with explanation.
```

---

## 7. Endpoint Rotation (The "Punch" Logic)

This is the core of Tier 2. No custom protocol — just WG endpoint updates.

### Algorithm

```rust
fn attempt_punch(peer: &PeerConfig, my_nat: &NatProfile, their_nat: &NatProfile) {
    let base_port = their_nat.external_port; // STUN-reflected
    let stride = their_nat.observed_stride;

    // Build candidate list
    let mut candidates = vec![base_port]; // Preserved port first

    if stride != 0 {
        // Stride offsets: +s, -s, +2s, -2s, ...
        for i in 1..=5 {
            candidates.push(base_port + (stride * i));
            candidates.push(base_port - (stride * i));
        }
    }

    // Sequential neighbors (covers small allocation jitter)
    for offset in 1..=10 {
        candidates.push(base_port + offset);
        candidates.push(base_port - offset);
    }

    candidates.dedup();

    // Symmetric + cone: cone peer initiates, symmetric peer waits
    let i_initiate = their_nat.nat_type == Symmetric && my_nat.nat_type == Cone;

    // Rotate through candidates
    let interval = if i_initiate { 500ms } else { 200ms };
    let timeout = 15s;

    for candidate_port in candidates.cycle() {
        wg_set_endpoint(peer.pubkey, their_nat.external_ip, candidate_port);
        // WG will attempt handshake to this endpoint

        if wg_handshake_complete(peer.pubkey) {
            return Ok(());
        }

        if elapsed > timeout {
            return Err(PunchTimeout);
        }

        sleep(interval);
    }
}
```

### WG Handshake as Probe

Why this works without a custom probe protocol:

1. WG handshake initiation is a single 148-byte UDP packet.
2. Sending it creates a NAT mapping on the sender's router.
3. The peer's response comes back through that mapping.
4. For port-preserving NATs, the external port == listen port, so the
   first attempt works.
5. For sequential NATs, we rotate through candidates until one hits.
6. WG has built-in retry logic, crypto verification, and replay
   protection. We don't need to reimplement any of that.

The only thing we do is rotate the endpoint. WG handles everything else.

---

## 8. Implementation Order

### Phase 1: IPv6 Detection + Preference (~300 lines)

Everything needed so Tier 1 works reliably for IPv6 users.

- [ ] Enumerate GUA IPv6 addresses on all interfaces
- [ ] Filter out link-local, ULA, loopback
- [ ] Include IPv6 candidates in invite tokens
- [ ] Joiner tries IPv6 candidates before IPv4
- [ ] WG port selection: try 41641, fall back to 41642-41650

### Phase 2: NAT Characterization (~400 lines)

Inform the user and gate invite behavior. No connection logic yet.

- [ ] Minimal STUN client (RFC 5389 binding request/response only)
- [ ] Two-server symmetric check (Google + Cloudflare)
- [ ] Stride measurement
- [ ] Classification: OPEN / CONE / SYMMETRIC / UNKNOWN
- [ ] Cache to `nat_profile.json`
- [ ] Fresh STUN binding at invite creation (for NAT'd nodes)
- [ ] Settings UI hook: "Detect network type"
- [ ] First-install prompt

### Phase 3: Two-Way Exchange + Hole Punch (~500 lines)

The core of Tier 2. Dramatically simpler without a custom probe protocol.

- [ ] `howm://accept/` token type: generate, parse, validate
- [ ] Accept token payload: all fields from Section 5
- [ ] Invite token: add nat_type, external_ip, external_port, stride,
      wg_port, relay_candidates
- [ ] Endpoint rotation loop (Section 7)
- [ ] Symmetric + cone logic: cone peer initiates first
- [ ] WG handshake monitoring (poll `wg show` for latest-handshake)
- [ ] Timing: Bob at 1/s, Alice at 200ms, 15s hard timeout
- [ ] Same-LAN shortcut: if both have RFC1918 in same /24, try direct
      on WG port first
- [ ] Tier 2 → Tier 3 fallback on timeout

### Phase 4: Peer Relay Signaling (~300 lines)

Wire up signaling-only relay. No traffic forwarding.

- [ ] `relay_candidates` field in invite token
- [ ] Relay opt-in config: `network.allow_relay: false` (default off)
- [ ] Signaling message types over p2pcd:
      - `relay-request`: "I want to reach peer X, here's my endpoint info"
      - `relay-offer`: "Peer X wants to reach you, here's their info"
      - `relay-exchange`: "Here's my info for peer X"
- [ ] Relay peer matches request to connected peers, forwards info
- [ ] Both peers set WG endpoints from relayed info, attempt handshake
- [ ] Fallback trigger: Tier 2 timeout or symmetric NAT detected
- [ ] Clear UX when relay config is off: "A peer asked to relay through
      you. Enable network.allow_relay in settings if you want to help."

### Phase 5: UX Polish

- [ ] Invite creation flow with tier-appropriate messaging
- [ ] Joiner flow with step-by-step guidance for two-way exchange
- [ ] UNREACHABLE error display with attempted tiers and suggestions
- [ ] NAT type display in settings / node info
- [ ] Relay opt-in explanation in settings UI

---

## 9. What We're NOT Building (and why)

| Feature | Reason to skip |
|---|---|
| Dedicated probe port (4725) | WG handshake IS the probe. No separate socket needed. |
| Custom probe packet format | WG handles crypto, replay protection, retries. Don't reimplement. |
| DERP relay servers | External infrastructure. Against core principles. |
| Traffic relay / bandwidth forwarding | Relay is signaling only. Peers connect directly or not at all. |
| Smart port selection (firewall-friendly, random) | Diminishing returns. Preserved + stride covers realistic cases. |
| Known network profiles | Premature. Single cached profile until proven otherwise. |
| Per-peer NAT cache | Optimization for later. |
| Mapping lifetime measurement | Use 25s persistent-keepalive default. |
| CHANGE-REQUEST tests (RFC 3489) | Two-server symmetric check is enough. |
| Network identity (gateway MAC) | User re-runs detection manually. |
| Self-probe for IPv6 reachability | Tier ladder handles failures. Don't build infra to avoid a 2s fallback. |
| QR code exchange for accept tokens | Nice-to-have for in-person. Chat/email covers the real use case. |
| Backward compatibility versioning | No users yet. Add version field at 1.0. |

---

## 10. Punchability Matrix

With symmetric+cone support and signaling relay:

| Peer A \ Peer B | Open | Cone | Symmetric |
|---|---|---|---|
| **Open** | Tier 1 | Tier 1 | Tier 1 |
| **Cone** | Tier 1 | Tier 2 | Tier 2 (cone initiates) |
| **Symmetric** | Tier 1 | Tier 2 (cone initiates) | Tier 3 signaling, then UNREACHABLE |

"Tier 1" = one peer is reachable, one-way invite works.
"Tier 2" = hole punch via WG handshake with endpoint rotation.
"Tier 3 signaling" = mutual peer relays endpoint info, then attempt direct.
"UNREACHABLE" = symmetric+symmetric with no helper. Both ports unpredictable.

---

## 11. Open Questions

1. **STUN from WG socket.** Kernel WireGuard owns the UDP socket once
   configured. Can we send STUN requests from it before any peers are
   added? If not, we need to send from WG port + 1 and account for the
   off-by-one in stride calculation. Needs a quick test on Linux/macOS
   to confirm kernel behavior. If the kernel takes the socket at
   interface creation time (before peer add), we use a companion socket
   on the next port.

2. **Endpoint rotation rate limiting.** Calling `wg set peer endpoint`
   at 200ms intervals means WG re-initiates handshake each time. Is
   there a rate limit in the kernel module? The WG spec has a 5-second
   retry backoff for failed handshakes. We may need to remove and re-add
   the peer config to reset the backoff, or use `wg-go` userspace
   implementation where we control retry timing directly. This is the
   biggest implementation risk for Phase 3.

3. **Relay peer discovery for strangers.** The relay signaling only
   works if Alice and Bob share a mutual peer. For the very first
   connection (bootstrapping into the mesh), there are no mutual peers.
   The first connection MUST be Tier 1 or Tier 2. Is this an acceptable
   constraint? (Likely yes — you need at least one reachable friend to
   join any mesh. That's the nature of P2P.)

4. **NAT mapping consistency across sockets.** If we STUN from socket A
   (WG port) and then WG sends from its own socket B (same port), does
   the NAT treat them as the same mapping? On Linux with `SO_REUSEPORT`,
   likely yes. On macOS, possibly not. Needs testing. If not, the
   STUN-reflected port is useless and we must find another way to
   discover our external mapping.

5. **IPv6 firewall detection without self-probe.** We decided to let the
   tier ladder handle IPv6 failures (try it, fall through if it fails).
   But the failure mode is a 15-second timeout before falling to Tier 2.
   Should we set a shorter IPv6-specific timeout (e.g., 3 seconds) since
   IPv6 direct connections either work immediately or not at all?

6. **Relay opt-in UX.** When relay is off (default) and a peer requests
   signaling relay, should we (a) silently ignore the request, (b) show
   a notification "Peer X asked you to relay, enable in settings", or
   (c) one-time prompt "Allow this relay? [Yes / Yes always / No]"?
   Leaning toward (b) — inform but don't nag.

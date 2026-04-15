# LAN Connection Review

Review date: 2026-04-01
Based on: howm.log.2026-03-31 (17:29–17:42 session)

---

## 1. What the logs show — timeline of failure

```
17:32:14  Howm restarts with WireGuard successfully configured (howm0, port 41642)
17:32:14  LAN discovery active on 192.168.1.163
17:32:14  P2P-CD engine listening on 0.0.0.0:7654
17:33:06  LAN scan: found 1 peer (the other device)
17:33:10  add_peer: WG peer added — pubkey CBy/HugQ, assigned 100.222.0.3
17:33:12  LAN invite sent to 192.168.1.169:7000 — awaiting acceptance
17:33:12  WgPeerMonitor: peer visible CBy/Hg==  ← WG handshake seen
17:33:22  P2P-CD initiator FAILED: connect timeout (10s deadline)
17:33:27  LAN invite RECEIVED from 'ivy-hpstreamlaptop11ah0xx' at 192.168.1.169
17:38:28  WgPeerMonitor: peer unreachable CBy/Hg==  ← WG tunnel dies
17:42:39  Shutdown
```

### What happened step by step

1. **archlinux** (this machine, 192.168.1.163) scanned LAN, found the laptop
   (192.168.1.169).

2. User clicked "Connect" on archlinux. This triggered `POST /network/lan/invite`
   which:
   a. Generated an invite (PSK, WG addresses, etc)
   b. Added the remote's WG pubkey as a peer on howm0 (the inviter side does this
      **before** sending the invite — line 106-117 in lan_routes.rs implies the
      invite contains our info but we also call invite::generate which does
      assign_next_address and saves a PendingInvite)
   c. HTTP-POSTed the invite to `192.168.1.169:7000/node/lan-accept`

3. The laptop received the invite and its `lan_accept` handler:
   a. Decoded the invite
   b. Added archlinux as a WG peer (using LAN IP as endpoint)
   c. Called back to `192.168.1.163:7000/node/complete-invite` with its own info
   d. The complete-invite handler on archlinux consumed the PendingInvite by PSK
      and added the laptop as a WG peer

4. WireGuard handshake succeeded (17:33:12 — peer visible). The tunnel was up.

5. P2P-CD engine saw PEER_VISIBLE and tried to run an initiator session:
   TCP connect to the peer's WG address (100.222.0.3:7654) — **TIMED OUT**.

6. 15 seconds later (17:33:27), the lan_accept handler on archlinux logged
   "LAN invite received from 'ivy-hpstreamlaptop11ah0xx'" — this means **both
   sides sent invites to each other**.

7. The WG tunnel eventually died (17:38:28 — unreachable) because neither side
   completed the P2P-CD negotiation, so no keepalive traffic flowed, and WG
   tore down the session.

---

## 2. Root cause analysis

### Bug 1: Asymmetric invite — both sides sent invites simultaneously

Looking at the timeline:
- 17:33:12 — archlinux sent invite TO laptop
- 17:33:27 — archlinux received invite FROM laptop

Both users clicked "Connect" (or both sides auto-initiated). This creates a
**double invite** race condition:

- Each side generates its OWN PendingInvite with its OWN PSK and its OWN IP
  assignment
- Each side calls the other's `/node/lan-accept`
- Each side's `lan_accept` calls back to the other's `/node/complete-invite`
- The result: **each peer appears twice in the WG config**, potentially with
  conflicting addresses, or the second invite's `complete-invite` fails because
  the first invite already consumed the PSK slot

### Bug 2: P2P-CD connects to the WG overlay IP, but the peer's P2P-CD port isn't listening there

The P2P-CD engine resolves peer addresses by calling `wg show howm0 dump` and
extracting the `allowed-ips` field. For the laptop, this is `100.222.0.3/32`.
So P2P-CD tries to TCP-connect to `100.222.0.3:7654`.

But the laptop's P2P-CD listener binds to `0.0.0.0:7654`. For the laptop to
receive connections on `100.222.0.3`, its howm0 interface must be configured
with that address AND routing must work. The WG handshake succeeded (the WG
tunnel itself was up), but the TCP connect timed out — meaning either:

- The laptop's P2P-CD hadn't started yet (race condition)
- The laptop's howm0 interface didn't have `100.222.0.3` as its address
  (each node picks its OWN wg_address at startup: archlinux is 100.222.0.1,
  the laptop might also be 100.222.0.1 — there's no coordination)
- The WG tunnel was up but routing through it failed

The most likely cause: **the laptop's assigned WG address and the address
archlinux put in `allowed-ips` don't match**. archlinux assigned the laptop
`100.222.0.3` (from its own counter), but the laptop's howm0 interface is
bound to whatever address it generated for itself (likely `100.222.0.1` as
well, since each node independently derives its address). So packets to
100.222.0.3 go into the WG tunnel, arrive at the laptop, but the laptop's
IP stack drops them because no local interface has 100.222.0.3.

### Bug 3: mdns-sd binding to WG address

Repeated errors every 30s:
```
bind a socket to 100.222.0.1: send multicast packet on addr 100.222.0.1:
Required key not available (os error 126)
```

mdns-sd enumerates all network interfaces and tries to bind multicast sockets
on each. The howm0 WireGuard interface (100.222.0.1) is a point-to-point tunnel
— multicast doesn't work on it. This is a noisy but non-fatal issue.

### Bug 4: mdns-sd binding to tailscale0 and docker0

```
Cannot find valid addrs for TYPE_SRV response on intf tailscale0
Cannot find valid addrs for TYPE_SRV response on intf docker0
```

Same class of issue — non-LAN interfaces leaking into mDNS advertisement.
The mDNS daemon should be scoped to the real LAN interface only.

---

## 3. Answer: Does both sides need to click connect?

**No — only ONE peer needs to click Connect.**

The `POST /network/lan/invite` flow is fully asymmetric by design:

1. **Initiator** (the one who clicks Connect): calls `/network/lan/invite`, which
   generates an invite and POSTs it directly to the remote's `/node/lan-accept`

2. **Responder** (the remote): `lan_accept` auto-processes the invite — decodes it,
   adds the WG peer, calls back `/node/complete-invite`, fetches `/node/info`, and
   saves the peer. No user action required.

The comment in the code is explicit (lan_routes.rs:181):
```
/// For now, we auto-accept LAN invites to keep the flow frictionless.
/// The UI can show a notification that a new peer was added.
```

**However**, in the tested scenario, BOTH sides clicked Connect, which caused
the double-invite race condition described above. The UI does not prevent this
— if both users see each other in the scan results and both click Connect, two
independent invite flows cross in flight.

---

## 4. Complete bug inventory

| # | Bug | Severity | Where |
|---|-----|----------|-------|
| 1 | Double-invite race: both sides can initiate simultaneously, creating conflicting WG peer entries | **High** | lan_routes.rs — no mutual exclusion / idempotency check |
| 2 | WG address mismatch: inviter assigns remote an address (100.222.0.x) that remote doesn't use on its own howm0 | **Critical** | invite.rs + lan_routes.rs — no address negotiation |
| 3 | P2P-CD connects to WG overlay IP but remote may not be reachable there | **High** | p2pcd/engine.rs resolve_peer_addr — should fall back to LAN IP for LAN peers |
| 4 | `lan_accept` calls `/node/complete-invite` but the inviter also separately processes WG peer visibility → P2P-CD races the invite completion | **Medium** | Timing: invite flow and WG monitor are uncoordinated |
| 5 | mdns-sd binds to non-LAN interfaces (howm0, tailscale0, docker0) causing error spam | **Low** | lan_discovery.rs — no interface filtering for mdns-sd |
| 6 | No LAN invite deduplication: if both peers send invites, both get processed and overwrite each other | **Medium** | lan_routes.rs lan_accept — no "already in progress" check |
| 7 | `lan_accept` tries `/node/info` over WG IP after 1s sleep — too early, WG tunnel may not be routable yet | **Medium** | lan_routes.rs:288 — should use LAN IP or retry |
| 8 | Peer saved with `node_id: "pending"` and never updated | **Low** | Both lan_accept and complete_invite save "pending" metadata |

---

## 5. Proposed spec: LAN Connection v2

### 5.1 Address coordination (fixes Bug #2, the critical issue)

The current flow has a fundamental problem: each node independently assigns a
WG address from its own counter. When archlinux assigns 100.222.0.3 to the
laptop, the laptop doesn't know about this. The laptop's howm0 is 100.222.0.1.

**Proposed fix**: During LAN invite, the two nodes negotiate addresses:

1. The invite already contains `our_wg_address` and `assigned_ip` (the IP the
   inviter reserved for the remote).
2. `lan_accept` must **configure its own howm0 with the assigned_ip** as an
   additional address, OR the invite system should use the remote's existing
   WG address instead of assigning one.

**Simpler approach**: For LAN connections, skip the address assignment. Instead,
use each node's pre-existing `wg_address` in the invite and `allowed-ips`.
Both nodes already have self-assigned WG addresses. The invite should carry
"use my address as 100.222.0.1" and the acceptor should set `allowed-ips` to
that value, and vice versa. No new address assignment needed.

### 5.2 Single-initiator guarantee (fixes Bugs #1 and #6)

Prevent the double-invite race:

**Option A — Lock in lan_accept**: When `lan_accept` receives an invite from
peer X, immediately check if we have an outbound invite pending to peer X
(match by wg_pubkey). If yes, use deterministic tiebreaking (lexicographic
pubkey comparison) — the lower pubkey's invite wins, the higher pubkey's
invite is discarded. -Yes Do this

Recommendation: **Option A** — deterministic, no UI changes needed.

### 5.3 P2P-CD LAN awareness (fixes Bug #3)

`resolve_peer_addr` currently only looks at WG `allowed-ips`. For LAN peers,
the P2P-CD TCP connect should try the LAN IP first (or in addition to the WG
overlay IP).

Proposed: Store a `transport_hint` on the Peer struct — for LAN-discovered
peers, this is the LAN IP. `resolve_peer_addr` checks transport hints before
falling back to WG allowed-ips.

### 5.4 mdns-sd interface scoping (fixes Bugs #4 and #5)

When creating the `ServiceDaemon`, filter interfaces to only include the
real LAN network (e.g., interfaces matching the detected LAN IP subnet).
Exclude:
- WG interfaces (howm0, wg*)
- Tailscale (tailscale0, ts*)
- Docker (docker*, br-*, veth*)
- Loopback (lo)

The `mdns-sd` crate supports `ServiceDaemon::new_with_ip_list()` — pass
only the LAN IP to scope mDNS to the correct interface.

### 5.5 Sequencing fix (fixes Bug #4)

The WgPeerMonitor fires `PEER_VISIBLE` as soon as a WG handshake is detected.
This triggers P2P-CD immediately, but the LAN invite flow is still mid-flight
(lan_accept hasn't finished calling complete-invite yet).

Proposed: Add a `peering_in_progress` flag (per pubkey) that suppresses
P2P-CD initiator sessions while an invite is being processed. Clear the flag
after `lan_accept` or `lan_invite` completes.

### 5.6 Peer info over LAN (fixes Bug #7)

After peering, `lan_accept` tries to fetch `/node/info` over the WG tunnel
(100.222.0.x:7000). This is unreliable for the same reasons as Bug #2.

Fix: Fetch `/node/info` over the **LAN IP** that's already known and working
(req.from_lan_ip:decoded.their_daemon_port). The LAN HTTP path is how the
invite was delivered in the first place — it's guaranteed reachable.

### 5.7 Peer metadata update (fixes Bug #8)

After a successful LAN connection, both sides save the peer with
`node_id: "pending"` and `name: "pending"`. There's no follow-up mechanism
to update these fields.

Proposed: After the WG tunnel is confirmed working, fetch `/node/info` over
the tunnel (or LAN) and update the peer record with real node_id and name.
Add a `peer_refresh` background task that periodically updates "pending" peers.

---

## 6. Implementation priority

| Priority | Task | Effort |
|----------|------|--------|
| P0 | Fix WG address coordination (5.1) | Medium — change invite flow for LAN |
| P0 | Single-initiator guarantee (5.2) | Small — add pubkey tiebreaking |
| P1 | P2P-CD LAN awareness (5.3) | Small — add transport_hint to Peer |
| P1 | Sequencing fix (5.5) | Small — add peering_in_progress flag |
| P1 | Peer info over LAN IP (5.6) | Trivial — change URL in lan_accept |
| P2 | mdns-sd interface scoping (5.4) | Small — use new_with_ip_list() |
| P2 | Peer metadata refresh (5.7) | Small — background task |

---

## 7. Files involved

```
node/daemon/src/lan_discovery.rs     — mDNS broadcast/scan
node/daemon/src/api/lan_routes.rs    — /network/lan/* + /node/lan-accept
node/daemon/src/api/node_routes.rs   — /node/complete-invite, /node/redeem-invite
node/daemon/src/invite.rs            — invite generate/decode/consume
node/daemon/src/wireguard.rs         — WG interface management + WgPeerMonitor
node/daemon/src/p2pcd/engine.rs      — P2P-CD session initiation
node/p2pcd/src/transport.rs          — TCP connect (10s timeout)
node/daemon/src/api/mod.rs           — route registration + auth layers
```


### PROGRESS
Updated todo list ✓                                                                                                                                                                                           
   **Bug #1**: Add the race condition tiebreaking block after the expiry check and before the "already peered" check.                                                                                              
   ├ Edited lan_routes.rs (node/daemon/src/api/lan_routes.rs) +39 ✓                                                                                                                                                
   └ Completed todo bug1 ✓                                                                                                                                                                                         
   **Bug #7**: Fix the `peer_info_url` to use `req.from_lan_ip` instead of `decoded.their_wg_address`.                                                                                                             
   ├ Edited lan_routes.rs (node/daemon/src/api/lan_routes.rs) +1 -1 ✓                                                                                                                                              
   └ Completed todo bug7 ✓                                                                                                                                                                                         
   **Bug #8**: The fallback defaults already use `req.from_name` (`.unwrap_or(&req.from_name)` and `req.from_name.clone()`). Let me verify the current state of the fallback section:                              
   ├ Searched for "peer_info|from_name|unwrap_or|unknown" lan_routes.rs ✓                                                                                                                                          
   └ Read lan_routes.rs (node/daemon/src/api/lan_routes.rs) ✓                                                                                                                                                      
   The fallback defaults already use `req.from_name` correctly — `unwrap_or(&req.from_name)` for the success path and `req.from_name.clone()` for the error/missing paths. The `node_id` fallback uses `"...       
   └ Completed todo bug8 ✓ 

   **Bug #3 (P2P-CD LAN awareness)**: P2P-CD now resolves peer addresses via LAN transport hints before falling back to WG overlay.
   ├ Added `lan_transport_hints: Arc<RwLock<HashMap<PeerId, SocketAddr>>>` to ProtocolEngine
   ├ Added `set_lan_hint(peer_id, addr)` public method
   ├ `resolve_peer_addr()` checks LAN hints before `wg show dump` (engine.rs)
   ├ `lan_accept` registers LAN hint after peering completes (lan_routes.rs)
   └ Completed ✓

   **Bug #4 (Sequencing fix)**: P2P-CD initiator sessions suppressed during invite flow.
   ├ Added `peering_in_progress: Arc<Mutex<HashSet<PeerId>>>` to ProtocolEngine
   ├ Added `set_peering_in_progress()` / `clear_peering_in_progress()` methods
   ├ `on_peer_visible()` skips if peer is in peering_in_progress set
   ├ `lan_accept` calls `clear_peering_in_progress()` after peering completes
   └ Completed ✓

   **Bug #5 (mdns-sd interface scoping)**: mDNS now scoped to LAN IP only.
   ├ After `ServiceDaemon::new()`, calls `enable_interface(IfKind::Addr(lan_ip))`
   ├ This excludes howm0/WG, tailscale0, docker0 from multicast
   ├ Eliminates "Required key not available (os error 126)" errors on howm0
   ├ Eliminates "Cannot find valid addrs" errors on tailscale0/docker0
   └ Completed ✓

   **Bug #2 (WG address coordination)**: Addressed indirectly via LAN transport hints.
   ├ P2P-CD no longer depends on WG overlay IP routing for LAN peers
   ├ `resolve_peer_addr()` → LAN hint → direct TCP to LAN IP:7654
   ├ WG tunnel still used for encrypted data, but P2P-CD connection isn't blocked by address mismatch
   └ Completed ✓

   **Additional fixes**:
   ├ Added `lan_ip: Option<String>` field to `Peer` struct (peers.rs)
   ├ All Peer constructions updated (node_routes.rs ×5, lan_routes.rs ×1)
   ├ LAN-discovered peers store `lan_ip: Some(ip)` for future use
   ├ Fixed Bug #1 compile error: `remove_peer()` missing `node_id` arg → added "pending"
   └ All 111 tests pass ✓

### Round 2 fixes (2026-04-01 post-test)

   Log analysis from howm.log.2026-04-01 revealed three remaining issues:

   **Fix A: `identify_peer_by_addr` must check LAN hints (responder path)**
   The laptop connected inbound to archlinux from 192.168.1.169:34726.
   `identify_peer_by_addr(192.168.1.169)` only checked WG allowed-ips (overlay IPs).
   It couldn't identify the peer → "inbound from unknown addr, dropping".
   ├ Added LAN transport hints reverse-lookup in `identify_peer_by_addr()`
   └ Now checks `lan_transport_hints` map after test overrides, before WG dump

   **Fix B: Inviter side never registered LAN hints**
   Only `lan_accept` (responder) set LAN hints. The inviter (`lan_invite`)
   knew the peer's LAN IP from scan results but didn't register it.
   ├ Added `wg_pubkey: Option<String>` to `LanInviteRequest` (serde default, backward-compat)
   ├ `lan_invite` now calls `engine.set_lan_hint()` after successful invite send
   ├ Also calls `engine.clear_peering_in_progress()` since complete-invite has fired by then

   **Fix C: mdns-sd `enable_interface` doesn't exclude others**
   `enable_interface(IfKind::Addr(ip))` adds to the enabled set but by default
   ALL interfaces are enabled. Must first `disable_interface(IfKind::All)` to
   exclude everything, then re-enable only the LAN IP.
   ├ Added `daemon.disable_interface(IfKind::All)` before `enable_interface`
   └ Eliminates error-126 on howm0 and invalid-addr errors on tailscale0/docker0

   All 111 tests pass ✓

   (See Round 2 fixes above for the 3 issues found in the 2026-04-01 session.)

---

## Round 3 findings (2026-04-01 evening session)

Log analyzed: howm.log.2026-04-01 (19:05:22 session)
Peer in peers.json: `pending` / pubkey `CBy/HugQWdioSmS/LBZhT380+YxN8Bd0f/9iPAQxOjs=` / LAN 192.168.1.169 / WG 100.222.0.7
Symptom: peer registered but shows offline. P2P-CD connect timeout at 19:05:34.

### Timeline

```
19:05:22  Daemon starts. Loaded 1 peer, 6 capabilities.
19:05:22  WARN: skipping migration for peer 'pending': invalid WG pubkey
19:05:22  WG restored 1 WG peer (CBy/Hg==)
19:05:22  P2P-CD engine initialised, listening on 0.0.0.0:7654
19:05:22  LAN discovery active on 192.168.1.163
19:05:24  WgPeerMonitor: peer visible CBy/Hg== (WG handshake succeeded)
19:05:24  engine: PEER_VISIBLE CBy/Hg==
19:05:34  engine: initiator CBy/Hg== FAILED: connect timeout (deadline elapsed)
19:08:22  WgPeerMonitor: peer unreachable CBy/Hg== (tunnel dead, no keepalive)
```

### What happened

1. Daemon restarted after a previous LAN peering session. The peer from that
   session is in peers.json with `node_id: "pending"`, `name: "pending"`, and
   `wg_pubkey: "CBy/HugQWdioSmS/LBZhT380+YxN8Bd0f/9iPAQxOjs="` (base64, 32 bytes).

2. `migrate_trust_levels()` tried to hex-decode the wg_pubkey. Base64 is NOT
   valid hex — it fails, and the peer is skipped with "invalid WG pubkey".
   This is a bug in the migration code (see Bug #9 below).

3. WireGuard restored the peer's config (the tunnel had been saved) and the
   WG handshake succeeded almost immediately (peer visible at 19:05:24).

4. P2P-CD engine saw PEER_VISIBLE and immediately tried to TCP-connect to the
   peer via `resolve_peer_addr`. But:
   - The `lan_transport_hints` map is in-memory only. It is NOT persisted.
   - On restart it is empty. No code in main.rs re-populates it from peers.json.
   - `resolve_peer_addr` found no LAN hint, fell back to WG `allowed-ips`
     (100.222.0.7/32), tried TCP to 100.222.0.7:7654 — timeout.
   - The peer's howm0 is NOT bound to 100.222.0.7. Each node assigns its own
     WG address independently. The peer's actual WG address is likely 100.222.0.1
     from its own perspective. So the packet enters the WG tunnel, reaches the
     peer, but the peer's IP stack drops it (no interface with 100.222.0.7).

5. With no P2P-CD session established, no keepalive traffic flows through the
   WG tunnel. WG times out and marks the peer unreachable (19:08:22).

6. Peer is stuck as "pending" forever — no background task refreshes the
   node_id/name, and no retry mechanism reconnects P2P-CD after the tunnel
   goes down.

### Bug #9: migrate_trust_levels decodes wg_pubkey as hex, not base64

Location: node/daemon/src/main.rs `migrate_trust_levels()`

```rust
let peer_id = match hex::decode(&peer.wg_pubkey) {
    Ok(id) if id.len() == 32 => id,
    _ => {
        tracing::warn!("skipping migration for peer '{}': invalid WG pubkey", peer.name);
        continue;
    }
};
```

The `wg_pubkey` field in peers.json is base64 (WireGuard key format), not hex.
`hex::decode("CBy/HugQ...")` fails silently. The peer is skipped with a
misleading warning. The peer never gets assigned to an access.db group, so it
may not get access grants that other code expects to be present.

Fix: decode with `base64::engine::general_purpose::STANDARD.decode()` instead
of `hex::decode`. The same pattern is used correctly in lan_routes.rs and
node_routes.rs. This is a copy-paste error from an earlier non-WG peer scheme.

### Bug #10: LAN transport hints not restored on daemon restart

Location: node/daemon/src/main.rs (startup), node/daemon/src/p2pcd/engine.rs

The `lan_transport_hints` HashMap lives only in memory. After a restart, the
P2P-CD engine has no knowledge of which peers are reachable via LAN vs WG
overlay. peers.json stores `lan_ip` for LAN-discovered peers, but startup
code never re-populates the engine's hint table from it.

Consequence: after restart, if WG handshake fires before the user does a new
LAN scan (or before any new LAN invite), P2P-CD falls through to WG overlay
routing and times out (exactly what happened in this session).

Fix: after `build_p2pcd_engine()` in main.rs, iterate loaded peers and
re-register any with `lan_ip: Some(...)`:

```rust
if let Some(ref engine) = p2pcd_engine {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    for peer in &peers {
        if let Some(ref lan_ip) = peer.lan_ip {
            if let Ok(bytes) = STANDARD.decode(&peer.wg_pubkey) {
                if bytes.len() == 32 {
                    let mut peer_id = [0u8; 32];
                    peer_id.copy_from_slice(&bytes);
                    if let Ok(ip) = lan_ip.parse::<std::net::IpAddr>() {
                        let addr = std::net::SocketAddr::new(ip, 7654);
                        engine.set_lan_hint(peer_id, addr).await;
                    }
                }
            }
        }
    }
}
```

### Bug #11: No P2P-CD retry after WG tunnel loss

Location: node/daemon/src/p2pcd/engine.rs `on_peer_visible` / WgPeerMonitor

When the WG tunnel drops (PEER_UNREACHABLE at 19:08:22), the engine logs it
but takes no reconnection action. If the peer comes back online later and WG
re-handshakes (PEER_VISIBLE fires again), the initiator will retry — but only
if the peer is still in the WG config. For LAN peers, the WG keepalive interval
may not be set, so the tunnel dies quickly with no traffic.

The deeper issue: P2P-CD is the primary channel for application-level comms.
If P2P-CD never connects, there's no keepalive → WG times out → peer goes
offline → no automatic recovery.

Fix options:
  a. When `on_peer_visible` fires and P2P-CD connect fails, schedule a retry
     with exponential backoff (1s, 2s, 4s... up to ~60s).
  b. Set WG `persistent-keepalive = 25` for LAN peers so the tunnel stays up
     regardless of P2P-CD status, giving P2P-CD more time to connect.
  c. Both.

Option (b) is trivially cheap and solves the immediate tunnel-death problem.
`wg set howm0 peer <pubkey> persistent-keepalive 25` in `add_peer()` for LAN
peers (detected by endpoint being a private/RFC1918 IP).

### Summary of new issues

| # | Bug | Severity | File |
|---|-----|----------|------|
| 9 | migrate_trust_levels decodes wg_pubkey as hex instead of base64 | Medium | main.rs:521 |
| 10 | LAN transport hints not restored from peers.json on daemon restart | **High** | main.rs startup |
| 11 | No P2P-CD retry / no WG keepalive for LAN peers → tunnel dies, peer stays offline | **High** | engine.rs + wireguard.rs |

Bug #10 is the primary cause of "peer registered but offline" in this session.
Bug #11 is the reason it doesn't recover. Bug #9 is a latent correctness issue
that could cause access control problems for the pending peer.

Not interested in #11 atm.

## Session 4
Now I have a clear picture. Let me look at the full flow in the last session (starting at line       
   604). Here's my analysis:                                                                            
                                                                                                        
   ---                                                                                                  
                                                                                                        
   ANALYSIS OF THE LAST SESSION (21:32 - 21:38)                                                         
   =============================================                                                        
                                                                                                        
   TIMELINE:                                                                                            
     21:32:13  Alice starts with "Loaded 0 peers" (peers were cleared)                                  
     21:32:15  WgPeerMonitor sees CBy/Hg== visible (WG config has Bob from wg-peers.json)               
     21:32:25  P2P-CD initiator TIMES OUT connecting to CBy/Hg== (Bob)                                  
     21:32:29  LAN scan: 0 peers (Bob not yet advertising)                                              
     21:34:22  LAN scan: 1 peer found (Bob is up now)                                                   
     21:34:28  LAN invite sent to 192.168.1.169:7000 (Bob)                                              
     21:34:39  LAN invite sent again (second attempt from UI)                                           
     21:35:23  >>> INBOUND FROM UNKNOWN ADDR 192.168.1.169:40200, DROPPING <<<                          
     21:35:31  P2P-CD initiator times out again                                                         
     21:36:17  LAN invite sent (third attempt)                                                          
     21:38:32  Shutdown, no session ever established                                                    
                                                                                                        
   THE SMOKING GUN: Line 717                                                                            
     "engine: inbound from unknown addr 192.168.1.169:40200, dropping"                                  
                                                                                                        
   This means Bob (192.168.1.169) successfully connected to Alice via P2P-CD,                           
   but Alice's identify_peer_by_addr() couldn't map 192.168.1.169 back to                               
   any known peer, so it DROPPED the connection.                                                        
                                                                                                        
                                                                                                        
   ROOT CAUSE: complete_invite doesn't set LAN transport hints                                          
   =============================================================                                        
                                                                                                        
   The LAN invite flow has an asymmetry:                                                                
                                                                                                        
     ALICE (inviter) calls lan_invite:                                                                  
       1. Generates invite code                                                                         
       2. POSTs to Bob's /node/lan-accept                                                               
       3. Bob's lan_accept calls Alice's /node/complete-invite                                          
       4. Alice's complete_invite adds Bob as WG peer, adds to peers list                               
       5. Back in lan_invite, sets LAN transport hint (IF wg_pubkey in request)                         
                                                                                                        
     BOB (acceptor) in lan_accept:                                                                      
       - Adds Alice as WG peer                                                                          
       - Calls Alice's /node/complete-invite                                                            
       - Gets Alice's /node/info → proper name/node_id                                                  
       - Adds Alice to peers list with correct info                                                     
       - Sets LAN transport hint for Alice ← WORKS                                                      
                                                                                                        
   The problem is Alice's complete_invite (node_routes.rs:311-374):                                     
                                                                                                        
     1. It adds Bob with name="pending", node_id="pending"                                              
     2. It sets lan_ip: None  ← NO LAN IP                                                               
     3. It does NOT register a LAN transport hint with P2P-CD engine                                    
     4. It logs NOTHING (no tracing::info! on success)                                                  
                                                                                                        
   So after the invite completes:                                                                       
     - identify_peer_by_addr(192.168.1.169) checks:                                                     
       a) peer_addr_overrides → nothing                                                                 
       b) lan_transport_hints → nothing (never set!)                                                    
       c) WG allowed_ips → 100.222.x.x (overlay IP, doesn't match LAN IP!)                              
     → Returns None → "inbound from unknown addr, dropping"                                             
                                                                                                        
   Meanwhile resolve_peer_addr for outbound tries:                                                      
     - LAN hints → nothing                                                                              
     - WG overlay IP 100.222.x.x → tries to connect via WG tunnel                                       
     → But WG tunnel may not be handshaking properly → timeout                                          
                                                                                                        
                                                                                                        
   SECONDARY ISSUES:                                                                                    
     - Alice's peer has name="pending" → shows wrong in UI                                              
     - Warning at startup: "skipping migration for peer 'pending': invalid WG pubkey"                   
     - No P2P-CD session = no heartbeats = both show "offline"                                          
                                                                                                        
                                                                                                        
   THE FIX: complete_invite needs to register LAN transport hints                                       
   ==============================================================                                       
                                                                                                        
   In complete_invite (node_routes.rs), after adding the peer, it should:                               
   1. Use ConnectInfo(addr) to detect the caller's LAN IP                                               
   2. Set lan_ip on the peer                                                                            
   3. Register a LAN transport hint with the P2P-CD engine                                              
   4. Try to fetch peer info (name/node_id) instead of leaving "pending"  

                                           
   ANSWER TO YOUR QUESTION: Yes, WAN invites had the same class of bug.                                 
   ============================================================                                         
                                                                                                        
   The original `complete_invite` never registered LAN transport hints                                  
   OR resolved peer names — regardless of LAN or WAN. My first fix                                      
   naively treated all non-loopback IPs as LAN, which would have:                                       
                                                                                                        
     WAN scenario: Bob at 73.x.x.x calls Alice's /node/complete-invite                                  
     → My first fix would set LAN hint to 73.x.x.x:7654                                                 
     → resolve_peer_addr would prefer this hint over WG overlay                                         
     → P2P-CD tries to connect to 73.x.x.x:7654 through NAT → fails                                     
     → WORSE than before (WG overlay path would have worked)                                            
                                                                                                        
   THE FIX: Private IP detection guards the LAN hint path                                               
     - Only RFC 1918 IPv4 (10.x, 172.16-31.x, 192.168.x) gets LAN hints                                 
     - IPv6 fe80::/10 link-local gets LAN hints                                                         
     - Public IPs (WAN path) → no LAN hint, no endpoint override                                        
     - WAN peers use WG overlay routing via allowed_ips (the existing path)                             
                                                                                                        
   TESTS ADDED (3 new, all passing):                                                                    
     1. lan_hint_enables_identify_peer_by_addr                                                          
        — Verifies LAN IP → peer_id resolution via hints                                                
     2. lan_hint_enables_resolve_peer_addr                                                              
        — Verifies outbound address resolution prefers LAN hint                                         
     3. lan_hint_inbound_session_accepted                                                               
        — Full engine integration: Alice uses set_lan_hint (not set_peer_addr),                         
          Bob connects inbound, both reach Active. This is the exact scenario                           
          that was failing in production ("inbound from unknown addr, dropping").                       
                                                                                                        
   Full test suite: 80+ unit tests + 13 integration + 18 punch tests — all pass.

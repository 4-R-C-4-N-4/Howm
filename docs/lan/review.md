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

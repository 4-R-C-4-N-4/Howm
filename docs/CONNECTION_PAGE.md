# Connection Page — Dedicated Networking UI

## Summary

Break the networking/invite/connectivity concerns out of the Dashboard into a
dedicated **Connection** page at `/connection`. The Dashboard is getting heavy —
it currently holds API token, node info, WireGuard status, open invites, peer
list, and capabilities all in one scroll. The connection stuff alone (WireGuard,
NAT detection, invite generation, invite redemption, open invites, accept tokens,
relay config) is complex enough to warrant its own page with contextual help that
explains what's happening in plain language.

This proposal covers the new Connection page design and lays the groundwork for
the NETWORKING_FINAL phases (NAT detection, two-way exchange, relay signaling)
to land cleanly in the UI.

---

## What Moves Out of Dashboard

| Component | Currently | Moves to |
|---|---|---|
| WireGuard status card | Dashboard | Connection page — Network Status section |
| Open Invite section | Dashboard (OpenInviteSection) | Connection page — Invites tab |
| Generate/Redeem Invite | PeerList header buttons | Connection page — Invites tab |
| Peer list | Dashboard (PeerList) | **Stays on Dashboard** (it's node management, not connection) |
| Endpoint missing warning | Dashboard WG card | Connection page — Network Status (with richer explanation) |

The Dashboard keeps: API token, node info, peer list, capabilities. It becomes
a clean operational overview. The Connection page owns everything about "how do
I connect to people and what's my network situation."

---

## Page Layout

```
┌─────────────────────────────────────────────────────────────────┐
│ howm    Dashboard    Connection    Feed    ···         Settings  │
│─────────────────────────────────────────────────────────────────│
│                                                                 │
│  Connection                                          [ⓘ Info]   │
│                                                                 │
│  ┌─── Your Network ────────────────────────────────────────┐    │
│  │                                                         │    │
│  │  Status      ● Connected                                │    │
│  │  Public IP   203.0.113.5 (IPv4)                         │    │
│  │              2001:db8::1 (IPv6)  ✓ Global Unicast       │    │
│  │  NAT Type    Cone (port-preserving)   [Re-detect]       │    │
│  │  WG Port     41641                                      │    │
│  │  Endpoint    203.0.113.5:41641                          │    │
│  │  Public Key  Abc123...xyz=                              │    │
│  │  Tunnels     3 active                                   │    │
│  │                                                         │    │
│  │  ┌──────────────────────────────────────────────────┐   │    │
│  │  │ ✓ Your node is directly reachable. One-way       │   │    │
│  │  │   invites will work for anyone.                  │   │    │
│  │  └──────────────────────────────────────────────────┘   │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                 │
│  ┌─── Invites ─────────────────────────────────────────────┐    │
│  │                                                         │    │
│  │  [Create Invite]  [Create Open Invite]  [Redeem]        │    │
│  │                                                         │    │
│  │  ┌ Active Open Invite ─────────────────────────────┐    │    │
│  │  │  ● Active    2/10 peers                         │    │    │
│  │  │  howm://open/abc123...                          │    │    │
│  │  │  [Copy Link]  [Revoke]                          │    │    │
│  │  └─────────────────────────────────────────────────┘    │    │
│  │                                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                 │
│  ┌─── Relay ───────────────────────────────────────────────┐    │
│  │                                                         │    │
│  │  Allow Relay Signaling    [  OFF  ]                     │    │
│  │                                                         │    │
│  │  When enabled, your node can help two of your peers     │    │
│  │  who can't reach each other directly exchange           │    │
│  │  connection info. No traffic is forwarded — just a      │    │
│  │  few small messages to help them find each other.       │    │
│  │                                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## The Info Panel

Clicking the **[ⓘ Info]** button in the page header opens a slide-out panel (or
expands an inline section) that explains the user's specific situation in plain
language. This is NOT a generic help page — it reads the actual network state and
composes a personalized explanation.

### How It Works

The info panel reads three pieces of state:
1. **NAT detection results** (nat_profile.json — type, external IP, stride)
2. **WireGuard status** (endpoint set or not, IPv6 available or not)
3. **Peer count** (are they isolated or already connected)

From these, it composes 3 sections:

#### Section 1: "Your Setup"

Plain English description of what the node detected about its network.

Example outputs depending on state:

**Best case (IPv6 + public):**
> Your node has a public IPv6 address (2001:db8::1) and is directly reachable
> from the internet. This is the ideal setup. Anyone can connect to you with a
> simple one-way invite — you generate a link, send it to them, they click it,
> done. No extra steps needed.

**Good case (IPv4, cone NAT):**
> Your node is behind a NAT router, but it's a "cone" type NAT which means
> connections can be punched through it. If you're inviting someone who is also
> behind NAT, you'll need to do a two-way exchange — you send them an invite,
> they send you an accept link back. If either of you has a public IP or IPv6,
> the simple one-way invite works fine.

**Tricky case (symmetric NAT):**
> Your node is behind a symmetric NAT, which is the hardest type to connect
> through. Direct connections only work if the other person has a public IP or
> IPv6. To connect with someone who is also behind NAT, you'll need a mutual
> friend already on the mesh who can relay the connection setup (they won't see
> your traffic — just help you find each other). If you have no mutual friends
> yet, your first connection needs to be with someone who has a public IP.

**No detection run:**
> Your network type hasn't been detected yet. Howm can figure out the best
> connection strategy for your setup if you run a quick detection. It sends two
> small UDP packets to public STUN servers — nothing is installed or changed.
>
> [Detect My Network]

**No endpoint set:**
> ⚠ Your WireGuard endpoint isn't configured. This means other nodes don't know
> how to reach you. You need to either:
> - Set your public IP: restart with `--wg-endpoint <your-ip>:41641`
> - Or let Howm auto-detect it by running network detection below.

#### Section 2: "How Connections Work For You"

A simplified flowchart specific to their NAT type:

**If OPEN or has IPv6:**
> **When you invite someone:**
> 1. Click "Create Invite" → get a link
> 2. Send the link to your friend (text, email, whatever)
> 3. They paste it into their howm → connected!
>
> **When someone invites you:**
> 1. They send you a howm://invite/... link
> 2. Paste it in "Redeem" → connected!

**If CONE (needs two-way for NAT peers):**
> **When you invite someone who has a public IP:**
> Same as above — one-way invite, done.
>
> **When you invite someone also behind NAT:**
> 1. Click "Create Invite" → get a link
> 2. Send it to them
> 3. They'll get a response link — they send that back to you
> 4. Paste their response in "Redeem Accept" → both sides punch through → connected!
>
> The two-way exchange is needed so both sides know where to aim the connection.

**If SYMMETRIC:**
> **You can connect directly to anyone with a public IP or IPv6.**
> Use one-way invites normally.
>
> **To connect with someone also behind NAT:**
> You need a mutual friend already on the mesh. That friend's node relays the
> connection setup (not traffic — just "hey, here's where to find each other").
> If you don't have mutual friends yet, connect to someone with a public IP first.

#### Section 3: "Connectivity Quick Reference"

Always shown. A simple table:

> | Them → | Public/IPv6 | Cone NAT | Symmetric NAT |
> |---|---|---|---|
> | **You** | ✓ one-way invite | ✓ one-way invite | ✓ one-way invite |
>
> (row changes based on user's NAT type to show their specific situation)

For cone NAT users:
> | Them → | Public/IPv6 | Cone NAT | Symmetric NAT |
> |---|---|---|---|
> | **You (cone)** | ✓ one-way | ↔ two-way exchange | ↔ two-way exchange |

For symmetric:
> | Them → | Public/IPv6 | Cone NAT | Symmetric NAT |
> |---|---|---|---|
> | **You (symmetric)** | ✓ one-way | ↔ they initiate | ⚠ needs relay friend |

---

## New API Endpoints

The Connection page needs data the Dashboard didn't:

| Method | Path | Description |
|---|---|---|
| GET | `/network/status` | Combined network status: NAT profile, public IPs (v4+v6), reachability tier, WG status, relay config |
| POST | `/network/detect` | Trigger NAT detection (STUN test battery). Returns results when done (~2-3s) |
| GET | `/network/nat-profile` | Cached NAT detection results (or null if never run) |
| PUT | `/network/relay` | Update relay config (`{ allow_relay: bool }`) |
| POST | `/node/accept` | Redeem an accept token (Tier 2 two-way exchange) |

The `/network/status` endpoint is a convenience aggregator that returns
everything the Connection page needs in one call:

```json
{
  "wireguard": {
    "status": "connected",
    "public_key": "abc...",
    "address": "100.222.0.1",
    "endpoint": "203.0.113.5:41641",
    "listen_port": 41641,
    "active_tunnels": 3
  },
  "nat": {
    "detected": true,
    "nat_type": "cone",
    "external_ipv4": "203.0.113.5",
    "external_port": 41641,
    "observed_stride": 0,
    "detected_at": 1742320000
  },
  "ipv6": {
    "available": true,
    "global_addresses": ["2001:db8::1"],
    "preferred": true
  },
  "reachability": "direct",
  "relay": {
    "allow_relay": false,
    "relay_capable_peers": 2
  },
  "peer_count": 5
}
```

The `reachability` field is computed server-side:
- `"direct"` — has public IPv4/IPv6 or open NAT
- `"punchable"` — cone NAT, needs two-way for NAT peers
- `"relay-only"` — symmetric NAT, needs relay for NAT peers
- `"unknown"` — no detection run yet

---

## New Components

### File: `ui/web/src/pages/Connection.tsx`

The main page component. Fetches `/network/status` and renders the three
sections (Network Status, Invites, Relay). Also owns the info panel toggle.

### File: `ui/web/src/components/NetworkStatus.tsx`

The "Your Network" card. Shows WG status, public IPs, NAT type, and the
reachability summary badge. Includes the "Re-detect" button that calls
`POST /network/detect`.

Reachability badge rendering:
```
direct      → ✓ green  "Directly reachable"
punchable   → ● yellow "NAT — two-way exchange for NAT peers"
relay-only  → ⚠ orange "Symmetric NAT — relay needed for NAT peers"
unknown     → ? grey   "Network not yet detected"
```

### File: `ui/web/src/components/ConnectionInfo.tsx`

The info panel. Receives network status as props, composes the three
explanation sections based on actual state. Pure presentational — all logic is
just conditional text rendering based on `reachability`, `nat.nat_type`,
`ipv6.available`, `peer_count`, and `wireguard.endpoint`.

Rendered as a slide-out panel from the right (like a help drawer), not a modal.
Can stay open while interacting with the page. Dismiss with X or clicking
outside.

### File: `ui/web/src/components/InviteManager.tsx`

Consolidates all invite functionality currently split across PeerList and
OpenInviteSection:

- **Create Invite** — generates a one-way `howm://invite/...` link
- **Create Open Invite** — creates a reusable open invite (existing flow)
- **Redeem Invite** — paste any `howm://invite/...` or `howm://open/...` link
- **Redeem Accept** — paste a `howm://accept/...` link (Tier 2, new)
- **Active open invite** display with copy/revoke

The invite creation flow is NAT-aware. When the user clicks Create Invite:

```
if reachability == "direct":
    → Generate invite, show link, done.
    → "Send this to your friend. They paste it, you're connected."

if reachability == "punchable":
    → Generate invite (includes NAT info), show link
    → "Send this to your friend. If they're also behind NAT,
       they'll send you a response link — paste it in 'Redeem Accept'."

if reachability == "relay-only":
    → Generate invite (includes relay candidates), show link
    → "Your network makes direct connections tricky. If your friend
       has a public IP, this will work normally. Otherwise, you'll
       need a mutual friend on the mesh to help with the connection."

if reachability == "unknown":
    → Show suggestion: "Run network detection first for the best
       connection experience." with a [Detect Now] button
    → Still allow invite creation (assume punchable)
```

### File: `ui/web/src/components/RelayConfig.tsx`

Simple toggle card for the relay opt-in setting. Shows:
- Current state (on/off)
- Count of relay-capable peers (peers who also have relay on)
- Plain language explanation of what relay does
- Toggle button that calls `PUT /network/relay`

### File: `ui/web/src/api/network.ts`

API client for all the new network endpoints:

```typescript
export interface NetworkStatus {
  wireguard: WgStatus;
  nat: NatProfile | null;
  ipv6: IPv6Status;
  reachability: 'direct' | 'punchable' | 'relay-only' | 'unknown';
  relay: RelayConfig;
  peer_count: number;
}

export interface NatProfile {
  detected: boolean;
  nat_type: 'open' | 'cone' | 'symmetric' | 'unknown';
  external_ipv4: string | null;
  external_port: number | null;
  observed_stride: number;
  detected_at: number;
}

export interface IPv6Status {
  available: boolean;
  global_addresses: string[];
  preferred: boolean;
}

export interface RelayConfig {
  allow_relay: boolean;
  relay_capable_peers: number;
}

export const getNetworkStatus = () =>
  api.get<NetworkStatus>('/network/status').then(r => r.data);

export const detectNetwork = () =>
  api.post<NatProfile>('/network/detect').then(r => r.data);

export const updateRelayConfig = (allow_relay: boolean) =>
  api.put<RelayConfig>('/network/relay', { allow_relay }).then(r => r.data);

export const redeemAccept = (accept_token: string) =>
  api.post('/node/accept', { accept_token }).then(r => r.data);
```

---

## App.tsx Changes

Add the Connection route and nav link:

```tsx
// New import
import { Connection } from './pages/Connection';

// In NavBar, after Dashboard:
<NavLink to="/connection" style={linkStyle}>Connection</NavLink>

// In Routes:
<Route path="/connection" element={<Connection />} />
```

---

## Dashboard Cleanup

Remove from Dashboard:
- WireGuard status card (entire section)
- OpenInviteSection component
- "Generate Invite" and "Redeem Invite" buttons from PeerList header
- Endpoint missing warning banner

PeerList stays on Dashboard but becomes purely a peer management view:
- Peer list with trust badges and last-seen
- Promote / Restrict / Remove actions
- No invite buttons (those are on Connection now)

Add a cross-link if no peers exist:
> No peers yet. Go to [Connection](/connection) to create or redeem an invite.

---

## File Change Summary

### New files
```
ui/web/src/pages/Connection.tsx         — main page
ui/web/src/components/NetworkStatus.tsx  — network status card
ui/web/src/components/ConnectionInfo.tsx — info panel (plain language help)
ui/web/src/components/InviteManager.tsx  — consolidated invite UI
ui/web/src/components/RelayConfig.tsx    — relay toggle card
ui/web/src/api/network.ts               — network API client
node/daemon/src/api/network_routes.rs   — /network/* route handlers
```

### Modified files
```
ui/web/src/App.tsx                      — add Connection route + nav link
ui/web/src/pages/Dashboard.tsx          — remove WG, invites, endpoint warning
ui/web/src/components/PeerList.tsx       — remove invite buttons, add cross-link
node/daemon/src/api/mod.rs              — wire network routes
node/daemon/src/config.rs               — add network.allow_relay config field
```

### Deleted files
```
ui/web/src/components/OpenInviteSection.tsx  — absorbed into InviteManager
```

---

## Daemon-Side Work (network_routes.rs)

The new `/network/*` routes are mostly aggregation and thin wrappers:

**GET /network/status** — Reads WG status (existing), nat_profile.json (file
read), enumerates IPv6 interfaces (new helper), computes reachability tier,
reads relay config from daemon config. Returns the combined NetworkStatus JSON.

**POST /network/detect** — Runs the STUN test battery (Phase 2 of
NETWORKING_FINAL). This is the actual NAT characterization: two STUN binding
requests, classify as open/cone/symmetric, cache result. Returns immediately
if cached and fresh (< 1 hour), otherwise runs detection (~2-3s) and returns
results. Requires auth.

**GET /network/nat-profile** — Simple file read of nat_profile.json. Returns
null/404 if never detected. No auth needed (read-only, non-sensitive).

**PUT /network/relay** — Updates `network.allow_relay` in daemon config.
Requires auth. Returns updated relay config.

**POST /node/accept** — Parses a `howm://accept/<payload>` token, validates it
references an outstanding invite from this node, configures WireGuard peer, and
begins the hole punch sequence. This is Tier 2 completion from the inviter's
side. Requires auth.

---

## Phasing

This work splits naturally:

### Phase A: Static Connection page (can ship now)

Move existing WG status and invite UI to the Connection page. No new daemon
endpoints needed — uses existing `/node/wireguard`, `/node/invite`,
`/node/open-invite`, etc. The info panel shows generic help text based on
whatever WG status we have. NAT type shows "Not detected" with a disabled
Re-detect button. Relay section shows as "Coming soon."

This is purely a frontend reorganization. Dashboard gets lighter, Connection
page exists and works with current data.

### Phase B: NAT detection integration (after NETWORKING_FINAL Phase 2)

Wire up `/network/status`, `/network/detect`, `/network/nat-profile`. The info
panel starts composing real personalized explanations. Re-detect button works.
Reachability badge shows real data.

### Phase C: Two-way exchange UI (after NETWORKING_FINAL Phase 3)

Add the "Redeem Accept" flow to InviteManager. Invite creation becomes
NAT-aware with tier-appropriate messaging. The info panel's "How Connections
Work For You" section activates.

### Phase D: Relay UI (after NETWORKING_FINAL Phase 4)

Relay toggle becomes functional. Relay section shows real peer counts. Info
panel includes relay explanations for symmetric NAT users.

---

## Info Drawer

The info panel renders as a right-side drawer that slides in over the page
content. Width: 400px on desktop, full-width on mobile. Semi-transparent
backdrop dims the main content but the drawer can stay open while scrolling.
Dismiss with the X button, clicking the backdrop, or pressing Escape.

### Structure

```
┌──────────────────────────────────────────┐
│  ✕   Understanding Your Connection       │
│──────────────────────────────────────────│
│                                          │
│  YOUR SETUP                              │
│  ─────────                               │
│  Your node has a public IPv6 address     │
│  (2001:db8::1) and is directly ...       │
│                                          │
│                                          │
│  HOW CONNECTIONS WORK FOR YOU            │
│  ────────────────────────────            │
│  When you invite someone:                │
│  1. Click "Create Invite" → get a link   │
│  2. Send the link to your friend ...     │
│                                          │
│  When someone invites you:               │
│  1. They send you a howm://invite/...    │
│  2. Paste it in "Redeem" → connected!    │
│                                          │
│                                          │
│  QUICK REFERENCE                         │
│  ───────────────                         │
│  ┌──────────┬────────┬──────┬──────┐     │
│  │ Them →   │ Public │ Cone │ Sym  │     │
│  ├──────────┼────────┼──────┼──────┤     │
│  │ You      │  ✓     │  ✓   │  ✓   │     │
│  └──────────┴────────┴──────┴──────┘     │
│                                          │
└──────────────────────────────────────────┘
```

### Implementation: `ConnectionInfo.tsx`

Props:
```typescript
interface ConnectionInfoProps {
  open: boolean;
  onClose: () => void;
  status: NetworkStatus;
}
```

The component is pure rendering — no data fetching. It receives the same
`NetworkStatus` the parent page already has and branches on `reachability`,
`nat.nat_type`, `ipv6.available`, `peer_count`, and `wireguard.endpoint`
to compose the three text sections.

Styling: uses a fixed-position container with `right: 0`, `transform:
translateX(100%)` when closed, `translateX(0)` when open, with a CSS
transition for the slide. Backdrop is a sibling div with
`background: rgba(0,0,0,0.4)` and `pointer-events: all`.

---

## Pending Invites

When a user creates a Tier 2 invite (two-way exchange), the invite has a
built-in expiration — default TTL is **15 minutes** (configurable via
`invite.ttl_s` in p2pcd config). The Pending Invites section shows
outstanding invites that are waiting for an accept token back.

### What Gets Tracked

The daemon already creates invite records with `expires_at` timestamps.
We add a lightweight tracking layer:

**New daemon state:** An in-memory list of "pending exchanges" — invites
generated by this node where `reachability != direct` (i.e., the inviter
knows this might need a two-way exchange). Each entry stores:

```rust
struct PendingExchange {
    invite_pubkey: String,     // the generated WG pubkey for this invite
    created_at: u64,           // unix timestamp
    expires_at: u64,           // from invite TTL
    status: ExchangeStatus,    // Waiting | Completed | Expired
}

enum ExchangeStatus {
    Waiting,     // invite sent, no accept received yet
    Completed,   // accept token redeemed, tunnel up
    Expired,     // TTL passed with no accept
}
```

This is volatile (in-memory, lost on daemon restart). That's fine — invites
expire in 15 minutes anyway, and a restart invalidates WG state.

### New API endpoint

| Method | Path | Description |
|---|---|---|
| GET | `/network/pending` | List pending two-way exchanges |

Returns:
```json
{
  "pending": [
    {
      "id": "abc123",
      "created_at": 1742320000,
      "expires_at": 1742320900,
      "status": "waiting",
      "time_remaining_secs": 542
    }
  ]
}
```

### UI: Pending Exchanges card

Shown in the Invites section when there are pending exchanges. Polls every
10 seconds to update countdowns and catch completions.

```
┌─── Pending Exchanges ──────────────────────────────────────┐
│                                                            │
│  ⏳ Waiting for response             8:42 remaining        │
│     Created just now                                       │
│     Paste their accept link below when they send it back.  │
│                                                            │
│  ✓  Connected!                       completed 2m ago      │
│     Peer joined via two-way exchange.                      │
│                                                            │
│  ✕  Expired                          expired 5m ago        │
│     No response received. Create a new invite to try again.│
│                                                            │
└────────────────────────────────────────────────────────────┘
```

Completed and expired entries linger for 10 minutes (cosmetic, helps the
user see what happened) then disappear. Active "Waiting" entries show a
live countdown.

When the user creates a Tier 2 invite, the Pending Exchanges card
automatically appears with the new entry, and the "Redeem Accept" input
is highlighted — making it obvious where to paste the response when it
comes back.

---

## Design Notes

### Why a separate page, not tabs on Dashboard

The Dashboard is the "what's happening now" view. Connection is "how do I reach
people." They serve different mental models. You check the Dashboard to see if
your node is healthy and who's connected. You go to Connection when you want to
add someone new or troubleshoot why you can't.

### Why an info panel, not tooltips

The networking stuff is genuinely complicated for non-technical users. Tooltips
can hold a sentence. This needs paragraphs. The info panel is the place where
howm can say "here's what's actually happening with your specific network, in
English." It's the thing that makes the difference between a user who gives up
at "symmetric NAT" and one who understands "oh, I just need to connect to
someone with a public IP first, then they can help bridge me to others."

### Info panel is reactive, not static

The info panel re-renders when network status changes. Run detection and the
explanation updates. Connect your first peer and the "you're isolated" warning
goes away. It's a living document about your node's current situation.

### Invite flow consolidation

Right now invite generation is on PeerList and open invites are their own
component. These are the same conceptual action (get someone connected to me)
split across two places. InviteManager unifies them. You go to Connection,
you see all the ways to invite someone, you pick one. One place, one mental
model.

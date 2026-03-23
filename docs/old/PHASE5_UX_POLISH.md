# Phase 5: UX Polish — Connection Page & Networking

## Context

The Connection page (`/connection`) already exists with four components:

- **NetworkStatus** — WG status, public IPs, NAT type, reachability badge, re-detect button
- **InviteManager** — create/redeem invites, open invites, pending two-way exchanges
- **RelayConfig** — relay toggle with peer count and description
- **ConnectionInfo** — slide-out info drawer with "Your Setup", "How Connections Work", and "Quick Reference" sections

All of this is backed by `/network/status`, `/network/detect`, `/network/relay`,
`/network/pending`, and the existing invite endpoints.

This proposal covers the Phase 5 items from NETWORKING_FINAL that aren't yet
implemented, plus additional polish that makes the Connection page feel complete
and genuinely helpful to non-technical users.

---

## What's Done vs What's Missing

| Phase 5 Item | Status | Notes |
|---|---|---|
| Invite creation with tier-appropriate messaging | ✓ Done | `inviteGuidance()` in InviteManager |
| NAT type display in settings/node info | ✓ Done | NetworkStatus shows NAT type + stride |
| Relay opt-in explanation | ✓ Done | RelayConfig has plain-language description |
| Info drawer with personalized setup explanation | ✓ Done | ConnectionInfo with reactive sections |
| Joiner flow with step-by-step guidance | ⚠ Partial | Drawer has it, but inline guidance during active redemption is missing |
| UNREACHABLE error display with tiers + suggestions | ✗ Missing | No error UX when connection attempts fail |
| Connection attempt progress/feedback | ✗ Missing | No visibility into what's happening during punch/relay |
| First-run guidance | ✗ Missing | New users land on Connection with no orientation |
| Mobile responsiveness for info drawer | ✗ Missing | Drawer is 400px fixed, no mobile adaptation |

---

## 1. UNREACHABLE Error Display

When a connection attempt fails (invite redemption, punch timeout, relay failure),
the user currently gets a generic error string. Phase 5 replaces this with a
structured error display that shows what was tried and what to do next.

### New type: `ConnectionAttempt`

```typescript
// api/network.ts

export type AttemptTier = 'direct' | 'punch' | 'relay';
export type AttemptResult = 'success' | 'timeout' | 'refused' | 'no-relay-path' | 'error';

export interface TierAttempt {
  tier: AttemptTier;
  result: AttemptResult;
  duration_ms: number;
  detail: string | null;  // e.g. "symmetric NAT detected", "no mutual peers with relay enabled"
}

export interface ConnectionAttempt {
  peer_id_short: string;      // first 8 hex chars
  started_at: number;
  completed_at: number;
  success: boolean;
  tiers_attempted: TierAttempt[];
  suggestion: string | null;  // daemon computes a plain-language suggestion
}
```

### Daemon-side: `POST /node/redeem-invite` and `POST /node/accept` changes

These endpoints currently return success or a generic error. Add a
`connection_attempt` field to both success and error responses:

```json
{
  "success": false,
  "error": "Connection failed after all tiers",
  "connection_attempt": {
    "peer_id_short": "a1b2c3d4",
    "started_at": 1742320000,
    "completed_at": 1742320018,
    "success": false,
    "tiers_attempted": [
      { "tier": "direct", "result": "timeout", "duration_ms": 5000, "detail": "no response on endpoint 203.0.113.5:41641" },
      { "tier": "punch", "result": "timeout", "duration_ms": 10000, "detail": "symmetric NAT — port prediction failed" },
      { "tier": "relay", "result": "no-relay-path", "duration_ms": 200, "detail": "no mutual peers with relay enabled" }
    ],
    "suggestion": "Your first connection should be with someone who has a public IP or IPv6. Once you have a peer with relay enabled, they can help bridge you to others behind NAT."
  }
}
```

The `suggestion` field is computed server-side based on:
- The user's NAT type
- Which tiers failed and why
- Current peer count and relay-capable peer count
- Whether the remote peer's NAT info was available

### Suggestion logic (daemon-side, `connection_attempt.rs`)

```
if all tiers failed:
  if user.nat_type == symmetric && user.peer_count == 0:
    "Your first connection should be with someone who has a public IP or IPv6.
     Once connected, they can help bridge you to others."

  if user.nat_type == symmetric && user.relay_capable_peers == 0:
    "None of your current peers have relay enabled. Ask a friend on your mesh
     to turn it on in their Connection settings, then try again."

  if tier_punch failed with "symmetric NAT":
    "Both you and this peer are behind restrictive NATs. You need a mutual
     friend already on both your meshes who has relay enabled."

  if tier_direct failed with "no response":
    "The peer's endpoint didn't respond. They may be offline, behind a
     firewall, or their endpoint address may have changed. Ask them to
     re-run network detection."

  if tier_punch failed with "timeout":
    "NAT punch-through timed out. This sometimes happens with strict
     firewalls. Try again — punching is probabilistic and sometimes needs
     a second attempt."
```

### UI: `ConnectionError` component

New component: `ui/web/src/components/ConnectionError.tsx`

Rendered inline in InviteManager when a redeem/accept mutation fails with
a `connection_attempt` payload.

```
┌─── Connection Failed ────────────────────────────────────────┐
│                                                               │
│  Could not connect to peer a1b2c3d4                          │
│                                                               │
│  What was tried:                                              │
│  ──────────────                                               │
│  1. Direct connection     ✕ timed out (5s)                   │
│     No response on their endpoint                             │
│                                                               │
│  2. NAT punch-through     ✕ timed out (10s)                  │
│     Symmetric NAT — port prediction failed                    │
│                                                               │
│  3. Relay via mutual peer ✕ no path                          │
│     No mutual peers with relay enabled                        │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ 💡 Your first connection should be with someone who    │   │
│  │    has a public IP or IPv6. Once connected, they can   │   │
│  │    help bridge you to others.                          │   │
│  └────────────────────────────────────────────────────────┘   │
│                                                               │
│  [Try Again]  [Open Info Panel]                               │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

**Styling:**
- Red border for the card (error state), but NOT an alarming full-red background
- Each tier attempt is a row with a result icon: ✓ green / ✕ muted red / ⏳ yellow
- The suggestion box uses the existing hint style (blue-tinted background)
- "Open Info Panel" button triggers the ConnectionInfo drawer for deeper context

**Integration with InviteManager:**

```typescript
// In InviteManager, after redeem mutations:
const lastAttempt = redeemMutation.error?.response?.data?.connection_attempt
  ?? acceptMutation.error?.response?.data?.connection_attempt
  ?? redeemOpenMutation.error?.response?.data?.connection_attempt;

// Render below the redeem panel when present:
{lastAttempt && <ConnectionError attempt={lastAttempt} onInfoClick={() => /* bubble up to Connection.tsx */} />}
```

---

## 2. Connection Progress Indicator

When a user redeems an invite, the connection process can take 2-18 seconds
(direct: 2s, punch: up to 15s, relay: variable). Currently the button just
says "Redeeming..." with no visibility into what's happening.

### New: Live progress during connection

The redeem endpoints become async with a status poll:

**Option A: Long-poll with progress (simpler)**

The existing `POST /node/redeem-invite` blocks until done but we add a
new `POST /node/redeem-invite-async` that returns immediately with a
`connection_id`, and a `GET /network/connection/:id` poll endpoint.

**Option B: SSE stream (richer but more complex)**

Not worth it for this. Option A is fine.

### Poll endpoint

```
GET /network/connection/:id

Response (in progress):
{
  "id": "conn_abc123",
  "status": "in_progress",
  "current_tier": "punch",
  "tiers_completed": [
    { "tier": "direct", "result": "timeout", "duration_ms": 5000 }
  ],
  "elapsed_ms": 7200
}

Response (done):
{
  "id": "conn_abc123",
  "status": "completed",   // or "failed"
  "success": true,
  "connection_attempt": { ... full ConnectionAttempt ... }
}
```

### UI: Progress display in InviteManager

Replace the flat "Redeeming..." button text with a mini progress view:

```
┌─── Connecting... ─────────────────────────────────────┐
│                                                        │
│  ✕ Direct connection — timed out                      │
│  ⏳ NAT punch-through — attempting... (3s)             │
│                                                        │
│  [Cancel]                                              │
└────────────────────────────────────────────────────────┘
```

Each tier lights up as it's attempted. Completed tiers show their result.
The current tier shows a spinner/elapsed time. If all tiers fail, this
transitions into the ConnectionError component from section 1.

### Implementation

New API functions in `api/network.ts`:

```typescript
export const redeemInviteAsync = (invite_code: string) =>
  api.post<{ connection_id: string }>('/node/redeem-invite-async', { invite_code }).then(r => r.data);

export const getConnectionProgress = (id: string) =>
  api.get<ConnectionProgress>(`/network/connection/${id}`).then(r => r.data);
```

InviteManager uses `useMutation` for the initial POST, then switches to
polling with `useQuery` (refetchInterval: 1000) until status !== 'in_progress'.

---

## 3. First-Run Guidance

When a user first opens the Connection page with no peers and no network
detection run, the page should feel welcoming rather than empty.

### Empty state: `ConnectionWelcome` component

Rendered at the top of the Connection page when `peer_count === 0` and
`reachability === 'unknown'`.

```
┌──────────────────────────────────────────────────────────────┐
│                                                               │
│  👋 Welcome to your Connection page                          │
│                                                               │
│  This is where you connect your howm to friends. Here's      │
│  how to get started:                                          │
│                                                               │
│  1. Detect your network                                       │
│     Let howm figure out your network setup so it can pick     │
│     the best connection strategy. Takes about 3 seconds.      │
│                                                               │
│     [Detect My Network]                                       │
│                                                               │
│  2. Create or redeem an invite                                │
│     Generate a link to send to a friend, or paste one         │
│     they've sent you.                                         │
│                                                               │
│  That's it. Once you're connected to one person, finding      │
│  others gets easier — your peers can help bridge you to       │
│  their peers.                                                 │
│                                                               │
│  Want to understand more? →  [ⓘ How connections work]        │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

This replaces the current state where a new user sees the full NetworkStatus
card with "Not detected" everywhere, which looks broken rather than fresh.

**Behavior:**
- The "Detect My Network" button in the welcome card calls the same
  `detectNetwork` mutation as NetworkStatus
- After detection completes, the welcome card updates to show step 1 as done
  with a checkmark, and the regular NetworkStatus card appears below
- After the first peer connects, the welcome card disappears permanently
  (keyed on `peer_count > 0`)
- The "How connections work" link opens the ConnectionInfo drawer

### File: `ui/web/src/components/ConnectionWelcome.tsx`

```typescript
interface ConnectionWelcomeProps {
  detected: boolean;
  onDetect: () => void;
  detecting: boolean;
  onInfoClick: () => void;
}
```

### Connection.tsx changes

```typescript
const showWelcome = status.peer_count === 0 && status.reachability === 'unknown';

return (
  <div style={pageStyle}>
    {/* ... header ... */}

    {showWelcome && (
      <ConnectionWelcome
        detected={status.nat?.detected ?? false}
        onDetect={() => detectMutation.mutate()}
        detecting={detectMutation.isPending}
        onInfoClick={() => setInfoOpen(true)}
      />
    )}

    {/* Regular sections shown always (or after welcome transitions out) */}
    <NetworkStatus status={status} />
    <InviteManager reachability={status.reachability} />
    <RelayConfig relay={status.relay} />
    {/* ... */}
  </div>
);
```

---

## 4. Inline Joiner Guidance

The ConnectionInfo drawer explains the two-way exchange flow, but during an
actual redemption the user needs inline guidance — not a separate drawer they
have to open.

### Smart redemption feedback in InviteManager

When a user redeems an invite and the response indicates a two-way exchange
is needed, the current flow generates a pending exchange entry. But the user
might not understand what to do next.

**Current behavior:** Pending exchange row says "Waiting for response" and
"Paste their accept link in Redeem when they send it back."

**New behavior:** Add a step-by-step inline guide that appears when a
two-way exchange is initiated:

```
┌─── Two-Way Exchange Started ─────────────────────────────────┐
│                                                               │
│  You and this peer are both behind NAT, so you need to       │
│  exchange connection info before you can connect.             │
│                                                               │
│  ✓ Step 1: Your invite has been sent                         │
│                                                               │
│  → Step 2: They paste your invite and get a response link    │
│            They need to send that link back to you.           │
│            (text, email, whatever you used to send the        │
│            invite)                                            │
│                                                               │
│  ○ Step 3: Paste their response link below                   │
│                                                               │
│  ┌────────────────────────────────────────────────────┐       │
│  │  howm://accept/...                          [Paste]│       │
│  └────────────────────────────────────────────────────┘       │
│                                                               │
│  ⏳ 12:34 remaining                                          │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

**Key differences from current:**
- Shows the full 3-step flow so the user knows what's happening
- Step indicators (✓ done, → current, ○ upcoming) make progress clear
- The accept input is right there in context, not hidden behind the Redeem tab
- The countdown timer is prominent

**Implementation:** This replaces the `PendingRow` component for "waiting"
status exchanges, or renders as an expanded view when there's exactly one
active pending exchange (the common case).

---

## 5. Info Drawer Polish

The existing ConnectionInfo drawer is solid but needs these refinements:

### 5a. Mobile responsiveness

Current: fixed 400px width. On screens < 600px the drawer should go
full-width with the close button in the top-right corner.

```typescript
// ConnectionInfo.tsx — update drawerStyle
const drawerStyle: React.CSSProperties = {
  position: 'fixed',
  top: 0, right: 0, bottom: 0,
  width: 'min(400px, 100vw)',  // ← this line
  // ... rest unchanged
};
```

### 5b. Deep-link to info sections

Allow opening the drawer to a specific section:

```typescript
// Connection.tsx
<ConnectionInfo
  open={infoOpen}
  section={infoSection}  // 'setup' | 'how' | 'reference' | null
  onClose={() => { setInfoOpen(false); setInfoSection(null); }}
  status={status}
/>
```

The "Open Info Panel" button in ConnectionError can link directly to the
relevant section (e.g., if relay failed, scroll to the relay explanation).

### 5c. Post-detection update animation

When the user runs detection while the drawer is open, the "Your Setup"
section should briefly highlight to show it updated. A simple 1-second
background flash (from the accent color to transparent) is enough.

### 5d. Interactive quick reference table

The current quick reference table is a static render. Make the cells
clickable — tapping a cell like "↔ two-way exchange" shows a tooltip
with a one-sentence explanation:

> "Both of you are behind NAT. You send an invite, they send a response
> link back, you paste it — then both sides punch through simultaneously."

---

## 6. Relay Status Integration

When Phase 4 (relay signaling) lands, the RelayConfig component needs
minor additions:

### 6a. Active relay sessions display

Show when your node is actively relaying for someone:

```
┌─── Relay ─────────────────────────────────────────────────────┐
│                                                                │
│  Allow Relay Signaling    [  ON  ]                             │
│                                                                │
│  When enabled, your node can help two of your peers who        │
│  can't reach each other directly exchange connection info.     │
│                                                                │
│  3 of your peers also have relay enabled.                      │
│                                                                │
│  Recent relay activity:                                        │
│  ─────────────────────                                         │
│  ✓ Helped peer a1b2 reach peer c3d4          2m ago           │
│  ✓ Helped peer e5f6 reach peer g7h8          15m ago          │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

### 6b. Relay request notification

When relay is OFF and a peer requests relay through this node, show an
inline prompt:

```
┌─────────────────────────────────────────────────────────────┐
│  A peer asked to relay through you. Enable relay signaling  │
│  to help your peers connect to each other.                  │
│                                                             │
│  [Enable Relay]  [Dismiss]                                  │
└─────────────────────────────────────────────────────────────┘
```

This requires a new daemon event/endpoint:

```
GET /network/relay-requests  →  { pending_requests: number, last_request_at: number | null }
```

---

## New Files

```
ui/web/src/components/ConnectionError.tsx     — structured error display (section 1)
ui/web/src/components/ConnectionWelcome.tsx    — first-run guidance (section 3)
```

## Modified Files

```
ui/web/src/pages/Connection.tsx               — welcome state, info section linking
ui/web/src/components/InviteManager.tsx        — progress indicator, inline joiner guide,
                                                 ConnectionError integration
ui/web/src/components/ConnectionInfo.tsx       — mobile width, section deep-links,
                                                 detection animation, interactive table
ui/web/src/components/RelayConfig.tsx          — relay activity, request notification
ui/web/src/api/network.ts                     — new types + endpoints
node/daemon/src/api/network_routes.rs         — connection_attempt in responses,
                                                 async redeem + progress poll,
                                                 relay-requests endpoint
node/daemon/src/punch.rs                      — emit tier attempt results
node/daemon/src/invite.rs                     — return connection_attempt on failure
```

## Daemon Work

### connection_attempt.rs (new file, ~120 lines)

Struct definitions for `ConnectionAttempt`, `TierAttempt`. A builder that
accumulates tier results as the connection flow progresses through
direct → punch → relay. The suggestion generator that maps failure patterns
to plain-language advice.

### Changes to invite redemption flow (~80 lines)

The `redeem_invite` and `redeem_open_invite` handlers currently call
`configure_wg_peer()` and `start_punch()` inline. Wrap these in a
`ConnectionAttemptBuilder` that records each tier's outcome. On success,
return the attempt with `success: true`. On failure, return it with the
suggestion.

### Async redemption (optional, ~60 lines)

If we go with the async poll approach (section 2), add:
- `POST /node/redeem-invite-async` — starts connection in background, returns ID
- `GET /network/connection/:id` — poll for progress
- In-memory `HashMap<String, ConnectionProgress>` with 5-minute TTL

This is optional — the sync endpoint with the structured error response
(section 1) already covers the most important case. The progress indicator
is nice-to-have and can ship later.

---

## Priority Order

1. **ConnectionError** (section 1) — highest value, directly addresses the
   "UNREACHABLE error display" Phase 5 item. Users currently get a wall of
   nothing when connections fail.

2. **First-run guidance** (section 3) — second highest, the empty Connection
   page is confusing for new users.

3. **Inline joiner guidance** (section 4) — improves the two-way exchange
   flow which is the most confusing part for users.

4. **Info drawer polish** (section 5) — refinements to existing good work.

5. **Connection progress** (section 2) — nice-to-have, requires async
   plumbing.

6. **Relay integration** (section 6) — depends on Phase 4 landing first.

---

## Relationship to Phase 4

Phase 4 (Peer Relay Signaling) needs to land before sections 2 (relay tier
in progress display) and 6 (relay status integration) are fully functional.
However, all other sections can ship independently — the ConnectionError
component simply shows "Relay: no path" as a static result until Phase 4
adds the actual relay attempt.

The UI is designed so that relay support slots in cleanly:
- ConnectionError already has a slot for the relay tier attempt
- InviteManager's progress indicator already has a relay step
- ConnectionInfo's drawer already explains relay for symmetric NAT users
- RelayConfig already has the toggle and description

Phase 4 delivers the daemon machinery. Phase 5 delivers the UX that makes
it understandable.

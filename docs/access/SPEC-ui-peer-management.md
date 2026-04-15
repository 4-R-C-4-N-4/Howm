# UI Spec: Peer Management & Access Control

**Source BRD:** `docs/access/BRD-access-control.md`
**Implementation Spec:** `docs/access/SPEC-access-control-implementation.md`
**Author:** IV (design), spec by agent
**Date:** 2026-03-23
**Status:** Draft — awaiting review

---

## 0. Context

### 0.1 What Exists Today

The UI is a React + TypeScript SPA at `ui/web/`. Stack:

- **React 18** with `react-router-dom` (BrowserRouter, Routes)
- **@tanstack/react-query** for data fetching + cache invalidation
- **axios** via `api/client.ts` (Bearer token from `<meta>` tag)
- **Inline styles** — no CSS framework, no Tailwind; all styling is `React.CSSProperties` objects
- **Dark theme** — dark backgrounds (`#0a0a0a`, `#111`), light text, accent colors via CSS vars

Current pages: Dashboard, Connection, Settings, CapabilityPage.

Current peer UI: `PeerList.tsx` component on Dashboard — flat list showing name, trust badge (Friend/Public/Restricted), last seen, with a trust dropdown and remove button inline. Uses the old `TrustLevel` enum and `PATCH /node/peers/:id/trust` endpoint.

### 0.2 What's New

Phase 1+2 built the `howm-access` crate and `/access/*` API routes. The old TrustLevel system is being deprecated. The UI needs to:

1. Replace the trust dropdown with group-based controls
2. Add a peer detail view
3. Add a groups management page
4. Integrate demotion/deny warnings
5. Surface effective permissions visually

### 0.3 API Surface (already implemented)

All routes are localhost-only, Bearer-authenticated.

```
GET    /access/groups                           → Group[]
POST   /access/groups                           → Group
GET    /access/groups/:group_id                 → Group (with members)
PUT    /access/groups/:group_id                 → Group
DELETE /access/groups/:group_id                 → { status, group_id }

GET    /access/peers/:peer_id/groups            → Group[]
POST   /access/peers/:peer_id/groups            → { status, peer_id, group_id, assigned_at }
DELETE /access/peers/:peer_id/groups/:group_id  → { status, peer_id, group_id }
GET    /access/peers/:peer_id/permissions       → { peer_id, permissions: { [cap]: { allowed, rate_limit?, ttl? } } }
POST   /access/peers/:peer_id/deny             → { status, peer_id, groups_removed, session_closed }

GET    /node/peers                              → { peers: Peer[] }  (existing)
DELETE /node/peers/:node_id                     → (existing)
```

Peer IDs in access routes are 64-char hex (32-byte WG pubkey). Node IDs in `/node/peers` routes are the same key, different encoding — the UI will need to normalize.

---

## 1. Design Principles

**Tier mental model, not group algebra.** Most users think in tiers: Default → Friends → Trusted. The UI presents this as the primary interaction. Multi-group assignment exists for power users but isn't the default path.

**Show what changes.** Every promotion/demotion shows the concrete capability delta — what the peer gains or loses. Never just "moved to Default."

**Destructive actions require confirmation.** Deny (session termination) gets a modal. Demotion gets an inline warning with the capability delta. Promotion is instant — no confirmation needed for granting access.

**Immediate effect.** All changes take effect on save. The backend triggers P2P-CD rebroadcast automatically. The UI should show a brief "Permissions updated" toast.

**No loading spinners for cached data.** Use react-query's stale-while-revalidate. Show stale peer list instantly, refresh in background.

---

## 2. Information Architecture

```
/peers                    ← Peer List (new top-level page)
/peers/:peer_id           ← Peer Detail
/access/groups            ← Groups Management
/access/groups/:group_id  ← Group Detail
```

Nav bar gets a new "Peers" link between Dashboard and Connection. The old `PeerList` component on Dashboard is replaced with a compact summary + link to `/peers`.

---

## 3. Peer List Page (`/peers`)

### 3.1 Layout

```
┌─────────────────────────────────────────────────────────┐
│  Peers (12)                              [+ Invite]     │
│─────────────────────────────────────────────────────────│
│  🔍 Search peers...                    Filter: [All ▾]  │
│─────────────────────────────────────────────────────────│
│  ● alice           Trusted 🟡    2m ago    ⋯            │
│  ● bob             Friends 🔵    just now  ⋯            │
│  ○ charlie         Default 🔘    3d ago    ⋯            │
│  ○ dave            Custom  🟣    14h ago   ⋯            │
│  ✕ eve             Denied  🔴    —         ⋯            │
└─────────────────────────────────────────────────────────┘
```

### 3.2 Peer Row

Each row displays:

| Element | Source | Notes |
|---------|--------|-------|
| Online indicator | `●` green if last_seen < 90s, `○` gray otherwise, `✕` red if denied | Heartbeat interval is 30s; 3× = stale |
| Name | `peer.name` from `/node/peers` | Truncate at 24 chars |
| Tier badge | Derived from group memberships | See §3.3 |
| Last seen | `peer.last_seen` formatted relative | "just now", "2m ago", "3d ago", "never" |
| Overflow menu `⋯` | Click → dropdown | Quick actions (see §3.4) |

Clicking anywhere on the row (except overflow) navigates to `/peers/:peer_id`.

### 3.3 Tier Badge Derivation

The badge shows the peer's *effective tier* — their highest-privilege built-in group. Derived client-side from the group memberships list:

```typescript
function effectiveTier(groups: Group[]): TierBadge {
  const builtInIds = new Set(groups.filter(g => g.built_in).map(g => g.group_id));
  if (builtInIds.has(GROUP_TRUSTED)) return { label: 'Trusted', color: '#fbbf24', bg: 'rgba(251,191,36,0.12)' };
  if (builtInIds.has(GROUP_FRIENDS)) return { label: 'Friends', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' };
  if (builtInIds.has(GROUP_DEFAULT)) return { label: 'Default', color: '#9ca3af', bg: 'rgba(156,163,175,0.12)' };
  // No built-in group — check if in any custom group
  if (groups.length > 0)           return { label: 'Custom',  color: '#c084fc', bg: 'rgba(192,132,252,0.12)' };
  // No groups at all = effectively denied
  return { label: 'Denied', color: '#f87171', bg: 'rgba(248,113,113,0.12)' };
}
```

Well-known UUIDs (constants shared between backend and frontend):

```typescript
const GROUP_DEFAULT  = '00000000-0000-0000-0000-000000000001';
const GROUP_FRIENDS  = '00000000-0000-0000-0000-000000000002';
const GROUP_TRUSTED  = '00000000-0000-0000-0000-000000000003';
```

### 3.4 Overflow Menu Quick Actions

```
┌──────────────────────┐
│  Move to Trusted     │
│  Move to Friends     │
│  Move to Default     │
│  ─────────────────── │
│  🔴 Deny Peer        │
└──────────────────────┘
```

"Move to X" means: remove from all built-in groups, assign to X. Custom group memberships are preserved. This matches the tier mental model.

The current tier is shown with a checkmark and is non-clickable.

### 3.5 Filter Dropdown

Options: All, Trusted, Friends, Default, Custom, Denied, Online.

Filter is client-side against the already-fetched peer+groups data. No extra API call.

### 3.6 Search

Client-side filter on `peer.name`. Debounced 200ms. Clears on Escape.

### 3.7 Data Fetching

```typescript
// Fetch all peers (existing endpoint)
const { data: peers } = useQuery({
  queryKey: ['peers'],
  queryFn: getPeers,
  refetchInterval: 30_000,
});

// Fetch all groups (for badge derivation + filter)
const { data: groups } = useQuery({
  queryKey: ['access-groups'],
  queryFn: getAccessGroups,
  refetchInterval: 60_000,
});

// Fetch per-peer group memberships (batch — one call per peer)
// OR: add a new batch endpoint GET /access/peers/memberships → { [peer_id]: Group[] }
// For now: individual calls, cached aggressively
const { data: peerGroups } = useQuery({
  queryKey: ['peer-groups', peerId],
  queryFn: () => getPeerGroups(peerId),
  staleTime: 60_000,
});
```

**Optimization note:** For nodes with many peers (50+), a batch memberships endpoint would be better than N individual calls. This can be added later without UI changes — just swap the query function.

---

## 4. Peer Detail Page (`/peers/:peer_id`)

### 4.1 Layout

```
┌─────────────────────────────────────────────────────────┐
│  ← Back to Peers                                        │
│                                                         │
│  alice                                        ● Online  │
│  Trusted 🟡                                             │
│                                                         │
│  ┌─ Identity ─────────────────────────────────────────┐ │
│  │  Node ID:     a1b2c3d4...ef56  [copy]              │ │
│  │  WG Pubkey:   Xk9mP2...Q=     [copy]              │ │
│  │  WG Address:  100.222.1.7                          │ │
│  │  WG Endpoint: 203.0.113.5:51820                    │ │
│  │  First seen:  2026-03-15                           │ │
│  │  Last seen:   2 minutes ago                        │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  ┌─ Access Level ─────────────────────────────────────┐ │
│  │                                                    │ │
│  │  [  Default  ] [  Friends  ] [ ★Trusted★ ]         │ │
│  │                                                    │ │
│  │  Groups: howm.trusted ✕  my-custom-group ✕  [+]    │ │
│  │                                                    │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  ┌─ Effective Permissions ────────────────────────────┐ │
│  │                                                    │ │
│  │  ✓ core.session.heartbeat.1                        │ │
│  │  ✓ core.session.attest.1                           │ │
│  │  ✓ core.session.latency.1                          │ │
│  │  ✓ core.network.endpoint.1                         │ │
│  │  ✓ core.session.timesync.1                         │ │
│  │  ✓ howm.social.feed.1                              │ │
│  │  ✓ howm.social.messaging.1                         │ │
│  │  ✓ howm.social.files.1                             │ │
│  │  ✓ howm.world.room.1                               │ │
│  │  ✓ core.network.peerexchange.1                     │ │
│  │  ✓ core.network.relay.1                            │ │
│  │                                                    │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  [🔴 Deny Peer]                                        │
└─────────────────────────────────────────────────────────┘
```

### 4.2 Access Level Section

**Tier selector:** Three buttons in a segmented control. The active tier is highlighted. Clicking a different tier triggers the "Move to" action (same as overflow menu — remove from all built-in groups, assign to new one).

**Group chips:** Below the tier selector, show all group memberships as removable chips/tags. Built-in groups show as their tier name. Custom groups show their custom name. Each has an `✕` to remove. The `[+]` button opens a dropdown of available groups to add.

Removing a built-in group chip auto-recalculates the tier selector highlight. If no built-in groups remain, the tier selector shows nothing highlighted and the badge becomes "Custom" or "Denied" depending on remaining memberships.

### 4.3 Effective Permissions Section

Fetched from `GET /access/peers/:peer_id/permissions`. Displayed as a capability list:

- `✓` green — allowed
- `✕` red — denied
- Gray subtext for rate_limit/ttl if present: `✓ howm.social.feed.1  (rate: 100/min)`

Capabilities are sorted: allowed first (alphabetical), then denied (alphabetical).

This section is read-only. Permissions are derived from group memberships, not editable per-peer.

### 4.4 Promotion Flow (e.g., Default → Friends)

1. User clicks "Friends" in tier selector
2. Capability delta toast appears briefly:
   ```
   ✓ Granting: social.feed, social.messaging, social.files, world.room, peerexchange
   ```
3. API calls: `DELETE /access/peers/:id/groups/...0001`, `POST /access/peers/:id/groups { group_id: "...0002" }`
4. React-query invalidates `['peer-groups', peerId]` and `['peer-permissions', peerId]`
5. Permissions section updates

No confirmation modal for promotions — granting access is non-destructive.

### 4.5 Demotion Flow (e.g., Friends → Default)

1. User clicks "Default" in tier selector
2. **Inline warning appears** below the tier selector:

```
┌─ Warning ──────────────────────────────────────────────┐
│                                                        │
│  Moving alice to Default will remove access to:        │
│                                                        │
│  ✕ howm.social.feed.1                                  │
│  ✕ howm.social.messaging.1                             │
│  ✕ howm.social.files.1                                 │
│  ✕ howm.world.room.1                                   │
│  ✕ core.network.peerexchange.1                         │
│                                                        │
│  This takes effect immediately. alice's active session  │
│  will be renegotiated.                                 │
│                                                        │
│          [Cancel]  [Confirm Demotion]                   │
└────────────────────────────────────────────────────────┘
```

3. On confirm: same API calls as promotion, reversed direction
4. Warning dismisses, permissions section updates

The warning is computed client-side by diffing the current permissions against the target tier's known capability set. The well-known capability sets for each built-in tier are constants:

```typescript
const TIER_CAPABILITIES: Record<string, string[]> = {
  [GROUP_DEFAULT]: [
    'core.session.heartbeat.1',
    'core.session.attest.1',
    'core.session.latency.1',
    'core.network.endpoint.1',
    'core.session.timesync.1',
  ],
  [GROUP_FRIENDS]: [
    // includes all default caps, plus:
    'howm.social.feed.1',
    'howm.social.messaging.1',
    'howm.social.files.1',
    'howm.world.room.1',
    'core.network.peerexchange.1',
  ],
  [GROUP_TRUSTED]: [
    // includes all friends caps, plus:
    'core.network.relay.1',
  ],
};
```

### 4.6 Deny Flow

1. User clicks "Deny Peer" button (red, bottom of page)
2. **Modal overlay** appears:

```
┌─────────────────────────────────────────────────────────┐
│                                                         │
│                    ⚠️  Deny alice?                       │
│                                                         │
│  This will:                                             │
│                                                         │
│  • Revoke ALL access immediately                        │
│  • Close their active P2P-CD session (AuthFailure)      │
│  • Remove them from all groups                          │
│  • They cannot reconnect until you re-add them          │
│                                                         │
│  alice will notice — their connection will drop.        │
│                                                         │
│              [Cancel]    [🔴 Deny alice]                 │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

3. On confirm: `POST /access/peers/:peer_id/deny`
4. Redirect to `/peers` with toast: "alice has been denied"
5. Peer appears in list with Denied badge

### 4.7 Data Fetching

```typescript
// Peer info (from existing endpoint, find by peer_id)
const { data: peer } = useQuery({
  queryKey: ['peer', peerId],
  queryFn: () => getPeerById(peerId),
});

// Group memberships
const { data: peerGroups } = useQuery({
  queryKey: ['peer-groups', peerId],
  queryFn: () => getPeerGroups(peerId),
});

// Effective permissions
const { data: permissions } = useQuery({
  queryKey: ['peer-permissions', peerId],
  queryFn: () => getPeerPermissions(peerId),
});
```

---

## 5. Groups Management Page (`/access/groups`)

### 5.1 Layout

```
┌─────────────────────────────────────────────────────────┐
│  Access Groups                        [+ Create Group]  │
│─────────────────────────────────────────────────────────│
│                                                         │
│  ┌─ Built-in ────────────────────────────────────────┐  │
│  │                                                    │  │
│  │  🔘 howm.default     5 peers     5 capabilities    │  │
│  │  🔵 howm.friends     3 peers    10 capabilities    │  │
│  │  🟡 howm.trusted     1 peer     11 capabilities    │  │
│  │                                                    │  │
│  └────────────────────────────────────────────────────┘  │
│                                                         │
│  ┌─ Custom ──────────────────────────────────────────┐  │
│  │                                                    │  │
│  │  🟣 media-viewers    2 peers     7 capabilities    │  │
│  │  🟣 relay-nodes      1 peer      6 capabilities    │  │
│  │                                                    │  │
│  └────────────────────────────────────────────────────┘  │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

Clicking a group row navigates to `/access/groups/:group_id`.

Built-in groups are always listed first, in tier order (default → friends → trusted). Custom groups below, sorted alphabetically.

Built-in groups cannot be deleted and their capability rules cannot be edited. The UI must reflect this — no delete button, no edit on capability rules. Name and description of built-in groups are also immutable.

### 5.2 Create Group Modal

Triggered by `[+ Create Group]` button.

```
┌─────────────────────────────────────────────────────────┐
│  Create Access Group                                    │
│                                                         │
│  Name:         [_________________________]              │
│  Description:  [_________________________]              │
│                                                         │
│  ┌─ Capabilities ────────────────────────────────────┐  │
│  │                                                    │  │
│  │  ☑ core.session.heartbeat.1                        │  │
│  │  ☑ core.session.attest.1                           │  │
│  │  ☑ core.session.latency.1                          │  │
│  │  ☑ core.network.endpoint.1                         │  │
│  │  ☑ core.session.timesync.1                         │  │
│  │  ☐ howm.social.feed.1                              │  │
│  │  ☐ howm.social.messaging.1                         │  │
│  │  ☐ howm.social.files.1                             │  │
│  │  ☐ howm.world.room.1                               │  │
│  │  ☐ core.network.peerexchange.1                     │  │
│  │  ☐ core.network.relay.1                            │  │
│  │                                                    │  │
│  │  Presets: [Default] [Friends] [Trusted] [None]     │  │
│  │                                                    │  │
│  └────────────────────────────────────────────────────┘  │
│                                                         │
│                    [Cancel]  [Create]                    │
└─────────────────────────────────────────────────────────┘
```

- Capability list is populated from the local manifest (`GET /capabilities` or hardcoded known set)
- Preset buttons check/uncheck to match a built-in tier's capability set — convenience, not required
- Core session capabilities (heartbeat, attest, latency, endpoint, timesync) are checked by default and have a note: "Recommended — required for basic connectivity"
- Name validation: 1-64 chars, no leading/trailing whitespace
- API: `POST /access/groups`

### 5.3 Data Fetching

```typescript
const { data: groups } = useQuery({
  queryKey: ['access-groups'],
  queryFn: getAccessGroups,
  refetchInterval: 60_000,
});
```

---

## 6. Group Detail Page (`/access/groups/:group_id`)

### 6.1 Layout

```
┌─────────────────────────────────────────────────────────┐
│  ← Back to Groups                                       │
│                                                         │
│  howm.friends                              Built-in 🔒  │
│  Social capabilities + room access + peer exchange.     │
│                                                         │
│  ┌─ Members (3) ─────────────────────────────────────┐  │
│  │                                                    │  │
│  │  ● alice     Trusted 🟡   [Remove from group]      │  │
│  │  ● bob       Friends 🔵   [Remove from group]      │  │
│  │  ○ charlie   Friends 🔵   [Remove from group]      │  │
│  │                                                    │  │
│  │                            [+ Add Peer]            │  │
│  └────────────────────────────────────────────────────┘  │
│                                                         │
│  ┌─ Capability Rules ────────────────────────────────┐  │
│  │                                                    │  │
│  │  ✓ core.session.heartbeat.1                        │  │
│  │  ✓ core.session.attest.1                           │  │
│  │  ✓ core.session.latency.1                          │  │
│  │  ✓ core.network.endpoint.1                         │  │
│  │  ✓ core.session.timesync.1                         │  │
│  │  ✓ howm.social.feed.1                              │  │
│  │  ✓ howm.social.messaging.1                         │  │
│  │  ✓ howm.social.files.1                             │  │
│  │  ✓ howm.world.room.1                               │  │
│  │  ✓ core.network.peerexchange.1                     │  │
│  │  ✕ core.network.relay.1                            │  │
│  │                                                    │  │
│  └────────────────────────────────────────────────────┘  │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 6.2 Built-in vs Custom Behavior

| Feature | Built-in | Custom |
|---------|----------|--------|
| Edit name | No | Yes (inline edit) |
| Edit description | No | Yes (inline edit) |
| Edit capability rules | No | Yes (checkbox toggles) |
| Delete group | No (no button shown) | Yes (with confirmation) |
| Add/remove members | Yes | Yes |

For custom groups, capability rules are editable checkboxes. Changes auto-save with debounce (500ms after last toggle) via `PUT /access/groups/:id`.

### 6.3 Add Peer Dropdown

`[+ Add Peer]` opens a searchable dropdown of all peers NOT already in this group. Selecting a peer calls `POST /access/peers/:peer_id/groups`.

### 6.4 Delete Custom Group

At page bottom for custom groups only:

```
┌─ Danger Zone ────────────────────────────────────────┐
│                                                      │
│  Delete this group? Members will be removed from     │
│  this group but retain other group memberships.      │
│                                                      │
│  [🔴 Delete Group]                                   │
│                                                      │
└──────────────────────────────────────────────────────┘
```

Confirmation modal: "Delete 'media-viewers'? 2 peers will be removed from this group."

API: `DELETE /access/groups/:group_id`

---

## 7. Dashboard Integration

Replace the current `PeerList` component on Dashboard with a compact summary card:

```
┌─ Peers ──────────────────────────────────────────────┐
│                                                      │
│  12 peers  •  7 online  •  3 friends  •  1 trusted   │
│                                                      │
│  Recent:                                             │
│  ● bob      Friends 🔵   just now                    │
│  ● alice    Trusted 🟡   2m ago                      │
│  ○ charlie  Default 🔘   3d ago                      │
│                                                      │
│  [View All Peers →]                                  │
│                                                      │
└──────────────────────────────────────────────────────┘
```

Shows the 3 most recently seen peers. "View All Peers" links to `/peers`.

---

## 8. Invite Flow Integration

When a new peer joins via invite (open or direct), they land in `howm.default`. The UI should surface this via a toast notification on the Dashboard and Peers pages:

```
┌──────────────────────────────────────────────────────┐
│  🆕 new-peer-name just joined via invite link        │
│     Currently: Default                               │
│     [Promote to Friend]  [View]                      │
└──────────────────────────────────────────────────────┘
```

The toast persists for 15 seconds or until dismissed. "Promote to Friend" is a one-click action that calls the same move-to-group API sequence. "View" navigates to the peer detail page.

Detection: compare peer list between react-query refetches. If a new peer_id appears that wasn't in the previous data, show the toast.

---

## 9. New API Client Functions

Add to `ui/web/src/api/access.ts` (new file):

```typescript
import api from './client';

// ── Types ──────────────────────────────────────────────

export interface AccessGroup {
  group_id: string;
  name: string;
  built_in: boolean;
  description: string | null;
  capabilities: CapabilityRule[];
  created_at: number;
}

export interface CapabilityRule {
  capability_name: string;
  allow: boolean;
  rate_limit: number | null;
  ttl: number | null;
}

export interface PeerPermissions {
  peer_id: string;
  permissions: Record<string, {
    allowed: boolean;
    rate_limit?: number | null;
    ttl?: number | null;
  }>;
}

// ── Well-known UUIDs ───────────────────────────────────

export const GROUP_DEFAULT  = '00000000-0000-0000-0000-000000000001';
export const GROUP_FRIENDS  = '00000000-0000-0000-0000-000000000002';
export const GROUP_TRUSTED  = '00000000-0000-0000-0000-000000000003';

// ── Well-known capability sets per built-in tier ───────

export const TIER_CAPABILITIES: Record<string, string[]> = {
  [GROUP_DEFAULT]: [
    'core.session.heartbeat.1',
    'core.session.attest.1',
    'core.session.latency.1',
    'core.network.endpoint.1',
    'core.session.timesync.1',
  ],
  [GROUP_FRIENDS]: [
    ...['core.session.heartbeat.1', 'core.session.attest.1', 'core.session.latency.1',
        'core.network.endpoint.1', 'core.session.timesync.1'],
    'howm.social.feed.1',
    'howm.social.messaging.1',
    'howm.social.files.1',
    'howm.world.room.1',
    'core.network.peerexchange.1',
  ],
  [GROUP_TRUSTED]: [
    ...['core.session.heartbeat.1', 'core.session.attest.1', 'core.session.latency.1',
        'core.network.endpoint.1', 'core.session.timesync.1',
        'howm.social.feed.1', 'howm.social.messaging.1', 'howm.social.files.1',
        'howm.world.room.1', 'core.network.peerexchange.1'],
    'core.network.relay.1',
  ],
};

// ── Group API ──────────────────────────────────────────

export const getAccessGroups = () =>
  api.get<AccessGroup[]>('/access/groups').then(r => r.data);

export const createAccessGroup = (name: string, description?: string, capabilities?: CapabilityRule[]) =>
  api.post<AccessGroup>('/access/groups', { name, description, capabilities }).then(r => r.data);

export const getAccessGroup = (groupId: string) =>
  api.get<AccessGroup>(`/access/groups/${groupId}`).then(r => r.data);

export const updateAccessGroup = (groupId: string, updates: {
  name?: string;
  description?: string | null;
  capabilities?: CapabilityRule[];
}) =>
  api.put<AccessGroup>(`/access/groups/${groupId}`, updates).then(r => r.data);

export const deleteAccessGroup = (groupId: string) =>
  api.delete(`/access/groups/${groupId}`).then(r => r.data);

// ── Peer Group Membership API ──────────────────────────

export const getPeerGroups = (peerId: string) =>
  api.get<AccessGroup[]>(`/access/peers/${peerId}/groups`).then(r => r.data);

export const assignPeerToGroup = (peerId: string, groupId: string) =>
  api.post(`/access/peers/${peerId}/groups`, { group_id: groupId }).then(r => r.data);

export const removePeerFromGroup = (peerId: string, groupId: string) =>
  api.delete(`/access/peers/${peerId}/groups/${groupId}`).then(r => r.data);

// ── Permissions API ────────────────────────────────────

export const getPeerPermissions = (peerId: string) =>
  api.get<PeerPermissions>(`/access/peers/${peerId}/permissions`).then(r => r.data);

// ── Deny API ───────────────────────────────────────────

export const denyPeer = (peerId: string) =>
  api.post(`/access/peers/${peerId}/deny`).then(r => r.data);

// ── Convenience: Move peer to a built-in tier ──────────

export async function movePeerToTier(peerId: string, targetGroupId: string): Promise<void> {
  const currentGroups = await getPeerGroups(peerId);
  const builtInGroups = currentGroups.filter(g => g.built_in);

  // Remove from all current built-in groups
  await Promise.all(
    builtInGroups
      .filter(g => g.group_id !== targetGroupId)
      .map(g => removePeerFromGroup(peerId, g.group_id))
  );

  // Assign to target if not already in it
  if (!builtInGroups.some(g => g.group_id === targetGroupId)) {
    await assignPeerToGroup(peerId, targetGroupId);
  }
}
```

---

## 10. New Components

### 10.1 File Structure

```
ui/web/src/
├── api/
│   ├── access.ts              ← NEW: access control API client
│   └── ... (existing)
├── components/
│   ├── PeerList.tsx            ← REWRITE: compact dashboard card
│   ├── PeerRow.tsx             ← NEW: shared row component
│   ├── TierSelector.tsx        ← NEW: segmented control for tier
│   ├── GroupChips.tsx          ← NEW: removable group tags
│   ├── PermissionGrid.tsx      ← NEW: capability allow/deny list
│   ├── DemotionWarning.tsx     ← NEW: inline capability delta warning
│   ├── DenyModal.tsx           ← NEW: deny confirmation modal
│   ├── CreateGroupModal.tsx    ← NEW: group creation form
│   ├── NewPeerToast.tsx        ← NEW: invite arrival notification
│   └── ... (existing)
├── pages/
│   ├── PeersPage.tsx           ← NEW: /peers
│   ├── PeerDetail.tsx          ← NEW: /peers/:peer_id
│   ├── GroupsPage.tsx          ← NEW: /access/groups
│   ├── GroupDetail.tsx         ← NEW: /access/groups/:group_id
│   └── ... (existing)
```

### 10.2 Shared Constants

```typescript
// ui/web/src/lib/access.ts

export const GROUP_DEFAULT  = '00000000-0000-0000-0000-000000000001';
export const GROUP_FRIENDS  = '00000000-0000-0000-0000-000000000002';
export const GROUP_TRUSTED  = '00000000-0000-0000-0000-000000000003';

export const BUILT_IN_TIERS = [
  { id: GROUP_DEFAULT, label: 'Default', color: '#9ca3af', order: 0 },
  { id: GROUP_FRIENDS, label: 'Friends', color: '#60a5fa', order: 1 },
  { id: GROUP_TRUSTED, label: 'Trusted', color: '#fbbf24', order: 2 },
] as const;

export function peerIdToHex(pubkey: string): string {
  // Convert base64 WG pubkey to hex for access API calls
  const bytes = atob(pubkey);
  return Array.from(bytes, b => b.charCodeAt(0).toString(16).padStart(2, '0')).join('');
}
```

---

## 11. Route Registration

In `App.tsx`, add new routes:

```tsx
import { PeersPage } from './pages/PeersPage';
import { PeerDetail } from './pages/PeerDetail';
import { GroupsPage } from './pages/GroupsPage';
import { GroupDetail } from './pages/GroupDetail';

// Inside <Routes>:
<Route path="/peers" element={<PeersPage />} />
<Route path="/peers/:peerId" element={<PeerDetail />} />
<Route path="/access/groups" element={<GroupsPage />} />
<Route path="/access/groups/:groupId" element={<GroupDetail />} />
```

Nav bar addition (in `NavBar` component):

```tsx
<NavLink to="/peers" style={linkStyle}>Peers</NavLink>
```

Position: after Dashboard, before Connection.

---

## 12. Peer ID Normalization

The existing `/node/peers` endpoint returns peers with `node_id` and `wg_pubkey` (base64). The access routes expect 64-char hex (raw 32 bytes). The UI must convert between them.

The canonical identifier for access operations is the **WG public key as hex**. The conversion:

```typescript
// base64 WG pubkey → 64-char hex
function wgPubkeyToHex(base64Pubkey: string): string {
  const binary = atob(base64Pubkey);
  return Array.from(binary, c => c.charCodeAt(0).toString(16).padStart(2, '0')).join('');
}

// 64-char hex → base64 WG pubkey (for display)
function hexToWgPubkey(hex: string): string {
  const bytes = hex.match(/.{2}/g)!.map(b => parseInt(b, 16));
  return btoa(String.fromCharCode(...bytes));
}
```

The PeerDetail page URL uses hex encoding: `/peers/a1b2c3...ef56`. Peer list rows link to this.

---

## 13. Styling Conventions

Follow existing patterns from the codebase:

- **All inline styles** via `React.CSSProperties` objects (no CSS modules, no Tailwind)
- **Dark palette:** `#0a0a0a` page bg, `#111` / `#1a1a1a` card bg, `#222` borders
- **Text colors:** `#e5e5e5` primary, `#888` / `#666` muted
- **Accent colors:** Use CSS vars where available (`var(--howm-success)`, `var(--howm-warning)`, `var(--howm-error)`), fall back to hardcoded values
- **Border radius:** `8px` for cards, `4px` for badges/chips
- **Spacing:** `12px`-`16px` padding in cards, `8px` gaps between items
- **Font sizes:** `0.875rem` for secondary text, `0.75rem` for badges

---

## 14. Error Handling

All mutations use react-query's `onError` callback to show toast notifications:

```typescript
const mutation = useMutation({
  mutationFn: ...,
  onSuccess: () => {
    queryClient.invalidateQueries({ queryKey: ['peer-groups', peerId] });
    queryClient.invalidateQueries({ queryKey: ['peer-permissions', peerId] });
    showToast('success', 'Permissions updated');
  },
  onError: (err: AxiosError) => {
    const msg = (err.response?.data as any)?.error || 'Failed to update permissions';
    showToast('error', msg);
  },
});
```

Toast system: reuse the existing `ToastContainer` from `App.tsx`. Add a toast context or lift toast state to App level so child pages can trigger toasts.

---

## 15. Migration Path

### 15.1 Old → New Transition

The old `PeerList.tsx` uses `TrustLevel` ('friend' | 'public' | 'restricted') and `updatePeerTrust()`. During the migration:

1. **Phase A:** Add new pages + components alongside old PeerList
2. **Phase B:** Swap Dashboard PeerList for compact summary card
3. **Phase C:** Remove old `TrustLevel` type, `updatePeerTrust()` from `api/nodes.ts`
4. **Phase D:** Remove `PATCH /node/peers/:id/trust` from daemon API

The old trust endpoint should return 410 Gone once Phase C is complete, pointing to the new access API.

### 15.2 Backend Endpoint Additions Needed

One new endpoint would improve performance:

```
GET /access/peers/memberships → { [peer_id_hex]: AccessGroup[] }
```

Batch fetch all peer-group memberships in one call. Not blocking for initial implementation (individual calls work fine for < 50 peers) but should be added before this is considered production-ready.

---

## 16. Out of Scope (for now)

- **Rate limit / TTL editing in peer detail view** — only visible in group detail
- **Audit log** of permission changes
- **Bulk operations** (multi-select peers, batch move)
- **"Preview" mode** — see what a peer would get at a different tier without committing
- **Per-peer capability overrides** — use groups only
- **Mobile-responsive layout** — desktop-first for now (insideOutside handles mobile)
- **Keyboard shortcuts** — navigate peers with arrow keys, etc.
- **WebSocket/SSE for real-time updates** — polling via react-query is sufficient

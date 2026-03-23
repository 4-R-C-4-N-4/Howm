# Access Control Implementation Spec

**Source BRD:** `docs/access/BRD-access-control.md`
**Author:** IV (BRD), implementation spec by agent
**Date:** 2026-03-23
**Status:** Draft — awaiting review

---

## 0. Current State Analysis

What exists today and what the BRD requires us to change.

### 0.1 Peer Management (peers.rs)

TODAY: Peers are stored in `peers.json` as a flat file. Each `Peer` has a
`TrustLevel` enum: `Friend | Public | Restricted`. This is a simple per-peer
label — not group-based, not capability-aware, and not stored in SQLite.

BRD REQUIRES: Replace with a group-based model in `access.db` (SQLite/WAL).
Peers are assigned to groups; groups carry capability rules. `TrustLevel`
becomes obsolete — replaced by group membership.

### 0.2 Trust Gate / Classification (p2pcd-types)

TODAY: `ClassificationTier` enum (`Public | Friends | Blocked`) lives in
p2pcd-types. `TrustPolicy` is per-capability and evaluates a peer against a
flat friends list + per-peer overrides at OFFER intersection time. The trust
gate runs inside `compute_intersection()` — if a peer is blocked for a
capability, it's excluded from the active set.

BRD REQUIRES: The trust gate concept remains, but the evaluation source
changes from TOML config + friends list to SQLite group membership. The
`ClassificationTier` enum needs to be replaced or extended with the new
three-tier model (default/friends/trusted). Critically, the BRD also
requires per-peer OFFERs (§5.3 FR-3.3) — today a single manifest is built
for all peers.

### 0.3 Session / Engine (engine.rs)

TODAY: `ProtocolEngine` builds ONE `local_manifest` at startup and reuses it
for all peers. Trust policies filter at intersection time. Rebroadcast exists
(`rebroadcast()` method) but is triggered by config changes, not group
membership changes.

BRD REQUIRES: Per-peer manifests (FR-3.3 — omit capabilities the peer can't
access). Rebroadcast on group membership change (FR-5.3). Session close on
deny (FR-5.2).

### 0.4 Capability Request Routing (proxy.rs)

TODAY: The proxy forwards requests to capability processes by matching on
capability name. It injects `X-Node-Id` and `X-Node-Name` headers but does
NOT inject the requesting peer's identity. Capabilities have no way to know
which remote peer is making the request through the proxy.

BRD REQUIRES: Capability processes must call `resolve_permission(peer_id,
capability_name)` before handling any request (FR-2.1). This requires the
requesting peer's identity to reach the capability process — either via
header injection at the proxy or via the capability SDK callbacks.

### 0.5 Storage

TODAY: No SQLite in the daemon. Peers in JSON, config in TOML. Social-feed
has its own SQLite DB.

BRD REQUIRES: `access.db` in `$DATA_DIR/access.db`, WAL mode, rusqlite
with bundled feature.

### 0.6 API Routes

TODAY: API routes exist for node management, capabilities, connections,
network, settings, p2pcd, and proxying. No `/access/*` routes.

BRD REQUIRES: Full `/access/*` CRUD API (§5.4).

---

## 1. Implementation Phases

### Phase 1: Foundation (access crate + DB + built-in groups)

  1.1  Create new workspace crate: `node/access/` (library crate)
       - Name: `howm-access` (or `access`)
       - Dependencies: rusqlite (bundled), uuid, serde, tracing
       - This is the shared library that FR-2.1 calls for

  1.2  Define Rust types:
       - `Group { group_id: Uuid, name: String, built_in: bool, capabilities: Vec<CapabilityRule>, created_at: u64, description: Option<String> }`
       - `CapabilityRule { capability_name: String, allow: bool, scope_overrides: Option<ScopeOverride> }`
       - `ScopeOverride { rate_limit: Option<u64>, ttl: Option<u64> }`
       - `PeerGroupMembership { peer_id: Vec<u8>, group_id: Uuid, assigned_at: u64, assigned_by: String }`
       - `PermissionResult { Allow { scope_overrides: Option<ScopeOverride> } | Deny }`

  1.3  Implement `AccessDb`:
       - `open(path)` — open/create access.db, WAL mode, run migrations
       - `init_built_in_groups()` — create howm.default/friends/trusted with
         well-known UUIDs if not present, with fixed capability rules
       - Schema: `groups`, `capability_rules`, `peer_group_memberships` tables

  1.4  Implement `resolve_permission(db, peer_id, capability_name) -> PermissionResult`:
       - Single SQL query joining memberships → groups → capability_rules
       - Apply "most permissive wins" logic
       - Implicit howm.default fallback for peers with no explicit membership
       - Target: < 5ms (SQLite WAL read, no network)

  1.5  Tests: unit tests for permission resolution, built-in group creation,
       most-permissive-wins semantics, implicit default fallback

### Phase 2: Daemon Integration

  2.1  Add `howm-access` dependency to daemon Cargo.toml

  2.2  Initialize AccessDb on daemon startup (main.rs):
       - Open `$DATA_DIR/access.db`
       - Call `init_built_in_groups()`
       - Store `Arc<AccessDb>` in `AppState`

  2.3  Migrate existing peer trust data:
       - Read `peers.json` TrustLevel values
       - Map: Friend → howm.friends, Public → howm.default,
         Restricted → howm.default
       - Create PeerGroupMembership records
       - One-time migration on startup if access.db is fresh

  2.4  Add `/access/*` HTTP API routes (access_routes.rs):
       - All endpoints from BRD §5.4
       - Local-only enforcement (existing `is_local_or_wg` check must be
         tightened — see QUESTION Q-1)
       - Wire into `build_router()` under authenticated routes

  2.5  Integrate with invite flow:
       - When a peer completes invite exchange, create explicit howm.default
         membership record (FR-5.1)
       - Hook into `open_invite.rs` and `invite.rs` completion paths

### Phase 3: Per-Peer Offer Filtering

  3.1  Replace single `local_manifest` with per-peer manifest generation:
       - New method: `build_manifest_for_peer(peer_id) -> DiscoveryManifest`
       - Resolve peer's effective permissions
       - Include only capabilities the peer is allowed to access
       - Always include `core.session.heartbeat.1` (FR-3.4)
       - Set `classification` field to highest-privilege built-in group UUID

  3.2  Modify `ProtocolEngine`:
       - `start_session()` calls `build_manifest_for_peer()` instead of
         using the shared `local_manifest`
       - `handle_inbound()` does the same for responder sessions

  3.3  Replace `TrustPolicy` usage:
       - Remove trust gate evaluation from `compute_intersection()`
       - Trust filtering now happens BEFORE the OFFER is sent (by omitting
         capabilities) rather than at intersection time
       - The `TrustPolicy` struct and `ClassificationTier` enum in
         p2pcd-types become obsolete for the new flow
       - Keep them temporarily for backward compat, mark deprecated

  3.4  Wire `classification` field in OFFER:
       - Use highest-privilege built-in group UUID as the `group_ref`
       - For peers in only custom groups, use string "custom" (per OQ-3
         current recommendation, though see QUESTION Q-3)

### Phase 4: Enforcement at Capability Handlers

  4.1  Add `howm-access` dependency to capability crates (social-feed, etc.)

  4.2  Inject requesting peer identity into proxied requests:
       - The proxy already injects `X-Node-Id` / `X-Node-Name` for the
         LOCAL node's identity
       - For P2P-CD routed requests: the daemon knows which peer the
         request comes from (via the P2P-CD session / bridge)
       - Inject `X-Peer-Id` header with the requesting peer's WG public key
       - See QUESTION Q-2 about the exact routing path

  4.3  Add access check middleware/guard to capability processes:
       - Before handling any remote peer request, call
         `resolve_permission(peer_id, capability_name)`
       - On Deny → return HTTP 403 with typed body per FR-2.3
       - This can be an axum middleware layer or a guard function
       - The capability needs read access to `access.db` (same file,
         WAL mode allows concurrent reads)

  4.4  Pass `$DATA_DIR` to capability processes:
       - Capabilities need to know where `access.db` lives
       - Add `HOWM_DATA_DIR` env var to capability launch config
       - Or pass as CLI arg (already have `--data-dir` patterns)

### Phase 5: Session Lifecycle Hooks

  5.1  Deny endpoint (FR-5.2):
       - `POST /access/peers/{peer_id}/deny`
       - Send P2P-CD CLOSE with reason_code 2 (AuthFailure)
       - Cache outcome as NONE in peer cache
       - Remove all group memberships for the peer

  5.2  Rebroadcast on membership change (FR-5.3):
       - When any PeerGroupMembership is created/deleted, trigger
         rebroadcast for the affected peer
       - Recompute personal_hash (now per-peer since manifests differ)
       - Increment sequence_num
       - Send new OFFER to the affected peer's active session
       - The active set remains operational during re-exchange (§8.4)

  5.3  Default group assignment on invite completion (FR-5.1):
       - Already covered in Phase 2.5 but listed here for lifecycle
         completeness

### Phase 6: UI Integration (out of scope for this spec)

  UI work (FR-6.1 through FR-6.5) is a frontend concern. The backend API
  from Phase 2.4 provides all needed data. UI spec should be separate.

---

## 2. Database Schema

```sql
-- access.db

CREATE TABLE groups (
    group_id    TEXT PRIMARY KEY,  -- UUID as text
    name        TEXT NOT NULL,
    built_in    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    description TEXT
);

CREATE TABLE capability_rules (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id        TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
    capability_name TEXT NOT NULL,
    allow           INTEGER NOT NULL DEFAULT 1,
    rate_limit      INTEGER,  -- NULL = no override
    ttl             INTEGER,  -- NULL = no override
    UNIQUE(group_id, capability_name)
);

CREATE TABLE peer_group_memberships (
    peer_id     BLOB NOT NULL,    -- 32-byte WG public key
    group_id    TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
    assigned_at INTEGER NOT NULL,
    assigned_by TEXT NOT NULL DEFAULT 'local',
    PRIMARY KEY (peer_id, group_id)
);

CREATE INDEX idx_pgm_peer ON peer_group_memberships(peer_id);
CREATE INDEX idx_pgm_group ON peer_group_memberships(group_id);
CREATE INDEX idx_cr_group ON capability_rules(group_id);
```

### Well-Known UUIDs

```
howm.default  = 00000000-0000-0000-0000-000000000001
howm.friends  = 00000000-0000-0000-0000-000000000002
howm.trusted  = 00000000-0000-0000-0000-000000000003
```

### Built-In Capability Rules

howm.default:
  - core.session.heartbeat.1  → allow
  - core.session.attest.1     → allow
  - core.session.latency.1    → allow
  - core.network.endpoint.1   → allow
  - core.session.timesync.1   → allow

howm.friends (adds to default):
  - howm.social.feed.1        → allow
  - howm.social.messaging.1   → allow
  - howm.social.files.1       → allow
  - howm.world.room.1         → allow
  - core.network.peerexchange.1 → allow

howm.trusted (adds to friends):
  - core.network.relay.1      → allow

---

## 3. Permission Resolution Query

Single-query approach for `resolve_permission()`:

```sql
SELECT cr.allow, cr.rate_limit, cr.ttl
FROM capability_rules cr
JOIN groups g ON cr.group_id = g.group_id
WHERE cr.capability_name = ?1
  AND (
    -- Explicit memberships
    g.group_id IN (
      SELECT group_id FROM peer_group_memberships WHERE peer_id = ?2
    )
    -- Implicit howm.default fallback
    OR g.group_id = '00000000-0000-0000-0000-000000000001'
  )
```

Then in Rust: if any row has `allow = 1`, result is Allow (with merged
scope_overrides using most-permissive values). Otherwise Deny.

---

## 4. Crate Dependency Graph (after implementation)

```
p2pcd-types  (no change)
     |
  p2pcd      (no change to library, deprecate TrustPolicy usage)
     |
howm-access  (NEW — rusqlite, uuid, serde)
   /    \
daemon   social-feed (and future capabilities)
```

howm-access has no dependency on p2pcd or p2pcd-types. It's a pure
application-layer library. It takes `peer_id: &[u8]` and
`capability_name: &str` — no protocol types.

---

## 5. Migration Path from TrustPolicy

The existing `TrustPolicy` / `ClassificationTier` / `ClassificationConfig`
system in p2pcd-types/config.rs needs a deprecation path:

1. Phase 1-2: Both systems coexist. AccessDb is authoritative for the new
   group model. TrustPolicy still compiled but unused at runtime once
   Phase 3 lands.

2. Phase 3: Engine stops using `trust_policies` for offer filtering.
   Replace with AccessDb-driven per-peer manifest generation.

3. Cleanup: Remove `TrustPolicy`, `ClassificationTier`,
   `ClassificationConfig`, `FriendsConfig` from p2pcd-types and config.
   Remove `[capabilities.*.classification]` and `[friends]` from TOML
   config schema. Bump config version.

The `classification` field in `CapabilityDeclaration` (key 4 in CBOR) is
currently "omitted from wire per spec §5.3". The BRD wants it populated
with the group UUID. This is a p2pcd-types change: add it to the wire
encoding when present.

---

## 6. Questions and Flags

### Q-1: Local-Only Enforcement for /access/* API

The BRD says the access API "MUST NOT be proxied through any path accessible
over the WireGuard tunnel" (NFR-4). However, the current API architecture
uses `is_local_or_wg()` which ALLOWS requests from the WG subnet
(100.222.0.0/16). For the access API, we need LOCALHOST ONLY — not WG peers.

QUESTION: Should `/access/*` routes use a stricter `is_localhost_only()`
guard that rejects WG-subnet IPs? This is a deviation from the rest of the
authenticated API which allows WG access. The BRD is explicit that remote
peers must not reach this API.

RECOMMENDATION: Yes. Add `is_localhost_only()` that only allows
127.0.0.0/8 and ::1. Apply to `/access/*` routes only.


### Q-2: Peer Identity in Proxied Capability Requests

The proxy (proxy.rs) forwards requests to capability processes. For LOCAL
API calls (operator using the UI), there is no peer_id — the operator is
not a peer. For P2P-CD routed requests (remote peer → daemon bridge →
capability), the daemon knows the peer_id from the session.

QUESTION: How do remote peer requests currently reach capability processes?
Looking at the code, it appears the P2P-CD bridge notifies capabilities
of peer-active/peer-inactive via HTTP callbacks, and inbound P2P-CD
messages are delivered as HTTP posts. The peer_id is already in the
`InboundMessage` payload.

But for direct HTTP requests from peers over the WG tunnel (not via P2P-CD
message routing), the capability process receives a request from a WG IP.
The daemon could resolve WG IP → peer_id from its peer list and inject
`X-Peer-Id`.

RECOMMENDATION: Two enforcement points:
  (a) P2P-CD message path: peer_id is already known from the session.
      Resolve permission in the daemon bridge before forwarding.
  (b) Direct HTTP over WG: proxy resolves WG source IP → peer_id,
      injects `X-Peer-Id`, capability checks permission.

Clarification needed from IV on whether (b) is a real path or if all
remote capability access goes through P2P-CD.


### Q-3: classification Field for Custom-Group-Only Peers (OQ-3)

The BRD leaves this open. Using the string "custom" leaks that the peer
is NOT in any built-in tier. Using the custom group's UUID is more opaque
but means the peer sees a different UUID than built-in-tier peers.

RECOMMENDATION: Use the custom group's UUID. It's maximally opaque — the
peer can't distinguish it from a built-in UUID without prior knowledge.
"custom" is a string that conveys semantic meaning; a UUID conveys nothing.


### Q-4: Capability Declaration Classification Field on Wire

The p2pcd-types CBOR encoding currently OMITS the classification field
(key 4) from the wire: "classification is intentionally omitted from the
wire (spec §5.3 note)". The BRD wants it populated with the group UUID
as an advisory signal (FR-3.1, FR-3.2).

QUESTION: Should we start encoding classification on the wire? This is a
wire format change that could affect interop with other P2P-CD
implementations (if any exist). The BRD says it's "advisory and
informational" but the spec note says it's omitted.

RECOMMENDATION: Add it as an optional field. Encode when present, skip
when absent. Receiving side ignores it for enforcement but may log it.
This preserves backward compat — old peers ignore unknown fields.


### Q-5: Per-Peer Manifests and Personal Hash

Today: one manifest, one personal_hash, shared across all peers.
BRD requires: per-peer manifests (different capability sets per peer).

This means personal_hash becomes per-peer too. When a peer's group
membership changes, only THAT peer's manifest changes — other peers'
sessions are unaffected.

QUESTION: Does the P2P-CD spec allow per-peer manifests? The spec says
"A peer's discovery_manifest represents its current capability profile"
(singular). If our manifest varies by who we're talking to, we have
multiple personal_hashes. The peer cache (keyed by personal_hash) would
need adjustment.

RECOMMENDATION: This is fine. The spec says the manifest represents what
we OFFER to a specific peer. The personal_hash is a hash of what we
sent to THAT peer. The cache already keys by (peer_id, personal_hash).
Per-peer manifests just mean different peers see different hashes, which
is the intended behavior — when we change a peer's group, their hash
changes, invalidating their cache entry and triggering re-exchange.


### Q-6: Scope Override Interaction with P2P-CD Negotiation

BRD §4.2 says scope_overrides "take effect in the CONFIRM accepted_params
field ... within the bounds already negotiated by both peers' OFFERs
(most-restrictive-wins per §7.3)".

QUESTION: How do group-level scope_overrides interact with the existing
per-capability scope config in p2pcd-peer.toml? The TOML scope values
go into the OFFER. The group scope_overrides would need to be applied
at CONFIRM time — but the CONFIRM is computed by `compute_intersection()`
which only has access to both OFFERs.

RECOMMENDATION: Phase 1 should NOT implement scope_overrides. The BRD
marks them as optional (`ScopeOverride?`). Focus on allow/deny first.
Scope overrides can be Phase 2 when the interaction model is clearer.


### Q-7: peers.json Coexistence

After migration, should `peers.json` still be the source of truth for
peer metadata (name, WG key, endpoint, etc.)? The access system only
manages group membership — it doesn't replace the peer registry.

RECOMMENDATION: Yes, keep peers.json for peer metadata. access.db only
stores group membership. The two are linked by peer_id (WG public key).
Long-term, peers.json should probably move to SQLite too, but that's
a separate effort.

---

## 7. Estimated Effort

  Phase 1 (access crate + DB + types + tests)     ~2-3 days
  Phase 2 (daemon integration + API + migration)   ~3-4 days
  Phase 3 (per-peer offers + engine changes)        ~3-4 days
  Phase 4 (capability enforcement)                  ~2-3 days
  Phase 5 (session lifecycle hooks)                 ~2-3 days

  Total: ~12-17 days of implementation work

  Phase 6 (UI) is out of scope for this spec.

---

## 8. Testing Strategy

- Unit tests in howm-access crate:
  - Permission resolution (all BRD §4.4 cases)
  - Most-permissive-wins semantics
  - Implicit default fallback
  - Built-in group immutability
  - Custom group CRUD

- Integration tests in daemon:
  - API endpoint tests for all /access/* routes
  - Invite → default group assignment
  - Deny → session close
  - Group change → rebroadcast trigger
  - Per-peer manifest generation correctness

- End-to-end:
  - Two-node test: peer in howm.default cannot access social feed
  - Promote to howm.friends → social feed becomes accessible
  - Deny → session closes, reconnection blocked

---

## 9. Files to Create/Modify

### New Files
  node/access/Cargo.toml          — new crate manifest
  node/access/src/lib.rs          — AccessDb, types, resolve_permission
  node/access/src/schema.rs       — SQL schema + migrations
  node/access/src/types.rs        — Group, CapabilityRule, etc.
  node/daemon/src/api/access_routes.rs  — /access/* HTTP handlers

### Modified Files
  Cargo.toml (workspace)          — add node/access member
  node/daemon/Cargo.toml          — add howm-access + rusqlite deps
  node/daemon/src/main.rs         — init AccessDb on startup
  node/daemon/src/state.rs        — add Arc<AccessDb> to AppState
  node/daemon/src/api/mod.rs      — wire access_routes
  node/daemon/src/p2pcd/engine.rs — per-peer manifest, rebroadcast hooks
  node/daemon/src/proxy.rs        — inject X-Peer-Id header
  node/daemon/src/invite.rs       — default group on invite complete
  node/daemon/src/open_invite.rs  — default group on open invite complete
  node/p2pcd-types/src/lib.rs     — deprecate TrustPolicy (later phases)
  node/p2pcd-types/src/config.rs  — deprecate ClassificationConfig (later)
  node/p2pcd/src/session.rs       — classification field encoding (if Q-4)
  capabilities/social-feed/Cargo.toml — add howm-access dep
  capabilities/social-feed/src/main.rs — init AccessDb read handle
  capabilities/social-feed/src/api.rs  — add permission check middleware

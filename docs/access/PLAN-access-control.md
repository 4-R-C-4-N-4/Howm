# Access Control — Implementation Plan

**Branch:** `access-control` (current)
**Source:** BRD-access-control.md + SPEC-access-control-implementation.md
**Date:** 2026-03-23

---

## Phase 1: howm-access Crate (Foundation)

Creates the shared library that everything else depends on. No daemon changes yet — pure library + tests.

### 1.1 Create crate scaffold

- [ ] Create `node/access/Cargo.toml`
  - Name: `howm-access`
  - Deps: `rusqlite` (features: bundled), `uuid` (features: v4, serde), `serde` (features: derive), `tracing`
- [ ] Add `"access"` to workspace members in `node/Cargo.toml`
- [ ] Create `node/access/src/lib.rs` — re-exports, `AccessDb` struct

### 1.2 Define types (`node/access/src/types.rs`)

```rust
pub struct Group {
    pub group_id: Uuid,
    pub name: String,
    pub built_in: bool,
    pub capabilities: Vec<CapabilityRule>,
    pub created_at: u64,
    pub description: Option<String>,
}

pub struct CapabilityRule {
    pub capability_name: String,
    pub allow: bool,
    pub rate_limit: Option<u64>,  // phase 2+
    pub ttl: Option<u64>,        // phase 2+
}

pub struct PeerGroupMembership {
    pub peer_id: Vec<u8>,       // 32-byte WG pubkey
    pub group_id: Uuid,
    pub assigned_at: u64,
    pub assigned_by: String,
}

pub enum PermissionResult {
    Allow { scope_overrides: Option<ScopeOverride> },
    Deny,
}
```

### 1.3 Implement schema (`node/access/src/schema.rs`)

- [ ] `create_tables(conn)` — groups, capability_rules, peer_group_memberships tables + indexes
- [ ] WAL mode pragma on open
- [ ] Foreign keys pragma on

### 1.4 Implement AccessDb (`node/access/src/db.rs`)

```
AccessDb {
    conn: rusqlite::Connection   // single-threaded for writes
}
```

Core methods:

- [ ] `open(path: &Path) -> Result<Self>` — open/create, WAL, run schema
- [ ] `init_built_in_groups()` — upsert howm.default/friends/trusted with well-known UUIDs + fixed capability rules
- [ ] `resolve_permission(peer_id: &[u8], capability_name: &str) -> PermissionResult` — the hot-path query (see SPEC §3)
- [ ] `resolve_all_permissions(peer_id: &[u8], capabilities: &[&str]) -> HashMap<String, PermissionResult>` — batch version for trust gate (one query, loop in Rust)

Group CRUD:

- [ ] `list_groups() -> Vec<Group>`
- [ ] `get_group(group_id: Uuid) -> Option<Group>`
- [ ] `create_group(name, description, capability_rules) -> Group`
- [ ] `update_group(group_id, name?, description?, capability_rules?) -> Result<Group>`
- [ ] `delete_group(group_id) -> Result<()>` — error if built_in

Membership CRUD:

- [ ] `list_peer_groups(peer_id: &[u8]) -> Vec<Group>`
- [ ] `assign_peer_to_group(peer_id: &[u8], group_id: Uuid) -> Result<PeerGroupMembership>`
- [ ] `remove_peer_from_group(peer_id: &[u8], group_id: Uuid) -> Result<()>`
- [ ] `get_peer_effective_permissions(peer_id: &[u8]) -> HashMap<String, PermissionResult>` — resolves across all groups

### 1.5 Well-known UUIDs (constants in `types.rs`)

```rust
pub const GROUP_DEFAULT: Uuid = uuid!("00000000-0000-0000-0000-000000000001");
pub const GROUP_FRIENDS: Uuid = uuid!("00000000-0000-0000-0000-000000000002");
pub const GROUP_TRUSTED: Uuid = uuid!("00000000-0000-0000-0000-000000000003");
```

### 1.6 Tests (`node/access/src/lib.rs` or `tests/`)

- [ ] Built-in groups created on init, not deletable
- [ ] Peer with no membership → implicit howm.default → heartbeat/attest/latency/endpoint/timesync allowed, social denied
- [ ] Peer in howm.friends → social caps allowed
- [ ] Peer in howm.trusted → relay allowed
- [ ] Most-permissive-wins: peer in howm.default + custom group granting files → files allowed
- [ ] Custom group CRUD
- [ ] resolve_permission < 5ms (assert timing in test)
- [ ] Peer removed from group → permission changes immediately (no caching)

### Deliverable
`cargo test -p howm-access` passes. Crate compiles standalone. No daemon changes.

---

## Phase 2: Daemon Integration (DB init + API + migration)

Wires howm-access into the daemon. Trust gate NOT swapped yet — both systems coexist.

### 2.1 Daemon dependency + state

- [ ] Add `howm-access` dep to `node/daemon/Cargo.toml`
- [ ] Add `pub access_db: Arc<howm_access::AccessDb>` to `AppState` in `state.rs`
- [ ] In `main.rs` startup: open `$DATA_DIR/access.db`, call `init_built_in_groups()`, wrap in Arc, pass to AppState

### 2.2 Peer trust migration

- [ ] On first startup (access.db just created / empty memberships table):
  - Read all peers from `peers.json`
  - Map `TrustLevel::Friend` → assign to `howm.friends`
  - Map `TrustLevel::Public` → assign to `howm.default`
  - Map `TrustLevel::Restricted` → assign to `howm.default`
- [ ] Log migration: "Migrated N peers from peers.json trust levels to access.db groups"
- [ ] This is one-time — skip if memberships table already has rows

### 2.3 Access API routes (`node/daemon/src/api/access_routes.rs`)

All routes localhost-only (127.0.0.1 / ::1). NOT accessible over WG tunnel.

| Method | Path | Handler |
|--------|------|---------|
| GET | `/access/groups` | `list_groups` |
| POST | `/access/groups` | `create_group` — body: `{ name, description?, capabilities[] }` |
| GET | `/access/groups/:group_id` | `get_group` |
| PATCH | `/access/groups/:group_id` | `update_group` — 403 if built_in rules modified |
| DELETE | `/access/groups/:group_id` | `delete_group` — 403 if built_in |
| GET | `/access/peers/:peer_id/groups` | `list_peer_groups` |
| POST | `/access/peers/:peer_id/groups` | `assign_peer` — body: `{ group_id }` |
| DELETE | `/access/peers/:peer_id/groups/:group_id` | `remove_peer_from_group` |
| GET | `/access/peers/:peer_id/permissions` | `get_effective_permissions` |
| POST | `/access/peers/:peer_id/deny` | `deny_peer` — stub (lifecycle hook in Phase 5) |

- [ ] Add `is_localhost_only()` guard — only 127.0.0.0/8 and ::1, reject WG subnet
- [ ] Wire into `build_router()` in `api/mod.rs`
- [ ] peer_id in URL: hex-encoded 32-byte WG public key

### 2.4 Invite flow integration

- [ ] In `invite.rs` — after successful invite exchange, call `access_db.assign_peer_to_group(peer_id, GROUP_DEFAULT)`
- [ ] In `open_invite.rs` — same for open invite completion
- [ ] Only create membership if peer has no existing memberships (don't downgrade a manually-promoted peer)

### 2.5 Tests

- [ ] API integration tests: CRUD for groups, memberships, permissions endpoint
- [ ] Migration test: mock peers.json with various trust levels, verify correct group assignments
- [ ] Localhost-only enforcement: request from non-localhost → 403
- [ ] Invite completion → default group assigned

### Deliverable
`cargo test -p daemon` passes. `/access/*` API functional. Trust gate still uses old TrustPolicy — no behavioral change to existing sessions.

---

## Phase 3: Trust Gate Data Source Swap

The core change — compute_intersection() switches from TrustPolicy to AccessDb.

### 3.1 Modify compute_intersection signature

Current:
```rust
pub fn compute_intersection(
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
    trust_policies: &HashMap<String, TrustPolicy>,
) -> (Vec<ActiveCapability>, ...)
```

New — add a trust gate callback:
```rust
pub fn compute_intersection(
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
    trust_gate: &dyn Fn(&str, &[u8]) -> bool,  // (cap_name, peer_id) -> allowed
) -> (Vec<ActiveCapability>, ...)
```

This is cleaner than passing AccessDb directly into p2pcd-types — keeps the library protocol-layer-pure. The daemon provides the closure that calls `resolve_permission()`.

- [ ] Update `compute_intersection()` in `p2pcd-types/src/lib.rs`
- [ ] In the intersection loop: for each candidate capability, call `trust_gate(cap_name, remote_peer_id)` — exclude if false
- [ ] Always pass `core.session.heartbeat.1` regardless of trust gate result (FR-3.4)
- [ ] Update all call sites in `session.rs` to use new signature

### 3.2 Wire AccessDb into ProtocolEngine

- [ ] In `engine.rs`: store `Arc<howm_access::AccessDb>` (or a read handle)
- [ ] In session creation / CONFIRM handling: build the trust gate closure:
  ```rust
  let db = self.access_db.clone();
  let trust_gate = move |cap_name: &str, peer_id: &[u8]| -> bool {
      matches!(db.resolve_permission(peer_id, cap_name), PermissionResult::Allow { .. })
  };
  ```
- [ ] Pass to `compute_intersection()`
- [ ] Remove `trust_policies: RwLock<HashMap<String, TrustPolicy>>` field from ProtocolEngine

### 3.3 Deprecate old trust types

- [ ] Mark `TrustPolicy`, `ClassificationTier`, `ClassificationConfig` as `#[deprecated]` in p2pcd-types
- [ ] Remove `[friends]` TOML config parsing from daemon config (or keep parsing but ignore)
- [ ] Keep types compiled — don't delete yet (one version deprecation cycle)

### 3.4 Wire classification field in OFFER

- [ ] In `CapabilityDeclaration` CBOR encoding: encode key 4 (classification) when `group_ref` is set
- [ ] In manifest construction: look up peer's highest-privilege built-in group UUID (or custom group UUID for custom-only peers)
- [ ] Set as classification field value
- [ ] Receiving side: decode but don't enforce (advisory only per FR-3.2)

### 3.5 Update existing tests

- [ ] All `compute_intersection` tests in p2pcd-types → update to use closure-based API
- [ ] Add new tests: same manifest, different peer → different intersection results based on group membership
- [ ] Test heartbeat always passes trust gate

### Deliverable
Trust gate reads from access.db. A peer in howm.default gets heartbeat/attest/latency/endpoint/timesync in active set. A peer in howm.friends gets social caps too. Old TrustPolicy code deprecated but still compiles. `cargo test --workspace` passes.

---

## Phase 4: Capability Handler Enforcement (Defense in Depth)

Second enforcement layer — each capability handler independently checks permissions.

### 4.1 Peer identity in proxied requests

- [ ] In `proxy.rs`: for P2P-CD routed requests, inject `X-Peer-Id` header with hex-encoded WG public key
- [ ] The daemon knows the peer from the P2P-CD session/bridge — trace the request origin through `bridge.rs`
- [ ] For local API requests (operator via UI): no `X-Peer-Id` header → skip access check (operator is god)
- [ ] For direct HTTP over WG: resolve WG source IP → peer_id from peers list, inject header

### 4.2 Access check middleware for social-feed

- [ ] Add `howm-access` dep to `capabilities/social-feed/Cargo.toml`
- [ ] In `main.rs`: open `access.db` in read-only mode (WAL allows concurrent reads)
  - Path from `HOWM_DATA_DIR` env var
- [ ] Create axum middleware layer:
  ```rust
  async fn access_guard(req: Request, next: Next) -> Response {
      let peer_id = req.headers().get("X-Peer-Id");
      if let Some(peer_id) = peer_id {
          let peer_bytes = hex::decode(peer_id)?;
          match access_db.resolve_permission(&peer_bytes, CAPABILITY_NAME) {
              PermissionResult::Deny => return (StatusCode::FORBIDDEN, json!({"error": "access_denied", ...})),
              PermissionResult::Allow { .. } => {}
          }
      }
      // No X-Peer-Id = local request = allow
      next.run(req).await
  }
  ```
- [ ] Apply to all routes in social-feed

### 4.3 Pass data dir to capability processes

- [ ] In `executor.rs` (capability launcher): set `HOWM_DATA_DIR` env var when spawning capability processes
- [ ] Value: same `$DATA_DIR` used by daemon

### 4.4 Template for future capabilities

- [ ] Document the pattern so messaging, files, world, relay, peerexchange capabilities can add the same middleware
- [ ] Consider extracting into a shared `howm-capability-sdk` crate later (not now — only social-feed exists)

### 4.5 Tests

- [ ] Social-feed with X-Peer-Id of a howm.default peer → 403
- [ ] Social-feed with X-Peer-Id of a howm.friends peer → 200
- [ ] Social-feed with no X-Peer-Id (local request) → 200
- [ ] 403 body matches FR-2.3 spec: `{ error: "access_denied", capability, peer_id }` — no group info leaked

### Deliverable
Social-feed independently enforces access. Both layers (trust gate + handler) active. Denied capability requests get caught at both layers.

---

## Phase 5: Session Lifecycle Hooks

Dynamic behavior — deny, rebroadcast, group change propagation.

### 5.1 Deny endpoint (`POST /access/peers/:peer_id/deny`)

- [ ] Remove peer from ALL groups
- [ ] If peer has active P2P-CD session:
  - Send CLOSE with `reason_code: 2` (AuthFailure) via ProtocolEngine
  - Cache outcome as NONE in peer cache (auto-deny on reconnect)
- [ ] Return 200 with confirmation

### 5.2 Rebroadcast on group membership change

- [ ] When `assign_peer_to_group()` or `remove_peer_from_group()` is called via API:
  - Increment `sequence_num` on the engine
  - Trigger `rebroadcast()` to the affected peer
  - The re-exchange will call `compute_intersection()` with the new group membership
  - Active set updates: peer gains/loses capabilities accordingly
- [ ] Active set remains operational during re-exchange (P2P-CD §8.4)
- [ ] personal_hash does NOT change (single manifest, same for all peers)
- [ ] Only sequence_num increments to signal "re-exchange me"

### 5.3 Wire lifecycle hooks to API

- [ ] `POST /access/peers/:peer_id/groups` (assign) → triggers rebroadcast
- [ ] `DELETE /access/peers/:peer_id/groups/:group_id` (remove) → triggers rebroadcast
- [ ] `POST /access/peers/:peer_id/deny` → triggers close + cache

### 5.4 Tests

- [ ] Deny peer → session closed, reason_code 2, reconnect blocked
- [ ] Promote peer (default → friends) → rebroadcast, new intersection includes social caps
- [ ] Demote peer (friends → default) → rebroadcast, social caps removed from active set
- [ ] Rapid group changes → no panics, last state wins

### Deliverable
Full lifecycle working. Operator can promote/demote/deny peers in real-time. Sessions react immediately.

---

## Phase 6: UI Integration (out of scope)

Listed for completeness — separate spec. Backend API from Phase 2 provides everything the UI needs.

- FR-6.1: Show peer group assignments
- FR-6.2: Drag/assign peers between groups
- FR-6.3: Effective permission summary per peer
- FR-6.4: Visual distinction for built-in vs custom groups
- FR-6.5: Warning before assigning to howm.trusted

---

## Execution Order & Dependencies

```
Phase 1 ──→ Phase 2 ──→ Phase 3 ──→ Phase 5
                 │              │
                 │              └──→ Phase 4
                 │
                 └── (Phase 3 and 4 can run in parallel after Phase 2)
```

Phase 1 is the foundation — everything depends on it.
Phase 2 needs Phase 1 (imports howm-access).
Phase 3 needs Phase 2 (AccessDb in AppState).
Phase 4 needs Phase 2 (HOWM_DATA_DIR, X-Peer-Id).
Phase 5 needs Phase 3 (rebroadcast uses new trust gate).
Phases 3 and 4 are independent of each other after Phase 2.

---

## Estimated Effort

| Phase | Work | Estimate |
|-------|------|----------|
| 1 | howm-access crate, types, DB, resolve_permission, tests | 2-3 sessions |
| 2 | Daemon init, migration, /access/* API, invite hooks | 3-4 sessions |
| 3 | Trust gate swap, compute_intersection refactor, deprecations | 2-3 sessions |
| 4 | X-Peer-Id injection, social-feed middleware | 2 sessions |
| 5 | Deny, rebroadcast hooks, lifecycle integration | 2-3 sessions |
| **Total** | | **11-15 sessions** |

---

## Risk Notes

1. **compute_intersection signature change** — Phase 3 touches p2pcd-types public API. All existing tests must be updated. The closure-based approach avoids adding howm-access as a dep to p2pcd-types (keeps protocol layer pure).

2. **Concurrent SQLite access** — WAL mode handles multiple readers + single writer. The daemon writes; capability processes only read. If a capability process opens access.db read-only, no WAL checkpoint contention.

3. **Rebroadcast storms** — If an operator bulk-changes group membership for many peers, each change triggers a rebroadcast. Consider batching (debounce 100ms) in Phase 5 if this becomes a problem.

4. **Migration safety** — Phase 2.2 migration is one-time and additive. Old peers.json is not modified. If migration fails, access.db can be deleted and re-created.

5. **personal_hash stability** — This is the big win of Option C. Group changes do NOT change the personal_hash. Only adding/removing capabilities from the node changes it. This means group changes are cheap — just re-exchange with affected peer, no cache invalidation across all peers.

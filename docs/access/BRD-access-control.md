# Access Control — Peer Groups and Classification Tiers

**Author:** Ivy Darling
**Project:** Howm
**Status:** Draft
**Version:** 0.1
**Date:** 2026-03-23
**Capability path:** `capabilities/access/` (or integrated into daemon — see OQ-1)
**P2P-CD name:** N/A — this BRD defines application-layer policy; it does not introduce a new P2P-CD capability

---

## 1. Background

The P2P-CD spec (§6) defines two built-in classification tiers — UNRESTRICTED and DENIED — and explicitly leaves everything between as implementation-defined. It provides the `classification` field in capability declarations for carrying a `group_ref` or credential, and the trust gate mechanism for per-capability access decisions. However, the spec makes clear that the receiving peer evaluates the sender's classification against local policy — not the other way around.

Howm's access control requirement is the inverse: an operator wants to control what their own capabilities expose to a given peer, based on operator-defined group membership. This is an application-layer policy problem. The P2P-CD machinery (classification field, trust gate) is useful as a signalling channel but is not the enforcement point. Enforcement happens in the HTTP handlers of each capability process — a request either gets a response or it doesn't, based on the requesting peer's group membership as evaluated locally by the operator's node.

This BRD defines:
- A fixed set of three named built-in tiers that sit between DENIED and UNRESTRICTED
- An operator-controlled group system that assigns peers to one or more groups
- The capability access rules each tier confers
- How group membership is stored, evaluated, and surfaced to peers
- How the `group_ref` field in P2P-CD OFFERs is used as an advisory signal

---

## 2. Design Principles

**Operator sovereignty.** Group membership is decided entirely by the local operator. No peer can influence their own group assignment. No mutual agreement is required.

**Most permissive wins.** A peer in multiple groups receives the union of all permissions granted across those groups. A permission granted by any group is granted.

**Groups are local; access level is the only thing a peer can infer.** Group names, group membership lists, and group UUIDs are never transmitted to peers. The only information a peer can derive is what they can and cannot access — by observing which capability responses they receive.

**The `group_ref` carries a group UUID, not a name.** When Howm writes into the P2P-CD `classification` field, it uses the group's stable UUID. Group names are local. A peer that sees their `group_ref` value knows only an opaque identifier — not the group's name, not who else is in it.

**Enforcement is at the capability handler, not the protocol layer.** The P2P-CD session may be ACTIVE, but a capability process that receives a request from a peer outside its permitted groups returns a `403 Forbidden` or a `capability_unavailable` envelope. The session is not closed.

**The built-in tiers are defaults, not ceilings.** An operator can create custom groups with any subset of capability access. Custom groups coexist with built-in tiers; a peer can be in both a built-in tier group and a custom group simultaneously.

---

## 3. Tier Stack

Four tiers are defined. Three are Howm built-ins; two are P2P-CD built-ins.

```
┌─────────────────────────────────────────────────────────────┐
│  UNRESTRICTED  (P2P-CD built-in, §6.1)                      │
│  Reserved. Not assigned by normal group logic.              │
├─────────────────────────────────────────────────────────────┤
│  howm.trusted                                               │
│  Full application access including relay.                   │
├─────────────────────────────────────────────────────────────┤
│  howm.friends                                               │
│  Social capabilities + room access + peer exchange.         │
├─────────────────────────────────────────────────────────────┤
│  howm.default  ◄── every invited peer starts here           │
│  Session health + endpoint reflection only.                 │
├─────────────────────────────────────────────────────────────┤
│  DENIED  (P2P-CD built-in, §6.1)                            │
│  No session. No capability exchange.                        │
└─────────────────────────────────────────────────────────────┘
```

### 3.1 `howm.default`

Every peer that has completed a successful invite exchange lands in `howm.default` unless the operator explicitly assigns them elsewhere. This tier establishes a minimum viable connected peer — enough to maintain the session and verify identity, nothing more.

| Capability | Access |
|------------|--------|
| `core.session.heartbeat.1` | ✓ Always — required by P2P-CD spec (§D.1, Mandatory) |
| `core.session.attest.1` | ✓ Build identity exchange |
| `core.session.latency.1` | ✓ Latency measurement |
| `core.network.endpoint.1` | ✓ Endpoint reflection |
| `core.session.timesync.1` | ✓ Clock offset estimation |
| `howm.social.feed.1` | ✗ |
| `howm.social.messaging.1` | ✗ |
| `howm.social.files.1` | ✗ |
| `howm.world.room.1` | ✗ |
| `core.network.peerexchange.1` | ✗ |
| `core.network.relay.1` | ✗ |

### 3.2 `howm.friends`

The primary social tier. Grants access to all social capabilities and room. Peer exchange is included — a `friends`-tier peer can discover other peers through gossip.

Includes everything in `howm.default`, plus:

| Capability | Access |
|------------|--------|
| `howm.social.feed.1` | ✓ |
| `howm.social.messaging.1` | ✓ |
| `howm.social.files.1` | ✓ (subject to per-offering access policy in BRD-003) |
| `howm.world.room.1` | ✓ (subject to room access policy in BRD-004) |
| `core.network.peerexchange.1` | ✓ |

### 3.3 `howm.trusted`

Full access. Includes everything in `howm.friends`, plus:

| Capability | Access |
|------------|--------|
| `core.network.relay.1` | ✓ This peer may use your node as a relay |

### 3.4 DENIED (P2P-CD built-in)

Explicitly blocked peers. The P2P-CD auto-deny mechanism (§9.2) applies — if a peer's current hash matches a cached NONE outcome, no session is initiated. An operator may also explicitly move a peer to DENIED post-session, which closes the session and caches the outcome.

---

## 4. Group Model

Built-in tiers are implemented as groups. The operator may also create **custom groups** with any capability access configuration. All groups — built-in and custom — share the same data model.

### 4.1 Group Record

```
Group {
    group_id        : uuid          -- stable, never changes, used as group_ref in P2P-CD
    name            : tstr          -- operator-visible label, local only, max 64 bytes
    built_in        : bool          -- true for howm.default, howm.friends, howm.trusted
    capabilities    : [CapabilityRule]
    created_at      : uint          -- Unix epoch seconds
    description     : tstr?         -- optional operator note, max 255 bytes
}
```

Built-in groups are created at first run and cannot be deleted. Their `group_id` values are stable across all Howm nodes (well-known UUIDs defined by this spec — see §7). Their capability rules can be inspected but not modified in phase 1. Custom groups can be created, renamed, and deleted freely.

### 4.2 Capability Rule

```
CapabilityRule {
    capability_name : tstr          -- fully-qualified P2P-CD name, e.g. "howm.social.feed.1"
    allow           : bool          -- grant or explicitly deny this capability
    scope_overrides : ScopeOverride?
}

ScopeOverride {
    rate_limit      : uint?         -- requests/sec, 0 = unlimited; overrides manifest default
    ttl             : uint?         -- seconds, 0 = no expiry; overrides manifest default
}
```

`scope_overrides` is optional. When present, its values take effect in the P2P-CD CONFIRM `accepted_params` field for that capability, within the bounds already negotiated by both peers' OFFERs (most-restrictive-wins per §7.3 of the spec still applies — a group cannot grant more than the manifest offered).

### 4.3 Peer Group Membership

```
PeerGroupMembership {
    peer_id         : bstr          -- Curve25519 public key
    group_id        : uuid
    assigned_at     : uint          -- Unix epoch seconds
    assigned_by     : tstr          -- always "local" in phase 1
}
```

A peer may have zero or more `PeerGroupMembership` records. A peer with zero memberships is implicitly in `howm.default` — the default group is the fallback, not an explicit assignment required for every peer.

### 4.4 Permission Resolution

Given a peer_id and a capability name, the resolved permission is computed as:

1. Collect all groups the peer is explicitly a member of.
2. Add `howm.default` (always applies, even if not explicitly assigned).
3. For each group, find the `CapabilityRule` for the requested capability, if any.
4. If any group has `allow: true` for that capability → **ALLOW**.
5. If no group has any rule for that capability → **DENY** (default-deny).
6. If all rules found are `allow: false` → **DENY**.

Most permissive wins. An explicit `allow: true` in any group overrides `allow: false` in any other group. This means there is no "blacklist veto" — if you want to block a peer from something, do not grant them access in any group. To revoke access, remove the peer from all groups that grant that capability.

---

## 5. Functional Requirements

### 5.1 Storage

- **FR-1.1** Groups, capability rules, and peer memberships SHALL be persisted to a SQLite database (`rusqlite`, `bundled`) at `$DATA_DIR/access.db`, shared across all capabilities on the node.
- **FR-1.2** The three built-in groups (`howm.default`, `howm.friends`, `howm.trusted`) SHALL be created with their well-known UUIDs (§7) on first run if they do not already exist.
- **FR-1.3** Built-in groups SHALL NOT be deletable. Their names and descriptions are editable; their capability rules are read-only in phase 1.
- **FR-1.4** The database SHALL be readable by all capability processes on the node. Write access is restricted to the daemon and the access management API.

### 5.2 Permission Evaluation API

All capability processes evaluate incoming requests against the access system via a shared library call or a local IPC endpoint. This is not an HTTP call to an external service — it is synchronous and in-process or via Unix socket.

- **FR-2.1** Every capability process that enforces access control SHALL call `resolve_permission(peer_id: bstr, capability_name: tstr) -> PermissionResult` before handling any request from a remote peer. This function SHALL be provided by a shared Rust library linked directly into each capability process — not an IPC call, not an HTTP request. The library reads from `access.db` locally. The specific library name and crate structure are an implementation decision.
- **FR-2.2** `PermissionResult` SHALL be one of: `Allow { scope_overrides: ScopeOverride? }` or `Deny`.
- **FR-2.3** On `Deny`, the capability process SHALL return HTTP 403 with a typed body `{ error: "access_denied", capability: tstr, peer_id: bstr }`. The response MUST NOT reveal which group the peer is in or which groups exist.
- **FR-2.4** Permission resolution SHALL complete in < 5ms. It reads from a local SQLite database; no network calls are made.
- **FR-2.5** Permission results SHALL NOT be cached across requests in phase 1. Group membership may change at any time; each request is evaluated fresh.

### 5.3 P2P-CD Integration

- **FR-3.1** When constructing an OFFER manifest for a session with a specific peer, the daemon SHALL resolve that peer's effective group (the highest-privilege built-in group they are a member of, or `"custom"` if they are only in custom groups) and write the corresponding `group_id` UUID as the `classification` field (`group_ref` variant) in each relevant capability declaration.
- **FR-3.2** The `classification` field is advisory and informational. The receiving peer sees an opaque UUID. It is not used for enforcement on the sending side — enforcement is at the capability handler (§5.2).
- **FR-3.3** Capabilities that the requesting peer is not permitted to access SHALL be **omitted from the OFFER manifest entirely** rather than included with a DENY classification. A peer should not learn what capabilities exist on a node they cannot access.
- **FR-3.4** `core.session.heartbeat.1` SHALL always be included in every OFFER regardless of group membership, as required by P2P-CD §D.1.

### 5.4 Access Management HTTP API

The access management API is exposed by the daemon (or a dedicated access capability process — see OQ-1) and proxied at `/access/*`. It is local-only; it is not callable by remote peers.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/access/groups` | List all groups (built-in and custom) |
| `POST` | `/access/groups` | Create a custom group |
| `GET` | `/access/groups/{group_id}` | Get group detail including capability rules |
| `PATCH` | `/access/groups/{group_id}` | Update group name, description, or capability rules (custom groups only) |
| `DELETE` | `/access/groups/{group_id}` | Delete a custom group (built-in groups return 403) |
| `GET` | `/access/peers/{peer_id}/groups` | List groups a peer is assigned to |
| `POST` | `/access/peers/{peer_id}/groups` | Assign a peer to a group; body: `{ group_id }` |
| `DELETE` | `/access/peers/{peer_id}/groups/{group_id}` | Remove a peer from a group |
| `GET` | `/access/peers/{peer_id}/permissions` | Resolve effective permissions for a peer across all capabilities |
| `POST` | `/access/peers/{peer_id}/deny` | Explicitly DENY a peer — closes active session, caches NONE outcome |

### 5.5 Session Lifecycle Integration

- **FR-5.1** When a new peer completes an invite exchange and their first session enters ACTIVE, the daemon SHALL ensure a `PeerGroupMembership` record for `howm.default` is present for that peer if no other group assignment exists. The `howm.default` implicit fallback (§4.4) means this record is optional for correctness, but explicit record creation aids auditability.
- **FR-5.2** When an operator calls `POST /access/peers/{peer_id}/deny`, the daemon SHALL send a P2P-CD CLOSE message with `reason_code: 2` (auth-failure) to the peer if a session is currently ACTIVE, then cache the outcome as NONE per §9.1 of the spec.
- **FR-5.3** When a peer's group membership changes, the daemon SHALL treat this as a classification change and MUST rebroadcast per P2P-CD §8.1: recompute the personal hash, increment `sequence_num`, and send a new OFFER to all active sessions. The active set remains operational during re-exchange per §8.4, but re-evaluation of the affected peer's capabilities against the new group membership MUST complete before the CONFIRM for the new exchange is sent. A peer who loses access to a capability will have it removed from the active set in the new exchange; a peer who gains access will have it added.
- **FR-5.4** The access management HTTP API (§5.4) is a thin wrapper around the same shared library used by capability processes. It is hosted by the daemon and is not a separate process.

### 5.6 UI

- **FR-6.1** The main Howm UI SHALL display each connected peer's current group assignment(s).
- **FR-6.2** The UI SHALL allow the operator to drag/assign peers between groups.
- **FR-6.3** The UI SHALL show the effective permission summary for a selected peer (`GET /access/peers/{peer_id}/permissions`).
- **FR-6.4** Built-in groups SHALL be visually distinguished from custom groups.
- **FR-6.5** The UI SHALL warn before assigning a peer to `howm.trusted` (relay access has bandwidth implications).

---

## 6. Non-Functional Requirements

- **NFR-1** Permission resolution (FR-2.1) SHALL complete in < 5ms under normal load.
- **NFR-2** The `access.db` SQLite database SHALL use WAL mode to allow concurrent reads from multiple capability processes without blocking.
- **NFR-3** Group membership changes SHALL propagate to all capability processes within 100ms (via SQLite WAL read or IPC notification).
- **NFR-4** The access management API SHALL be unreachable from remote peers. It MUST NOT be proxied through any path accessible over the WireGuard tunnel.

---

## 7. Well-Known Group UUIDs

These UUIDs are fixed by this specification. All Howm nodes use the same values, enabling stable `group_ref` identification across the mesh.

| Group | UUID |
|-------|------|
| `howm.default` | `00000000-0000-0000-0000-000000000001` |
| `howm.friends` | `00000000-0000-0000-0000-000000000002` |
| `howm.trusted` | `00000000-0000-0000-0000-000000000003` |

Custom group UUIDs are generated as UUIDv4 at creation time and are node-local.

---

## 8. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-1 | Should access management live as a standalone capability process or integrated into the daemon as a shared library? | Partially closed — the permission resolution path (FR-2.1) is on the hot path for every capability request from every peer. A standalone process would require an IPC call per request, adding latency on paths like messaging ACK where every millisecond counts. The **recommended approach** is a shared Rust library (`p2pcd-access` or similar) that all capability processes link against directly, reading from `access.db` via WAL mode with no network or socket hop. The library exposes a single synchronous function: `resolve_permission(peer_id, capability_name) -> PermissionResult`. The access management HTTP API (FR-5.4) remains a thin wrapper around the same library, hosted by the daemon. Specific library name, crate structure, and linking strategy are left to the implementing engineer. |
| OQ-2 | Phase 1 makes built-in group capability rules read-only. Should phase 2 allow operators to modify built-in tier rules, or should built-in tiers be permanently fixed and custom groups be the only customisation surface? | Open |
| OQ-3 | The `classification` field in the OFFER (FR-3.1) uses the highest-privilege built-in group UUID. For a peer who is only in custom groups, it uses `"custom"`. Should this instead use the custom group's own UUID, accepting that the receiving peer sees an opaque value they cannot interpret? | Open |
| OQ-4 | Active session behaviour on membership change. | Closed — P2P-CD §8.1 is unambiguous: a classification change MUST trigger an immediate rebroadcast and re-exchange. FR-5.3 reflects this. Active set continuity during re-exchange is provided by §8.4. No open question remains. |
| OQ-5 | Layering between group-level capability gate and per-offering access gate in `howm.social.files.1`. | Closed — the two gates are orthogonal and operate independently. Gate 1 (P2P-CD session layer): does this peer have `howm.social.files.1` in their active set at all? Determined entirely by the OFFER/CONFIRM handshake — if the peer's group does not grant the capability, it is absent from the OFFER and they never know it exists. Gate 2 (application layer): given that the capability is active, which offerings can this peer see? Determined by each offering's access policy — peer_id explicit or group-level (`friends`, `trusted`). Neither gate depends on the other's logic; they share only vocabulary (group names and peer_ids). BRD-003 should document that offering access policies reference the same group system defined here. |

---

## 9. Dependencies

- P2P-CD spec §6 (Trust Gates and Peer Classification), §9 (Auto-Deny and Peer Cache).
- All capability processes that enforce access control must link against or call the shared permission resolution interface (FR-2.1). This creates a compile-time or runtime dependency on the access system for: `howm.social.feed.1`, `howm.social.messaging.1`, `howm.social.files.1`, `howm.world.room.1`, `core.network.relay.1`, `core.network.peerexchange.1`.
- `rusqlite` with `bundled` feature — `access.db` in WAL mode.
- Daemon session lifecycle hooks — for deny (FR-5.2), rebroadcast on membership change (FR-5.3), and default group assignment on invite completion (FR-5.1).

---

## 10. Success Criteria

- A newly invited peer is automatically in `howm.default` and can exchange heartbeat and attest but cannot access feed, messages, files, or room.
- Moving a peer to `howm.friends` grants feed, messages, files, and room access without restarting the session.
- A peer in both a custom group granting files and `howm.default` (which denies files) receives files access — most permissive wins.
- Calling `POST /access/peers/{peer_id}/deny` closes the active session and prevents reconnection until the peer's manifest changes.
- A capability process receives a request from a peer outside its permitted group and returns 403 without revealing group information.
- The `classification` field in an OFFER to a `howm.friends` peer carries the `howm.friends` well-known UUID.

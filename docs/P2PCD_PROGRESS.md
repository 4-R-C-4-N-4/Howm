# P2P-CD Implementation Progress Log

Tracking live progress of the P2P-CD-01 v0.3 rewrite per `P2PCD_WORK.md`.

---

## Status

| Phase | Task | Status | Notes |
|-------|------|--------|-------|
| 0 | 0.1 Core types crate | ✅ Done | All types, Role::matches, ScopeParams::reconcile, TrustPolicy::evaluate, compute_intersection, WgPeerState |
| 0 | 0.2 CBOR encoding | ✅ Done | DiscoveryManifest encode/decode, ProtocolMessage length-prefix encode/decode, personal_hash |
| 0 | 0.3 TOML config | ✅ Done | PeerConfig, generate_default, to_manifest, trust_policies, validate_capability_name |
| 1 | 1.1 WireGuard state monitor | ⬜ Not started | |
| 2 | 2.1 TCP transport layer | ⬜ Not started | |
| 2 | 2.2 Session state machine | ⬜ Not started | |
| 2 | 2.3 OFFER/CONFIRM exchange | ⬜ Not started | |
| 3 | 3.1 Protocol engine coordinator | ⬜ Not started | |
| 3 | 3.2 Integrate engine into daemon startup | ⬜ Not started | |
| 4 | 4.1 PING/PONG heartbeat | ⬜ Not started | |
| 5 | 5.1 Trust gate with friends list | ⬜ Not started | |
| 5 | 5.2 Peer cache with auto-deny | ⬜ Not started | |
| 6 | 6.1 Rebroadcast on change | ⬜ Not started | |
| 6 | 6.2 Capability notification interface | ⬜ Not started | |
| 7 | 7.1 P2P-CD status HTTP API | ⬜ Not started | |
| 7 | 7.2 Remove legacy discovery/HTTP polling | ⬜ Not started | |
| 7 | 7.3 Update social-feed capability | ⬜ Not started | |

**Tests:** `p2pcd-types` — 31/31 passing ✅

---

## Design Decisions & Changes

### Capability Consolidation — `howm.social.feed.1`

**Session:** First implementation session.

**Issue found:** The spec (§7.4) defines intersection per capability *name* — same name, roles must match. The original two-capability model (`p2pcd.social.post.1/PROVIDE` + `p2pcd.social.feed.1/CONSUME`) produces zero social matches between two Normal Users because PROVIDE+PROVIDE and CONSUME+CONSUME don't match on the same name.

The POC doc §9 scenarios assumed cross-name matching (Alice's PROVIDE matches Bob's CONSUME on *different* names), which contradicts the spec.

**Decision (from IV):** Consolidate to single capability `howm.social.feed.1` with `role: both, mutual: true`. Direction (who fetches/posts) pushed to application layer inside the social-feed capability. The daemon only gates access.

**Result:** Correct intersection behavior for all scenarios:
- Social ↔ Social → `[heartbeat, howm.social.feed.1]`
- Social ↔ No-Social → `[heartbeat]`
- Private ↔ Stranger (trust gate) → `[heartbeat]`
- Private ↔ Friend → `[heartbeat, howm.social.feed.1]`

**Files changed:** `p2pcd-types/src/lib.rs`, `p2pcd-types/src/cbor.rs`, `p2pcd-types/src/config.rs`
**Doc updated:** `p2pcd-poc-config.md` (revision note appended)

### CloseReason::AuthFailure

The `AuthFailure` variant had a placeholder `***` value in the source. Fixed to `= 2` per the spec (reason_code 2 in the state machine table).

---

## Open Questions (resolved)

| # | Question | Answer |
|---|----------|--------|
| 1 | `CloseReason::AuthFailure` value | `2` (per spec state machine table) |
| 2 | debug/dev flags | env vars `HOWM_DEBUG`/`HOWM_DEV` |
| 3 | Web UI consumers of proxy_routes/network_routes | TBD at Phase 7 cleanup |
| 4 | WireGuard interface name default | `howm0` |

---

## Next Up

**Phase 1 — Task 1.1: WireGuard state monitor**

File to modify: `node/daemon/src/wireguard.rs`

- `parse_wg_dump(output: &str) -> Vec<WgPeerState>`
- `WgPeerMonitor` struct with background polling loop
- `WgPeerEvent` enum: `PeerVisible(PeerId)`, `PeerUnreachable(PeerId)`, `PeerRemoved(PeerId)`
- Event channel to feed the protocol engine
- Do NOT touch existing WG setup functions (used by invite system)

**Then Phase 2.1** — TCP transport layer, then session state machine, then OFFER/CONFIRM.

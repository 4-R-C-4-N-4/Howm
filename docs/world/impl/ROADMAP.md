# Howm World — Remaining-Work Roadmap

**Date:** 2026-06-16
**Branch:** `world`
**Owner priorities (in order):** SDK adoption · Multiplayer · World-gen algorithm conformance · All entity types

This roadmap sequences the unbuilt work found in the doc↔code gap analysis
(see PROGRESS.md "Phase R2"). It is dependency-ordered: each phase unblocks the
next. Effort sizes are rough (S ≤ 1 day, M = 2–4 days, L = 1–2 weeks).

Grounding references:
- SDK surface: `node/p2pcd/src/capability_sdk.rs` (`CapabilityApp`, `PeerStream`,
  `PeerTracker`, `CapabilityRuntime`), `node/p2pcd/src/bridge_client.rs`
  (`send_msg`, `rpc_call`, `broadcast_event`, `list_peers`, `get_local_peer_id`,
  `peer_id_bytes`). Reference cap: `capabilities/presence/src/main.rs`.
- Spaces protocol: `docs/world/howm-spaces.md` (seeds/constants/RPC quoted below).
- Gen algorithms: `docs/world/howm-spec.md`, `howm-building-form.md`,
  `howm-atmosphere.md`. Renderer: `docs/world/astral-projection.md`.

---

## Phase S — SDK Adoption (foundation) · size M

**Why first:** every multiplayer feature needs peer awareness (who's online,
their `peer_id` + `wg_address`) and the send/RPC primitives. Home/Inside/avatar
generation all need the **local** `peer_id` as a seed. Today `world/main.rs`
hand-rolls axum, binds nothing to p2pcd, and has no notion of peers.

**Deliverables**
1. Refactor `world/main.rs` onto `CapabilityApp::new("howm.world.room.1", port, state)`
   - Move all existing routes into `.with_routes(|r| …)` (district, geometry,
     objects, atmosphere, prefetch, scene, map, neighbors, neighborhood, `/live`
     WS). `with_routes` takes a raw axum `Router`, so the WS upgrade route ports
     over unchanged.
   - `.with_ui(&UI_DIR)`, `init_tracing()`, drop the manual `health` + bind code.
2. Add a `PeerTracker`/`PeerStream` for `howm.world.room.1` (Type-3 pattern from
   presence) so the cap knows the active peer set + each peer's `wg_address`.
3. Add a shared `AppState { bridge: BridgeClient, peers: PeerTracker, local_peer_id }`.
   Resolve `local_peer_id` lazily via `BridgeClient::get_local_peer_id` /
   `peer_id_bytes` (used as the seed for home/Inside/avatar).
4. `.with_inbound_handler(inbound)` stub that decodes `InboundMessage` and routes
   by method prefix (`presence.*`, `avatar.*`, `room.*`) — handlers land in later
   phases.

**Verification:** existing endpoints unchanged (re-run the smoke test +
`/scene`); `list_peers` returns the active world peers; local peer id resolves.
**Risk/decision:** confirm `CapabilityApp` body-limit is large enough for scene
JSON responses (use `.with_body_limit` if needed). No behavior change to gen.

---

## Phase G — World-Gen Algorithm Conformance · size M (parallelizable with S)

Pure-Rust, no networking — can proceed independently of Phase S. Makes the
generator match the spec's deterministic algorithms so worked-example appendices
verify. Order within the phase is by value/independence.

1. **Object salt-registry conformance (creatures, flora)** · M
   - Rewrite `gen/creatures.rs` field derivation to the Appendix-A salt registry:
     each field `ha(creature_seed ^ 0xa1..0xe2)`, derive `locomotion_mode` first
     then map to ecological role (§15.1.5). Add Appendix C.1 assertions.
   - Add structured flora fields in `gen/flora.rs`: `growth_stage` (5-stage from
     `effective_age`), `has_canopy`/`canopy_radius` (0xc2/0xc3), `shed_type`/
     `shed_rate` (0xc4/0xc5), height/spread (0xc6–c8); align growth-form vocab to
     spec. Add Appendix D assertions.
2. **Atmosphere colour/light output** · S — add sky colour, ambient light,
   sun/moon direction+colour, `precip_type` to `AtmosphereState` (atmosphere
   §2.3–2.5, §3.4–3.5). Lets `/atmosphere` and the scene compiler stop deriving
   sky independently.
3. **Building shell interiors + gap-fill + entry-adjacency** · M
   - `buildings.rs`: inset-polygon interiors (§12.8 / building-form §11), no-alley
     gap-fill post-process (§12.2), wire `wall_adjacency_tol` into entry-point
     selection (currently dead config).
4. **Geometry rigor** · M — river Catmull-Rom control points from gy±2 neighbour
   polygons (`rivers.rs`, currently linear-approx), true Sutherland-Hodgman where
   `zones.rs`/`buildings.rs` approximate, edge-intersection river adjacency in
   `blocks.rs`.
5. **Unify `RenderPacket`** · M — `objects.rs::RenderPacket` is defined but never
   produced; have each generator emit it uniformly (material_seed/state_seed/
   interaction_zone). Cleans up the entity contract before adding new types.

**Verification:** new appendix-vector tests (C.1, D); existing 137 tests stay green.

---

## Phase E — All Entity Types (Outside completeness + Inside/spaces entities) · size L

Depends on: Phase S (needs `local_peer_id`, installed-cap list, peer pairs),
Phase G #5 (RenderPacket contract). Split into Outside-additions and the
Inside/Underground generators.

### E1 — Home placement in the Outside · size S (only needs Phase S)
- New `gen/home.rs`: `home_seed = ha(peer_id_u32 ^ cell_key)`,
  `point_in_polygon(cell_polygon, home_seed)`, walkable re-roll (16 attempts,
  `^attempt`), archetype from `HOME_ARCHETYPES = [pavilion,tower,chamber,portal,
  burrow,shrine]` via `ha(home_seed ^ 0xb1d)`, radius/height from `^0xb1d^0x1/^0x2`.
  Consts `HOME_MIN/MAX_RADIUS 1.5/4.0`, `HOME_MIN/MAX_HEIGHT 2.5/5.0`,
  `point_on_road` within `LAMP_OFFSET 3.5`.
- Inject into the Outside scene; HDL uses district palette, distinct form.
- Endpoint: `GET /district/:ip/homes` (or fold into `/scene` when peers known).

### E2 — Inside generation · size M
- New `gen/inside.rs`: `inside_seed = ha(peer_id)`; rooms =
  `[entry_hall] + [capability_room(cap) …]` from the **installed-cap list**
  (fetch via bridge / daemon). Room seed `ha(inside_seed ^ ha(cap_name))`;
  category→room-type (feed→gallery, presence→hearth, messaging→correspondence,
  files→archive, voice→amphitheatre); geometry from area/aspect; layout
  cardinal(≤4)/ring(≤8)/corridor(>8) via `ha(inside_seed ^ 0x1a70)`
  *(spec writes `0xla70` — typo, confirm)*. Consts: `HALL_BASE_AREA 40`,
  `HALL_AREA_PER_CAP 10`, `ROOM_BASE_AREA 30`, `DOOR_WIDTH/HEIGHT 1.2/2.8`, etc.
- Endpoints: `GET /inside/:peer_id`, `GET /inside/:peer_id/scene`.

### E3 — Room-feature entities · size M (depends on E2 + live cap state)
- Feed posts (`fixture:display_surface`), attachments, message threads, files —
  each a `DescribedEntity` whose params come from live capability state pulled via
  the bridge (counts, unread, sizes). Mutations pushed as `DescriptionPacket`/
  `StatePacket` (ASTRAL §4) over the live channel.

### E4 — Underground / tunnel generation · size M (needs Phase S peer pairs)
- New `gen/tunnel.rs`: `tunnel_seed = ha(min(A,B) ^ max(A,B))` (byte-wise on WG
  keys); length/width/height from `latency_ms`/`bandwidth_kbps`/`active_cap_count`
  (stub metrics initially — see decision D3); aesthetic **lerp** between the two
  cells' palettes along `t`; cross-section from `ha(tunnel_seed ^ 0xc055) %
  CROSS_SECTIONS`; wall/floor/ceiling slabs (`building:block` archetype); capability
  markers at `(i+1)/(n+1)` for the mutually-active set; uptime→lighting table.
  Consts in §3.6 (`TUNNEL_BASE_LENGTH 10`, `TUNNEL_SEGMENT_LENGTH 4.0`, …).
- Tunnel-door entities in the entry hall (E2): `door_seed = ha(inside_seed ^
  tunnel_seed)`, remote-aesthetic surface, online→glow.
- Endpoint: `GET /underground/:peer_a/:peer_b/scene`.

### E5 — Portals + transitions · size M
- Portal entity = the fixed HDL graph in spaces §5.1 (verbatim traits: tall/0.3
  aspect, `opacity shifting`, `emission glow/breathing`, `regard welcoming`, the
  two regard→emission sequences). Emit at every door/home/tunnel mouth.
- Transition state machine (idle→activating→loading→ready / timeout / offline),
  `PORTAL_TIMEOUT_MS 10000`, `PORTAL_REGARD_RADIUS 3.0`. Local transitions are
  instant; remote (Outside→peer Inside, Underground→peer Inside) call the peer's
  node (decision D2: define `howm.world.room.get`/`inside.get` RPC — unnamed in spec).

### E6 — Renderer-side new entity rendering · size M
- The renderer already raymarches `DescribedEntity`. New work: render avatars
  (Phase M), portal interaction (regard radius → `injectEvent('relation.regard.
  activated')` already supported by SequenceEngine), and a space-transition hook
  in `entry.ts`/RenderLoop that swaps the active SceneProvider on portal cross.
  Keep Rust-side scene compilation (do **not** do the full Option-B migration —
  decision D4).

**Verification:** per-entity determinism tests; visual check of home/inside/tunnel
scenes via `/scene` endpoints in the renderer.

---

## Phase M — Multiplayer · size L

Depends on: Phase S (peers + send/RPC), E1/E2/E4 (somewhere to be present),
E6 (render avatars).

1. **Presence relay** · M — `howm.world.presence.1` carrying
   `{position, orientation, velocity}` at 2–4 Hz over `core.data.stream.1`
   (or `core.data.event.1`, decision D5):
   - Outside: peer-to-peer, only directly-tunnelled peers in the same cell.
   - Inside: **host-mediated** — the Inside owner relays each visitor's position
     to every other visitor it has tunnels to (`relay_presence`, O(N²) fan-out;
     uncapped initially per OQ-S9). Implement in the inbound handler + a relay task.
   - Underground: 1-to-1 between the two endpoints only.
2. **Avatars** · M — `howm.world.avatar.get { peer_id } → { description_graph }`
   RPC (`BridgeClient::rpc_call` + an inbound RPC handler). Default avatar:
   `avatar_seed = ha(peer_id ^ 0xface)`, tall/moderate, surface/material from the
   visitor's **home** district aesthetic, faint background glow. Cache fetched
   avatars; render incoming peer positions as their avatar entity.
3. **Wire into the live channel** · M — extend `stream/protocol.rs` +
   `HowmStreamProvider.ts` with `peer_enter`/`peer_move`/`peer_leave` messages so
   the renderer shows other players moving in real time.

**Verification:** two daemons on loopback (`node/scripts/local-two-peer.sh`),
both running the world cap, redeem an invite, confirm each sees the other's avatar
move in the Outside and inside a shared Inside.

---

## Phase V — IPv6 Worlds · size M (independent; do anytime after Phase G)

`cell.rs::from_ip_str` is IPv4-only. Add the IPv6 path (BRD §3): `/32` cells,
`key = (group0<<16)|group1`, `gx = group1, gy = group0`; carry `ip_mode "v4"|"v6"`
+ `ip_bytes` (4 or 16) in the data contract (BRD §9). Special regions (`::1`,
`fe80::/10`, `2001:db8::/32`). Wilderness rendering = BRD phase W5 (deferred).

---

## Cross-cutting: data contract + web-shell · size S
- Emit the BRD §9 `OutsideDescription` public seed fields (`ip_bytes`, `ip_mode`,
  `cell_key`, `neighbor_keys`) so clients can stitch districts deterministically.
- Web-shell launch affordance: `manifest.json` `ui.style: "fullscreen"` isn't
  handled by `App.tsx` (knows `nav`/`fab`). Either add a `fullscreen` case to the
  shell or set a handled style. (UI rebuild required.)

---

## Open decisions (confirm before coding the dependent phase)

- **D1 — `0xla70` salt typo** (Inside layout_seed, spaces §2.5): non-hex `l`/`a`;
  almost certainly `0x1a70`. Confirm, then lock with a test.
- **D2 — Inside-describe RPC name:** spec names `howm.world.avatar.get` and
  `howm.world.presence.1` but leaves the "describe a peer's Inside" RPC unnamed
  (OQ-S6). Propose `howm.world.room.get { peer_id } → { scene }`.
- **D3 — Tunnel metric sources:** `latency_ms`/`bandwidth_kbps`/`uptime` come from
  the WireGuard/connection layer, not specified here. Stub with constants first;
  wire real metrics via the bridge later.
- **D4 — Renderer architecture:** keep Rust-side scene compilation (Option A/C);
  defer the full Option-B renderer-owned interpretation (`resolveGeometry`,
  SceneGraph/packet protocol) — large and not required for multiplayer. Add only
  avatar/portal/transition rendering.
- **D5 — Presence transport:** `core.data.stream.1` (continuous) vs
  `core.data.event.1` (OQ-S4 prefers events for real-time state). Note: the
  `event` core cap's fan-out is currently a stub (PROGRESS R2) — if we choose
  events, that must be implemented first.

---

## Suggested execution order

```
S  (SDK adoption)  ─┬─►  E1 home  ─►  E2 inside  ─►  E3 room features
                    │                     └─►  E4 tunnel  ─►  E5 portals  ─►  E6 render
                    └─►  M (multiplayer: presence → avatars → live wire)
G  (gen conformance)  ── parallel, no deps ──►  V (IPv6)
```

Recommended first slice to ship end-to-end value: **S → E1 → (minimal M: avatar
get + Outside presence)** — two players seeing each other's avatars walk around
the same Outside district. Everything else layers onto that spine.

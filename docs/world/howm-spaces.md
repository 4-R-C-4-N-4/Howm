# Howm Spaces — Inside & Underground Specification

**Author:** Ivy Darling  
**Project:** Howm  
**Document type:** Design Specification  
**Status:** Draft  
**Version:** 0.1  
**Date:** 2026-03-29  
**Depends on:** `howm-suite.md`, `howm-spec.md` (Outside generation), `howm-description-language.md` (HDL), `astral-projection.md` (renderer)

---

## 1. The Three Spaces

Howm is composed of three navigable spaces. Each is generated from a different seed source, rendered by the same engine (Astral Projection), and described in the same vocabulary (HDL). The renderer does not know which space it is drawing — it receives description graphs and renders them.

```
┌──────────────────────────────────────────────────────────────┐
│                         OUTSIDE                              │
│                                                              │
│  Seed: IP address (cell_key)                                 │
│  Character: the city — districts, roads, buildings, creatures│
│  Ownership: none — deterministic, same for all peers         │
│  Presence: peer-to-peer (no host)                            │
│                                                              │
│        ┌──────────┐              ┌──────────┐                │
│        │ Alice's  │              │  Bob's   │                │
│        │ building │              │ building │                │
│        │  entry   │              │  entry   │                │
│        └────┬─────┘              └────┬─────┘                │
└─────────────┼─────────────────────────┼──────────────────────┘
              │                         │
              ▼                         ▼
┌─────────────────────┐   ┌─────────────────────┐
│   ALICE'S INSIDE    │   │    BOB'S INSIDE     │
│                     │   │                     │
│ Seed: Alice's       │   │ Seed: Bob's         │
│   peer_id +         │   │   peer_id +         │
│   capabilities      │   │   capabilities      │
│                     │   │                     │
│ Character: home —   │   │ Character: home —   │
│   rooms for each    │   │   rooms for each    │
│   capability        │   │   capability        │
│                     │   │                     │
│ Aesthetic: Alice's  │   │ Aesthetic: Bob's    │
│   district palette  │   │   district palette  │
│                     │   │                     │
│ Ownership: Alice    │   │ Ownership: Bob      │
│ Presence: host-     │   │ Presence: host-     │
│   mediated (Alice   │   │   mediated (Bob     │
│   relays visitors)  │   │   relays visitors)  │
│                     │   │                     │
│ ┌─────┐ ┌─────┐    │   │ ┌─────┐ ┌─────┐    │
│ │Door │ │Door │    │   │ │Door │ │Door │    │
│ │→Bob │ │→Carol│   │   │ │→Alice│ │→Carol│   │
│ └──┬──┘ └─────┘    │   │ └──┬──┘ └─────┘    │
└────┼────────────────┘   └────┼────────────────┘
     │                         │
     ▼                         ▼
┌──────────────────────────────────────────────┐
│              UNDERGROUND                      │
│                                              │
│  Seed: ha(min(peer_A, peer_B)               │
│         XOR max(peer_A, peer_B))            │
│                                              │
│  Character: tunnel — passage between homes   │
│  Length: proportional to latency             │
│  Width: proportional to bandwidth            │
│  Aesthetic: gradient blend from A to B       │
│                                              │
│  Ownership: shared (both endpoints)          │
│  Presence: 1-to-1 (only the two peers)       │
└──────────────────────────────────────────────┘
```

### 1.1 Navigation Flow

A complete journey from Alice's district to Bob's home:

```
1. Alice is Outside in her IP district
2. Alice walks to her home structure (a unique building at a peer_id-derived position)
3. Alice approaches the portal door — it glows, recognising her
4. Alice crosses the threshold → portal flares → enters her Inside (her home)
5. Alice finds the tunnel door labelled "Bob" in her entry hall
6. Alice approaches — the door glows with Bob's district tint
7. Alice crosses → portal loads → enters Alice↔Bob Underground
8. Alice walks through the tunnel (aesthetic shifts from her palette to Bob's)
9. Alice reaches the far end → portal loads Bob's Inside via RPC
10. Alice emerges into Bob's Inside (Bob's home)
11. Alice can explore Bob's capability rooms
12. Alice walks out through Bob's building exit portal
13. Alice is Outside in Bob's IP district — Bob's home structure is visible nearby
```

The reverse works identically. From Bob's Outside, find Bob's home structure, enter, find Alice's tunnel door, walk through.

### 1.2 Home Placement

A peer's home is not an existing building in the district. It is a **unique structure injected into the Outside world**, placed on any walkable surface in the peer's cell. Peers are not citizens of the generated world — they are agents whose structures don't follow the district's generative rules. A home can appear in a park, a plaza, a road edge, wedged between buildings, or standing alone in an open space. It belongs to the peer, not to the address.

#### Home position

The home's position is derived from the peer's WireGuard public key (peer_id), not from the cell's block/plot system:

```
cell_key       = ip_to_cell_key(peer_ip)
cell_polygon   = voronoi_cell(cell_key)        // the district boundary

// Hash peer_id to a position within the cell
// peer_id is 32 bytes; pack first 4 bytes as u32 for hashing
peer_id_u32    = (peer_id[0] << 24) | (peer_id[1] << 16) | (peer_id[2] << 8) | peer_id[3]
home_seed      = ha(peer_id_u32 XOR cell_key)
home_position  = point_in_polygon(cell_polygon, home_seed)
```

The `point_in_polygon` algorithm (howm-spec §11.9) guarantees the position falls within the cell boundary. The same peer_id always produces the same position in the same cell.

#### Walkable surface check

The home position must be on a block interior — not in water and not on a road:

```
home_block_type = block_type_at(home_position)   // which block contains this point
on_road         = point_on_road(home_position)    // within road corridor width

if home_block_type == "water" or on_road:
  // Re-roll with successive salts until we hit valid ground
  for attempt in 1..16:
    fallback_seed = ha(home_seed ^ attempt)
    fallback_pos  = point_in_polygon(cell_polygon, fallback_seed)
    if block_type_at(fallback_pos) != "water" and not point_on_road(fallback_pos):
      home_position = fallback_pos
      break
  // After 16 attempts, place at cell centroid (guaranteed land in most cells)
```

`point_on_road` tests whether the position falls within `CONFIG.LAMP_OFFSET` (3.5 world units) of any road segment centreline. Roads are traversal infrastructure — homes don't belong on them.

Valid placement surfaces: building blocks (between or beside existing buildings), parks (pavilion in the grass), plazas (freestanding chamber in the open square), riverbanks (structure at the water's edge). The generated world flows around the injected home.

#### Home structure

The home's exterior is a standalone structure generated from the peer's identity, not from the district's plot system. It uses the district's aesthetic palette for material coherence but has its own form:

```
home_archetype_seed = ha(home_seed ^ 0xb1d)
home_archetype      = HOME_ARCHETYPES[home_archetype_seed % HOME_ARCHETYPES.length]

HOME_ARCHETYPES = [
  "pavilion"    // open-sided, low, park-friendly
  "tower"       // narrow, tall, fits between buildings
  "chamber"     // compact, enclosed, fits anywhere
  "portal"      // minimal — a doorframe standing alone
  "burrow"      // partially subterranean, low profile
  "shrine"      // small, ornate, landmark-like
]

home_footprint_radius = CONFIG.HOME_MIN_RADIUS
                      + (ha(home_seed ^ 0xb1d ^ 0x1) / 0xFFFFFFFF)
                        × (CONFIG.HOME_MAX_RADIUS - CONFIG.HOME_MIN_RADIUS)

home_height = CONFIG.HOME_MIN_HEIGHT
            + (ha(home_seed ^ 0xb1d ^ 0x2) / 0xFFFFFFFF)
              × (CONFIG.HOME_MAX_HEIGHT - CONFIG.HOME_MIN_HEIGHT)
```

The home structure carries its own HDL description graph, using the district aesthetic palette for `being.surface` and `being.material` but with a distinct `being.form` that doesn't match any generated building. It looks like it belongs materially but not architecturally — same stone, different shape.

#### Multiple peers per cell

Multiple peers can share a cell (same `/24` subnet). Each peer's home_seed is different (derived from their unique peer_id), so each home appears at a different position within the cell. They coexist naturally — multiple unique structures scattered across the district.

If two peers' home positions happen to overlap (extremely rare given the continuous position space), the structures simply occupy the same area. The renderer handles overlapping entities without issue — SDFs naturally blend at close range.

#### Home configuration

```
CONFIG = {
  HOME_MIN_RADIUS:   1.5,     // world units — smallest footprint
  HOME_MAX_RADIUS:   4.0,     // world units — largest footprint
  HOME_MIN_HEIGHT:   2.5,     // world units
  HOME_MAX_HEIGHT:   5.0,     // world units
}
```

---

## 2. Inside — Peer Identity as Architecture

### 2.1 Seed and Aesthetic

The Inside is seeded from the peer's identity and inherits its aesthetic from the peer's home district:

```
inside_seed  = ha(peer_id)                    // master seed for layout
cell_key     = ip_to_cell_key(peer_ip)        // peer's home cell
aesthetic    = derive_aesthetic(cell_key)      // from howm-spec §10
```

The Inside uses the same aesthetic palette as the peer's Outside district: same popcount_ratio, same age, same domain, same hue. This means crossing the threshold from Outside to Inside feels coherent — the material vocabulary, colour temperature, and surface detail character carry through. The architectural layout changes (it's a home, not a city block), but the materials are familiar.

### 2.2 Room Model

The Inside is a set of connected rooms. Every Inside has an **entry hall** plus one room per installed capability.

```
rooms = [entry_hall] + [capability_room(cap) for cap in installed_capabilities]
```

#### Entry Hall

The entry hall is the central space. It connects to:
- The Outside (via the building entry point — the door to the street)
- Every capability room (via interior doorways)
- Every active tunnel (via tunnel doors)

The entry hall's size scales with the peer's total capability count and connection count:

```
hall_area = CONFIG.HALL_BASE_AREA
          + installed_capabilities.length × CONFIG.HALL_AREA_PER_CAP
          + active_tunnels.length × CONFIG.HALL_AREA_PER_TUNNEL
```

The entry hall always exists, even for a peer with zero capabilities. An empty hall with one door (to Outside) is the minimum viable Inside — a bare room with nothing in it. Each capability installation adds a doorway. Each peer connection adds a tunnel door.

#### Capability Rooms

Each installed capability produces one room. The room's properties are derived from the capability's nature and state:

```
capability_room {
  capability_name: string              // e.g. "social.feed"
  room_seed:       ha(inside_seed ^ ha(capability_name))
  area:            base_area(capability_type) × activity_multiplier(state)
  height:          base_height(capability_type)
  room_type:       derive_room_type(capability_name)
  features:        derive_features(capability_name, capability_state)
  access:          derive_access(capability_name, access_groups)
}
```

### 2.3 Capability Type → Room Type

Each capability category maps to a room character. The room type drives the HDL description graph for the room's geometry and contents:

| Capability category | Room type | Character |
|---|---|---|
| `social.feed` | **gallery** | Open walls with display surfaces. Posts manifest as displayed objects — text surfaces, image frames, media pedestals. Recent posts are prominent; old posts recede. High activity = crowded gallery. Low activity = sparse exhibition. |
| `social.presence` | **hearth** | A small warm space near the entry hall. Shows active/away state of the peer. When the peer is active, the hearth glows. When away, it dims. Visitor presence radiates from here. |
| `messaging` | **correspondence room** | A writing desk, shelves, surfaces with text. Unread messages produce visible indicators — glowing objects, stacked items. Conversation threads are spatial clusters. |
| `files` | **archive / vault** | Shelved storage. File count drives room size. Large files are large objects. The room feels full or empty based on storage usage. Shared files are on accessible shelves; private files are behind barriers. |
| `voice` | **amphitheatre / parlour** | Open acoustic space. When a call is active, the room is alive — sound indicators, resonance effects. When idle, it's a quiet chamber. Room size scales with max participant count. |

These mappings are starting points. The important thing is the pattern: the capability's function determines the room's archetype, and the capability's state determines the room's population and energy.

### 2.4 Room Features as HDL Entities

Each room contains entities described in HDL, just like Outside objects. The entities represent capability state:

**Feed room entities:**

```
// Each post is a display_surface fixture
post_entity {
  archetype: "fixture:display_surface"
  being.form.silhouette: "wide"
  being.form.scale: proportional to post length
  being.surface.texture: "inscribed"
  being.surface.age: derived from post age
  effect.emission: if unread → "glow", channel: "foreground"
  relation.context.narrative: "scribe"
}

// Media attachments are ornament fixtures
attachment_entity {
  archetype: "fixture:ornament"
  being.form: derived from media type (image = "wide", audio = "compact", video = "tall")
  effect.emission: if playable → "pulse"
}
```

**Messaging room entities:**

```
// Each conversation thread is a cluster of objects
thread_entity {
  archetype: "fixture:display_surface"
  being.form.composition: "stacked"
  being.form.composition.count: message_count in thread (capped)
  being.surface.age: derived from last message timestamp
  effect.emission: if has_unread → "glow", channel: "background", rhythm: "breathing"
}
```

**File archive entities:**

```
// Each file is a container
file_entity {
  archetype: "fixture:offering_point"
  being.form.scale: proportional to file size
  being.surface.texture: derived from file type
    // documents → "inscribed"
    // images → "glazed"
    // archives → "bolted"
    // code → "gridded"
  being.material.density: proportional to file size
}
```

The renderer doesn't know these are capability representations. It receives description graphs and renders them. A visitor walking through the feed room sees display surfaces with text, glowing with unread indicators. They don't need to know it's a social feed — they experience it spatially.

### 2.5 Room Layout

Rooms are arranged around the entry hall using a seeded layout algorithm:

```
room_count = installed_capabilities.length
layout_seed = ha(inside_seed ^ 0xla70)

if room_count <= 4:
  // Rooms arranged as cardinal directions from the entry hall
  // N, E, S, W — entry hall at center
  layout = cardinal_layout(rooms, layout_seed)

if room_count <= 8:
  // Rooms arranged in a ring around the entry hall
  // Doorways at regular intervals around the hall perimeter
  layout = ring_layout(rooms, layout_seed)

if room_count > 8:
  // Rooms arranged as a branching corridor system
  // Main corridor from entry hall, rooms off each side
  layout = corridor_layout(rooms, layout_seed)
```

Room positions are stable — they don't shift when capabilities are added or removed. New capabilities fill the next available slot. Removed capabilities leave their slot empty (an empty room, or a sealed doorway, depending on implementation).

Each room is a rectangular volume with dimensions derived from its area and height:

```
room_width  = sqrt(room.area × room.aspect_ratio)
room_depth  = room.area / room_width
room_height = room.height

// Aspect ratio from capability type
aspect_ratio(gallery) = 1.5    // wide
aspect_ratio(hearth)  = 1.0    // square
aspect_ratio(correspondence) = 1.2
aspect_ratio(archive) = 0.8   // deep
aspect_ratio(amphitheatre) = 1.0
```

### 2.6 Tunnel Doors

Each active WireGuard tunnel produces a door in the entry hall. Tunnel doors are distinct from capability room doorways — they lead Underground rather than to another room.

```
tunnel_door {
  peer_id:      remote_peer_id
  tunnel_seed:  ha(min(local_peer_id, remote_peer_id) XOR max(local_peer_id, remote_peer_id))
  door_seed:    ha(inside_seed ^ tunnel_seed)

  // Door position in entry hall
  position:     seeded position along entry hall perimeter (avoiding capability doorways)

  // Door appearance from remote peer's aesthetic
  remote_cell_key = ip_to_cell_key(remote_peer_ip)
  remote_aesthetic = derive_aesthetic(remote_cell_key)

  // The door's visual character hints at where it leads
  being.surface.texture: from remote_aesthetic
  being.material.substance: from remote_aesthetic
  being.material.temperature: from remote_aesthetic
  effect.emission: if remote peer is online → "glow", channel: "background"
                   if remote peer is offline → "none"

  // Door label from remote peer's name or identifier
  name_seed: ha(tunnel_seed ^ 0x5)
}
```

A door to an online peer glows with the colour character of their district. A door to an offline peer is dark — the tunnel exists but nobody's home. Walking through a dark door still works (you arrive at the Underground, then at their empty Inside), but the visual cue tells you nobody's there.

### 2.7 Activity and Mutation

Unlike Outside (which is immutable and deterministic), Inside is **mutable**. State changes that affect the Inside:

| Event | Effect on Inside |
|---|---|
| Capability installed | New room appears (doorway opens in entry hall) |
| Capability uninstalled | Room sealed (doorway closes or becomes a wall) |
| New peer connection | New tunnel door appears in entry hall |
| Peer disconnection | Tunnel door goes dark (stays visible, becomes inactive) |
| New feed post | New display surface appears in feed room |
| Message received | Thread entity glows with unread indicator |
| File uploaded | New container entity appears in archive room |
| Peer goes online | Their tunnel door starts glowing |
| Peer goes offline | Their tunnel door dims |

The generator pushes these mutations as `DescriptionPacket` (for new entities) or `StatePacket` (for state changes on existing entities) through the same protocol defined in Astral Projection §4. The renderer doesn't distinguish between Outside and Inside mutations — it processes packets identically.

### 2.8 Visitor Access

When a peer visits someone's Inside, what they can see and interact with is governed by the access control system (howm-access groups). The host's access configuration determines which rooms are visible to which visitors:

```
room_access(capability_name, visitor_peer_id):
  // Check if the visitor's access group has permission for this capability
  group = host.access_groups.find(g => g.members.includes(visitor_peer_id))
  if group and group.capabilities.includes(capability_name):
    return visible
  else:
    return hidden
```

Hidden rooms are not included in the visitor's description packets. The doorway appears as a sealed wall. The visitor doesn't know the room exists — it's not a locked door, it's an absent door. This is consistent with P2P-CD's principle that capability activation is bilateral (both peers must agree to activate a capability).

### 2.9 Inside Configuration

```
CONFIG = {
  // ── Home structure (Outside placement) ────────────────────────
  HOME_MIN_RADIUS:        1.5,       // world units — smallest footprint
  HOME_MAX_RADIUS:        4.0,       // world units — largest footprint
  HOME_MIN_HEIGHT:        2.5,       // world units
  HOME_MAX_HEIGHT:        5.0,       // world units

  // ── Entry hall ────────────────────────────────────────────────
  HALL_BASE_AREA:         40,       // world units² — empty hall
  HALL_AREA_PER_CAP:      10,       // additional area per installed capability
  HALL_AREA_PER_TUNNEL:   6,        // additional area per active tunnel
  HALL_HEIGHT:            4.0,       // world units

  // ── Capability rooms ──────────────────────────────────────────
  ROOM_BASE_AREA:         30,       // world units² — empty room
  ROOM_MAX_AREA:          120,      // cap on room growth from activity
  ROOM_BASE_HEIGHT:       3.5,      // world units
  ROOM_ACTIVITY_SCALE:    0.5,      // how much activity multiplies area (0 = none, 1 = double)

  // ── Room entity population ────────────────────────────────────
  MAX_ENTITIES_PER_ROOM:  20,       // cap on visible entities per room
  ENTITY_RECENCY_WINDOW:  50,       // how many recent items are shown (posts, messages, files)

  // ── Tunnel doors ──────────────────────────────────────────────
  DOOR_WIDTH:             1.2,      // world units
  DOOR_HEIGHT:            2.8,      // world units

  // ── Portal transitions ────────────────────────────────────────
  PORTAL_TIMEOUT_MS:      10000,    // max wait for remote load
  PORTAL_REGARD_RADIUS:   3.0,     // world units — activation distance
  PORTAL_FLARE_DURATION:  0.3,     // seconds — burst on successful transition
}
```

---

## 3. Underground — Tunnel as Passage

### 3.1 Seed

The Underground is seeded from both peer identities, canonically ordered so both peers compute the same geometry:

```
tunnel_seed = ha(min(peer_id_A, peer_id_B) XOR max(peer_id_A, peer_id_B))
```

`min`/`max` comparison is byte-wise lexicographic on the 32-byte WireGuard public keys.

### 3.2 Geometry

The tunnel is a corridor connecting two Inside spaces. Its physical properties reflect the connection:

```
// Length from latency
tunnel_length = CONFIG.TUNNEL_BASE_LENGTH
              + latency_ms × CONFIG.TUNNEL_LENGTH_PER_MS
// A 20ms tunnel is short and quick. A 200ms tunnel is long and slow.
// Walking time roughly corresponds to network latency (perceptually, not literally).

// Width from bandwidth
tunnel_width = CONFIG.TUNNEL_MIN_WIDTH
             + clamp(bandwidth_kbps / CONFIG.TUNNEL_BANDWIDTH_REF, 0, 1)
               × (CONFIG.TUNNEL_MAX_WIDTH - CONFIG.TUNNEL_MIN_WIDTH)

// Height
tunnel_height = CONFIG.TUNNEL_BASE_HEIGHT
              + (active_capability_count / 10) × CONFIG.TUNNEL_HEIGHT_PER_CAP
// More shared capabilities = taller tunnel (richer connection)

// Cross-section shape from tunnel_seed
// Some tunnels are rectangular, some arched, some irregular
cross_section_seed = ha(tunnel_seed ^ 0xc055)
cross_section = CROSS_SECTIONS[cross_section_seed % CROSS_SECTIONS.length]
// rectangular | arched | rounded | irregular | hexagonal
```

### 3.3 Aesthetic Gradient

The tunnel's visual character transitions from one peer's aesthetic to the other's. At Alice's end, the tunnel looks like Alice's district. At Bob's end, it looks like Bob's. The midpoint is a blend.

```
aesthetic_A = derive_aesthetic(cell_key_A)    // Alice's district palette
aesthetic_B = derive_aesthetic(cell_key_B)    // Bob's district palette

// At any point along the tunnel:
t = position_along_tunnel / tunnel_length     // 0 = A's end, 1 = B's end

local_hue         = lerp(aesthetic_A.hue, aesthetic_B.hue, t)
local_popcount    = lerp(aesthetic_A.popcount_ratio, aesthetic_B.popcount_ratio, t)
local_age         = lerp(aesthetic_A.age, aesthetic_B.age, t)
local_temperature = lerp_temperature(aesthetic_A, aesthetic_B, t)
```

This gradient is applied to all description graphs in the tunnel — wall surfaces, fixtures, decorative elements. The tunnel is a liminal space where two digital identities meet.

### 3.4 Tunnel Contents

The tunnel is not empty. Its contents reflect the relationship between the two peers:

#### Wall Treatment

Tunnel walls carry the blended aesthetic. Surface texture and material shift with the gradient:

```
wall_entity(segment_index) {
  t = segment_index / total_segments

  being.surface.texture: lerp between A's district texture and B's
  being.material.substance: lerp between A's and B's
  being.form.detail: from blended popcount (denser connections = more detail)
}
```

#### Shared Capability Markers

Each mutually active capability in the P2P-CD intersection produces a visible feature along the tunnel:

```
for i, cap_name in active_set:
  marker_t = (i + 1) / (active_set.length + 1)    // evenly spaced along tunnel
  marker_position = tunnel_start + marker_t × tunnel_direction × tunnel_length

  marker_entity {
    archetype: "fixture:ornament"
    position: marker_position
    being.form.silhouette: form_from_capability_type(cap_name)
    being.surface.texture: from blended aesthetic at marker_t
    effect.emission.type: "glow"
    effect.emission.intensity: "subtle"
    effect.emission.channel: "both"
    relation.context.narrative: "herald"    // these markers announce what the connection carries
  }
```

Capabilities that only one peer has do not produce markers — the tunnel only reflects what's shared.

#### Illumination

Tunnel lighting is derived from connection health:

```
if connection_uptime > 0.9:   // stable tunnel
  light_intensity = "moderate"
  light_rhythm = "constant"

if connection_uptime 0.5–0.9:  // intermittent
  light_intensity = "subtle"
  light_rhythm = "flickering"

if connection_uptime < 0.5:    // unreliable
  light_intensity = "faint"
  light_rhythm = "sporadic"
```

A stable, long-lived tunnel is well-lit. A new or flaky tunnel flickers. The lighting tells you about the quality of the connection before you reach the other end.

### 3.5 Tunnel Endpoints

Each end of the tunnel connects to an Inside space:

```
// Alice's end
endpoint_A {
  position: aligned with Alice's tunnel door in her entry hall
  orientation: facing into the tunnel
  // Walking backward from here returns to Alice's Inside
}

// Bob's end
endpoint_B {
  position: aligned with Bob's tunnel door in his entry hall
  orientation: facing into the tunnel
  // Walking forward past here enters Bob's Inside
}
```

The transition from Inside to Underground (and back) is a spatial threshold — you walk through the door and the space changes. The renderer handles this as a scene transition: unload Inside entities, load Underground entities. The camera position is continuous; the scene content changes.

### 3.6 Underground Configuration

```
CONFIG = {
  // ── Tunnel dimensions ─────────────────────────────────────────
  TUNNEL_BASE_LENGTH:      10,       // world units — minimum tunnel length
  TUNNEL_LENGTH_PER_MS:    0.2,      // world units per ms of latency
  TUNNEL_MIN_WIDTH:        2.0,      // world units — narrowest tunnel
  TUNNEL_MAX_WIDTH:        6.0,      // world units — widest tunnel
  TUNNEL_BANDWIDTH_REF:    10000,    // kbps — bandwidth that produces max width
  TUNNEL_BASE_HEIGHT:      3.0,      // world units
  TUNNEL_HEIGHT_PER_CAP:   0.3,      // additional height per shared capability

  // ── Tunnel segments ───────────────────────────────────────────
  TUNNEL_SEGMENT_LENGTH:   4.0,      // world units per wall segment
  // Total segments = ceil(tunnel_length / TUNNEL_SEGMENT_LENGTH)
  // Each segment gets its own blended aesthetic

  // ── Cross-sections ────────────────────────────────────────────
  CROSS_SECTIONS: ["rectangular", "arched", "rounded", "irregular", "hexagonal"]
}
```

---

## 4. Presence Model

### 4.1 Outside — Peer-to-Peer

No host. Peers who share a cell and have direct WireGuard tunnels stream positions to each other via `howm.world.presence.1` (using `core.data.stream.1`). You only see peers you're directly connected to.

### 4.2 Inside — Host-Mediated

The peer whose Inside it is acts as presence host. When multiple visitors are in the same Inside, the host relays positions between them:

```
// Alice and Carol are both in Bob's Inside
// Bob receives positions from both via their respective tunnels
// Bob relays Alice's position to Carol and Carol's position to Alice

relay_presence(host, visitors):
  for visitor in visitors:
    for other in visitors where other != visitor:
      if host.has_tunnel(visitor) and host.has_tunnel(other):
        send_position(visitor.position, to: other, via: host)
```

This means visitors can see each other even without direct tunnels between them. Alice↔Carol may not have a tunnel, but they can both see each other in Bob's home because Bob relays.

The host does not need to be present in their own Inside for visitors to be there. The Inside exists as long as the host's node is running. Visitors can explore the home even when the host is Away (per `social.presence` status). They just won't see the host as a rendered entity.

### 4.3 Underground — 1-to-1

Only the two tunnel endpoints can be in the Underground. No relay, no third parties. The tunnel is a private passage. If Alice is walking through Alice↔Bob Underground, only Alice and Bob can be present in that space. If both happen to be in the tunnel simultaneously (Alice walking to Bob, Bob walking to Alice), they see each other. Otherwise, the tunnel is solitary.

---

## 5. Transitions

### 5.1 Portal Transitions

Every transition between spaces passes through a **portal** — a visible threshold entity that serves as both a door and a loading indicator. The portal is an HDL-described entity with its own appearance, emission, and sequences. The renderer doesn't know it's a loading screen — it renders the portal's description graph like any other entity.

#### Portal behaviour

```
portal_states:
  idle        → portal breathes faintly (default emission rhythm)
  activating  → player approaches, regard triggers, emission intensifies (0.2s)
  loading     → player crosses threshold, glow intensifies further,
                generator fetches destination entities in background
  ready       → destination loaded, portal flares and clears, player crosses
  timeout     → after CONFIG.PORTAL_TIMEOUT_MS, portal dims, player stays
  offline     → remote peer not reachable, portal dims immediately
```

#### Portal entity description

Every door (building entry, tunnel door, room doorway) carries a portal description graph:

```json
{
  "traits": [
    { "path": "being.form.silhouette", "term": "tall", "params": { "aspect": 0.3 } },
    { "path": "being.form.scale", "term": "moderate", "params": {} },
    { "path": "being.surface.texture", "term": "smooth", "params": { "reflectance": 0.4 } },
    { "path": "being.surface.opacity", "term": "shifting", "params": { "level": 0.6, "variance": 0.3 } },
    { "path": "being.material.substance", "term": "light", "params": { "luminance": 0.5, "saturation": 0.3 } },
    { "path": "effect.emission.type", "term": "glow", "params": { "radius": 3.0 } },
    { "path": "effect.emission.intensity", "term": "faint", "params": { "value": 0.2 } },
    { "path": "effect.emission.rhythm", "term": "breathing", "params": { "period": 3.0 } },
    { "path": "effect.emission.channel", "term": "both", "params": {} },
    { "path": "relation.regard.awareness", "term": "attentive", "params": { "radius": 3.0 } },
    { "path": "relation.regard.disposition", "term": "welcoming", "params": { "threshold": 0.5 } }
  ],
  "sequences": [
    {
      "trigger": { "path": "relation.regard", "event": "activated" },
      "effect": { "path": "effect.emission", "action": "intensify", "factor": 3.0 },
      "timing": { "delay": 0.0, "duration": null }
    },
    {
      "trigger": { "path": "relation.regard", "event": "deactivated" },
      "effect": { "path": "effect.emission", "action": "diminish", "factor": 0.3 },
      "timing": { "delay": 0.0, "duration": 1.0 }
    }
  ]
}
```

The portal is a shifting-opacity entity that breathes gently. When the player walks near it (regard activates), it intensifies — the door "wakes up." This happens before the player crosses — it's a visual invitation. The destination's aesthetic tint colours the portal's emission, hinting at where it leads.

#### Transition sequence

```
1. Player approaches portal (within regard radius)
   → Portal emission intensifies (sequence fires)
   → Portal tints toward destination aesthetic

2. Player crosses threshold (position passes door plane)
   → Generator begins loading destination:
     - Local transitions (own Inside, Underground): instant
     - Remote transitions (other's Inside): RPC to remote node
   → Portal glow flares to maximum
   → Current space entities held in memory (for fast return)

3a. Load succeeds
   → Portal clears (emission burst, then fade)
   → Previous space entities unloaded
   → Destination space entities loaded
   → Player camera continues smoothly into new space

3b. Load times out (CONFIG.PORTAL_TIMEOUT_MS, default 10s)
   → Portal dims and flickers
   → Player remains in current space
   → Portal returns to idle state

3c. Remote peer offline (known from presence data)
   → Portal is dark before approach (no breathing emission)
   → Crossing threshold immediately shows "unreachable" state
   → Player remains in current space
```

#### Transition types

| From | To | Portal character | Load method |
|---|---|---|---|
| Outside | Own Inside | Home entry door, own district tint | Local — instant |
| Outside | Peer's Inside | Home entry door, peer's district tint | RPC to peer's node |
| Inside | Outside | Building entry (reverse), sky tint | Local — instant |
| Inside | Underground | Tunnel door, blended tint | Local — instant |
| Underground | Peer's Inside | Tunnel endpoint, peer's district tint | RPC to peer's node |
| Inside | Capability room | Interior doorway, no portal effect (instant, no load) | Local — instant |

Interior doorways between rooms within the same Inside are the one exception — they don't need a portal because all rooms are loaded together. Walking between rooms is seamless.

#### Portal configuration

```
CONFIG = {
  PORTAL_TIMEOUT_MS:    10000,     // max wait for remote load
  PORTAL_REGARD_RADIUS: 3.0,      // world units — how close to activate
  PORTAL_FLARE_DURATION: 0.3,     // seconds — burst on successful transition
}
```

### 5.2 Camera Continuity

The camera position is continuous across transitions. The player's position in the new space corresponds to the portal's position in that space. The world changes around the player — the player doesn't teleport.

For Underground transitions, the player enters at one end and their position maps to the tunnel entrance. For Inside transitions, the player's position maps to just inside the entry hall at the corresponding doorway.

---

## 6. Inside Generation Pipeline

### 6.1 Overview

```
peer_id + peer_ip
  → inside_seed + aesthetic_palette                    // §2.1
  → room_list(installed_capabilities)                  // §2.2
  → room_layout(room_list, inside_seed)                // §2.5
  → for each room:
      room_geometry(room_type, area, height)            // §2.5
      room_entities(capability_state, aesthetic)        // §2.4
      room_access_filter(visitor, access_groups)        // §2.8
  → entry_hall_geometry(hall_area, hall_height)
  → tunnel_doors(active_connections, inside_seed)       // §2.6
  → DescriptionPacket[] for all entities
```

### 6.2 Entity Description Graphs

All Inside entities use the same HDL as Outside entities. The aesthetic palette from the peer's home district drives:

- `being.surface.texture` — district popcount determines surface complexity
- `being.material.substance` — district domain determines material vocabulary
- `being.material.temperature` — district domain determines warmth
- `being.form.detail` — district age determines weathering/detail
- Colour derivation — district hue drives the colour palette

This means Alice's feed room looks materially similar to Alice's Outside district. If Alice lives in a crystalline, cold, faceted district (low popcount, mineral substance), her feed room has crystalline display surfaces with cold blue-grey tones. If Bob lives in a warm, organic, rough district (high popcount, organic substance), Bob's feed room has rough wooden display surfaces with amber tones.

---

## 7. Underground Generation Pipeline

### 7.1 Overview

```
peer_id_A + peer_id_B
  → tunnel_seed                                        // §3.1
  → aesthetic_A, aesthetic_B                           // §3.3
  → tunnel_dimensions(latency, bandwidth, active_set)  // §3.2
  → tunnel_segments(length, segment_length)
  → for each segment:
      t = segment_position / tunnel_length
      blended_aesthetic = lerp(aesthetic_A, aesthetic_B, t)
      wall_entities(segment, blended_aesthetic)          // §3.4
  → capability_markers(active_set, tunnel_length)       // §3.4
  → lighting(connection_uptime)                         // §3.4
  → DescriptionPacket[] for all entities
```

### 7.2 Tunnel Geometry

The tunnel is generated as a sequence of wall, floor, and ceiling segments, each carrying its own blended aesthetic. The renderer receives these as standard building-like entities with footprint polygons and heights.

```
segment_count = ceil(tunnel_length / CONFIG.TUNNEL_SEGMENT_LENGTH)

for i in 0..segment_count:
  t = i / segment_count
  segment_aesthetic = lerp(aesthetic_A, aesthetic_B, t)

  // Floor
  floor_entity {
    archetype: "building:block"
    footprint: rectangle(tunnel_width, CONFIG.TUNNEL_SEGMENT_LENGTH)
    height: 0.1    // thin floor slab
    being.surface: from segment_aesthetic
    being.material: from segment_aesthetic
  }

  // Walls (left and right)
  wall_entity {
    archetype: "building:block"
    footprint: rectangle(0.3, CONFIG.TUNNEL_SEGMENT_LENGTH)
    height: tunnel_height
    being.surface: from segment_aesthetic
    being.material: from segment_aesthetic
  }

  // Ceiling
  // Shape depends on cross_section type
  if cross_section == "arched":
    ceiling = dome-like entity
  else:
    ceiling = flat slab entity
```

---

## 8. Multiplayer in Inside

### 8.1 Host Relay Protocol

When visitors are present in a host's Inside, the host relays presence data:

```
// Using howm.world.presence.1 over core.data.stream.1

// Each visitor streams their position to the host
visitor → host: { position, orientation, velocity }  // 2-4 Hz

// Host relays each visitor's position to all other visitors
host → each_other_visitor: { visitor_peer_id, position, orientation }  // 2-4 Hz
```

The host's relay is simple fan-out — for N visitors, the host sends N-1 position updates per incoming position. With 5 visitors at 4 Hz, that's 5 × 4 × 4 = 80 small packets per second. Negligible on a local WireGuard network.

### 8.2 Visitor Rendering

Each visitor is rendered as an entity with a description graph — their avatar. When a visitor enters someone's Inside, the host requests their avatar description:

```
RPC: howm.world.avatar.get { peer_id }
  → { description_graph: DescriptionGraph }
```

The avatar description graph uses the same HDL as any entity. A visitor is just another `DescribedEntity` in the scene. The renderer doesn't know it's a human — it renders the description.

Avatar description graphs are a future specification. For now, a default avatar is derived from the visitor's peer_id:

```
default_avatar(peer_id):
  avatar_seed = ha(peer_id ^ 0xface)
  being.form.silhouette: "tall"
  being.form.scale: "moderate"
  being.surface.texture: from visitor's home district aesthetic
  being.material.substance: from visitor's home district aesthetic
  behavior.motion.method: "continuous"
  effect.emission.type: "glow"
  effect.emission.intensity: "faint"
  effect.emission.channel: "background"
```

---

## 9. Open Questions

| # | Question | Status |
|---|---|---|
| OQ-S1 | Avatar system: how do players customise their appearance? HDL-based avatar editor? | Future spec |
| OQ-S2 | Inside customisation: can peers rearrange rooms, choose materials, place decorative objects? | Future — for now, generated from capability state + district aesthetic |
| OQ-S3 | Group spaces: shared Underground for peer groups (howm-access groups)? | Future — architecture supports it via group_id seed |
| OQ-S4 | Inside persistence: when a capability's state changes (new post, new message), how quickly do visitors see the change? Real-time via events or on next visit? | Open — real-time via `core.data.event.1` is preferred |
| OQ-S5 | Tunnel path: should the tunnel be a straight corridor or a curved/winding path derived from the tunnel_seed? | Straight for now, seed-derived curves later |
| OQ-S6 | Offline visiting: can you enter a peer's Inside when their node is down? | No — the RPC to describe the Inside requires the host's node |
| OQ-S7 | Home placement collision: two peers' positions overlap in the same cell. | **Closed** — overlapping SDFs blend naturally. Extremely rare in continuous space. |
| OQ-S8 | Transition between spaces: loading delay for remote peers. | **Closed** — portal transition model (§5.1). Portal entity with HDL-described loading states. Timeout at 10s. |
| OQ-S9 | Maximum visitors per Inside: should there be a cap? Host relay cost scales with N². | Open — start uncapped, monitor performance |
| OQ-S10 | Tunnel doors for offline peers: should they eventually disappear, or persist indefinitely? | Persist — the tunnel exists as long as the WireGuard config does |

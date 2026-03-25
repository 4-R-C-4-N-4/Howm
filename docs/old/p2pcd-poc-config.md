# P2P-CD Proof of Concept — Social Feed

Peer configuration reference for Rust implementation over WireGuard.
Implements P2P-CD-01 v0.3.

---

## 1. WireGuard Integration Overview

WireGuard provides three of the protocol's requirements out of the box, which eliminates the need for separate identity, authentication, and transport layers:

| P2P-CD Requirement | WireGuard Provides | Implication |
|--------------------|--------------------|-------------|
| **Peer identity** (`peer_id`) | Curve25519 static public key (32 bytes) | The WireGuard public key IS the `peer_id`. No separate identity keypair needed. |
| **Mutual authentication** (HANDSHAKE state) | Noise IK handshake with static keys | WireGuard tunnel establishment satisfies the HANDSHAKE → CAPABILITY_EXCHANGE transition. Both peers are cryptographically verified before any P2P-CD message is sent. |
| **Encrypted transport** | ChaCha20-Poly1305 authenticated encryption | All P2P-CD messages travel inside the WireGuard tunnel. No TLS, no application-layer encryption needed. |
| **Replay protection** (transport-level) | Built-in replay counter per session | WireGuard's transport-layer replay protection supplements (does not replace) P2P-CD's `sequence_num` replay detection, which operates at the manifest level. |
| **Peer reachability** | Handshake success/failure, persistent keepalive | WireGuard peer state changes can drive PEER_VISIBLE transitions. A successful handshake = PEER_VISIBLE. |

### What WireGuard does NOT provide

| P2P-CD Requirement | Still needed |
|--------------------|-------------|
| Capability discovery and negotiation | The core of P2P-CD — what each peer can do, role matching, trust gates |
| Per-capability access control | Trust gates, classification tiers, CONFIRM reconciliation |
| Session lifecycle management | State machine, rebroadcast, auto-deny |
| Application-level liveness | Heartbeat (core.session.heartbeat.1) — WireGuard keepalive is transport-level only, it doesn't confirm the application is still functioning |

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Application Layer                                       │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐ │
│  │ social.post  │  │ social.feed  │  │   heartbeat   │ │
│  │  PROVIDE /   │  │  CONSUME /   │  │  BOTH/mutual  │ │
│  │  CONSUME     │  │  PROVIDE     │  │               │ │
│  └──────┬───────┘  └──────┬───────┘  └───────┬───────┘ │
│         │                 │                   │         │
│  ┌──────┴─────────────────┴───────────────────┴──────┐  │
│  │              P2P-CD Protocol Engine                │  │
│  │  • CBOR manifest encoding (deterministic)         │  │
│  │  • 4-message OFFER/CONFIRM exchange               │  │
│  │  • Role intersection + trust gate evaluation      │  │
│  │  • Peer cache + auto-deny                         │  │
│  │  • Rebroadcast on state change                    │  │
│  └──────────────────────┬────────────────────────────┘  │
│                         │                               │
│  ┌──────────────────────┴────────────────────────────┐  │
│  │              WireGuard Interface (wg0)            │  │
│  │  • peer_id = WireGuard public key (32 bytes)      │  │
│  │  • HANDSHAKE = WireGuard Noise IK handshake       │  │
│  │  • Transport = UDP inside WG tunnel               │  │
│  │  • PEER_VISIBLE = WG handshake succeeded          │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Message flow inside the WireGuard tunnel

P2P-CD messages are sent as length-prefixed CBOR over a TCP connection established between the two peers' WireGuard interface addresses. The TCP connection runs *inside* the encrypted WireGuard tunnel:

```
Peer A (wg0: 10.0.0.1)  ←──WireGuard tunnel──→  Peer B (wg0: 10.0.0.2)
         │                                                │
    TCP connect to 10.0.0.2:7654                    TCP listen on :7654
         │                                                │
    [4-byte len][CBOR OFFER]  ──────────────────→         │
         │                    ←──────────────────  [4-byte len][CBOR OFFER]
    [4-byte len][CBOR CONFIRM] ─────────────────→         │
         │                    ←──────────────────  [4-byte len][CBOR CONFIRM]
         │                                                │
    ACTIVE ──── application data (GET/response) ──── ACTIVE
```

---

## 3. Capability Definitions

Two capabilities are defined under the organization namespace `p2pcd.social`. Both use the namespace grammar from §4.4 of the spec.

### 3.1 p2pcd.social.post.1

| Field | Value |
|-------|-------|
| Full name | `p2pcd.social.post.1` |
| Purpose | Publish the local peer's posts to the network. A peer that PROVIDEs this capability is making its posts available for consumption by matched peers. |
| Allowed roles | PROVIDE — the peer serves its posts to others; CONSUME — the peer reads posts from a specific remote peer; BOTH — the peer both publishes and reads (typical for mutual exchange) |
| Mutual | false (asymmetric provider/consumer relationship) |
| Applicable scope params | rate_limit (requests/sec for GET calls), ttl (session duration) |
| Wire protocol | Request-response. Consumer sends GET; provider returns a page of posts as a CBOR array. |

### 3.2 p2pcd.social.feed.1

| Field | Value |
|-------|-------|
| Full name | `p2pcd.social.feed.1` |
| Purpose | Aggregate and display posts from the network. A peer that CONSUMEs this capability is pulling posts from remote peers who PROVIDE `p2pcd.social.post.1`. |
| Allowed roles | CONSUME — the peer pulls aggregated feed content; PROVIDE — the peer serves aggregated feed (e.g., a relay/aggregator node); BOTH — the peer both aggregates and serves (relay that also reads) |
| Mutual | false |
| Applicable scope params | rate_limit (poll frequency), ttl (session duration) |
| Wire protocol | Request-response. Consumer sends GET with optional filters (since_timestamp, peer_id_filter); provider returns a page of posts. |

> **How post and feed relate:** `social.post` is the individual-peer publishing capability. `social.feed` is the aggregation capability. In the simplest deployment (no relay nodes), every peer PROVIDEs `social.post` and CONSUMEs `social.post` from other peers directly — no `social.feed` needed. The `social.feed` capability becomes relevant when a peer acts as an aggregator/relay that collects posts from many peers and serves them as a combined feed. For the initial proof of concept, `social.post` with PROVIDE/CONSUME is sufficient.

---

## 4. Classification Tiers

Three classification tiers are defined for the social feed application. These map to the trust gate system in §6 of the spec.

| Tier | Spec mapping | Behavior |
|------|-------------|----------|
| `PUBLIC` | UNRESTRICTED | Any peer with a valid WireGuard tunnel can access. Trust gate always returns ALLOW. |
| `FRIENDS` | Implementation-defined | Only peers whose WireGuard public key appears in the local friends list can access. Trust gate checks public key membership before returning ALLOW. |
| `BLOCKED` | DENIED | Peer is explicitly blocked. Trust gate always returns DENY. Equivalent to the built-in DENIED tier. |

Classification is applied independently per capability. A peer can be PUBLIC for `social.post` (anyone with a WireGuard tunnel can read my posts) while being FRIENDS for `social.feed` (only friends can pull my aggregated feed).

---

## 5. Peer Configuration Schema

Each peer is configured with a TOML file that drives manifest generation. The WireGuard configuration is separate (standard `wg0.conf`) — the P2P-CD config references it.

```toml
# ─── p2pcd-peer.toml ───

[identity]
# peer_id is derived from the WireGuard public key at runtime.
# This points to the WireGuard private key file, from which we derive the public key.
wireguard_private_key_file = "/etc/wireguard/private.key"
# Alternatively, specify the WireGuard interface to read the key from:
# wireguard_interface = "wg0"
display_name = "alice"                  # human-readable, not transmitted in protocol

[protocol]
version = 1                             # protocol_version field in manifest
hash_algorithm = "sha-256"              # IANA registered name

[transport]
# P2P-CD messages are sent over TCP inside the WireGuard tunnel.
listen_port = 7654                      # TCP port on the WireGuard interface
wireguard_interface = "wg0"             # which WG interface to bind to

[discovery]
# How to detect new WireGuard peers becoming reachable.
mode = "wireguard"                      # "wireguard" | "mdns" | "manual"
# wireguard mode: poll WG interface for handshake state changes.
# A peer whose latest handshake timestamp changes from 0 or advances → PEER_VISIBLE.
poll_interval_ms = 2000                 # how often to check WG peer state
# Optional: also broadcast on mDNS for peers not yet in WG config
mdns_fallback = false
broadcast_full_manifest = false         # lightweight discovery (hash only) — WG tunnel
                                        # is already authenticated, full manifest goes
                                        # in the OFFER after tunnel is up

# ─── Capability: social.post ───
# "I am publishing my posts to the network"
[capabilities.social_post]
name = "p2pcd.social.post.1"
role = "provide"                        # "provide" | "consume" | "both"
mutual = false

[capabilities.social_post.scope]
rate_limit = 10                         # max 10 GET requests/sec from any single peer
ttl = 3600                              # session lives for 1 hour before renegotiation

[capabilities.social_post.classification]
# Who can read my posts?
default_tier = "public"                 # "public" | "friends" | "blocked"
# Override for specific peers (WireGuard public key, base64-encoded)
# [capabilities.social_post.classification.overrides]
# "hFn3Kx..." = "friends"
# "jQ9mRp..." = "blocked"

# ─── Capability: social.feed ───
# "I want to read posts from the network"
[capabilities.feed]
name = "p2pcd.social.feed.1"
role = "consume"                        # this peer reads, doesn't aggregate/serve
mutual = false

[capabilities.feed.scope]
rate_limit = 5                          # I'll poll each peer at most 5 times/sec
ttl = 3600

[capabilities.feed.classification]
# Whose posts am I willing to read?
default_tier = "public"                 # accept posts from anyone with a WG tunnel
# [capabilities.feed.classification.overrides]
# "xYz123..." = "blocked"              # block specific peers

# ─── Capability: heartbeat (mandatory) ───
[capabilities.heartbeat]
name = "core.session.heartbeat.1"
role = "both"
mutual = true

[capabilities.heartbeat.params]
interval_ms = 5000                      # ping every 5 seconds
timeout_ms = 15000                      # 3 missed pings = session failure

# ─── Friends List ───
# WireGuard public keys (base64-encoded, same format as wg0.conf)
[friends]
list = [
    # "hFn3KxQ4bG7pLmN8vR2sT5wY9zA1cE3fH6jK8mP0qU=",
    # "jQ9mRpS2uW4xZ6bD8eG0iK3lN5oQ7rT1vX3yA5cF8h=",
]
```

### Companion WireGuard configuration

The standard WireGuard config (`/etc/wireguard/wg0.conf`) is managed separately. Each P2P-CD peer must be a WireGuard peer:

```ini
# /etc/wireguard/wg0.conf (Alice's node)

[Interface]
PrivateKey = <alice_private_key>
Address = 10.0.0.1/24
ListenPort = 51820

[Peer]
# Bob
PublicKey = hFn3KxQ4bG7pLmN8vR2sT5wY9zA1cE3fH6jK8mP0qU=
AllowedIPs = 10.0.0.2/32
Endpoint = bob.example.com:51820
PersistentKeepalive = 25

[Peer]
# Carol
PublicKey = jQ9mRpS2uW4xZ6bD8eG0iK3lN5oQ7rT1vX3yA5cF8h=
AllowedIPs = 10.0.0.3/32
Endpoint = carol.example.com:51820
PersistentKeepalive = 25
```

> **Key correspondence:** The WireGuard `PublicKey` in `wg0.conf` is the same value used as `peer_id` in P2P-CD manifests and as keys in the friends list. A single namespace for identity across both layers.

---

## 6. Peer Archetypes

Four deployment archetypes cover the proof-of-concept scenarios:

### 6.1 Normal User (read + write, public)

```
capabilities:
  social.post   → role: PROVIDE,  classification: PUBLIC,  rate_limit: 10
  social.feed   → role: CONSUME,  classification: PUBLIC
  heartbeat     → role: BOTH,     mutual: true
```

Posts are visible to any peer with a WireGuard tunnel. Reads posts from everyone. The most common config.

### 6.2 Private User (read + write, friends only)

```
capabilities:
  social.post   → role: PROVIDE,  classification: FRIENDS, rate_limit: 5
  social.feed   → role: CONSUME,  classification: FRIENDS
  heartbeat     → role: BOTH,     mutual: true
```

Posts only visible to friends (peers whose WireGuard public key is in the friends list). Only reads posts from friends. Trust gate checks public key against friends list before ALLOW.

### 6.3 Lurker (read only)

```
capabilities:
  social.feed   → role: CONSUME,  classification: PUBLIC
  heartbeat     → role: BOTH,     mutual: true
```

No social.post capability advertised. This peer never appears as a provider. Reads posts from everyone.

### 6.4 Broadcast-only (write only)

```
capabilities:
  social.post   → role: PROVIDE,  classification: PUBLIC,  rate_limit: 20
  heartbeat     → role: BOTH,     mutual: true
```

No social.feed capability advertised. Publishes but never consumes. Useful for announcement bots.

---

## 7. State Machine with WireGuard

How the P2P-CD states map to WireGuard events:

```
WireGuard Event                          P2P-CD Transition
─────────────────────────────────────    ────────────────────────────────
WG peer added to config                  (no transition — peer exists but
                                          no handshake yet)

WG handshake succeeds (latest_handshake  PEER_VISIBLE
timestamp changes from 0 or advances)     │
                                          ├─ cache check: hash match +
                                          │  last_outcome=NONE → DENIED
                                          │
                                          └─ no cache hit or hash changed
                                             → open TCP to peer:7654
                                             → HANDSHAKE (trivial — WG
                                               already authenticated)
                                             → CAPABILITY_EXCHANGE
                                             → OFFER / OFFER / CONFIRM / CONFIRM
                                             → ACTIVE or NONE

WG handshake timeout / peer unreachable  If ACTIVE → ungraceful disconnect
                                          → CLOSED → PEER_VISIBLE
                                          (wait for WG to re-handshake)

WG peer removed from config              CLOSED → peer forgotten
```

> **The HANDSHAKE state is nearly instantaneous.** Since WireGuard has already authenticated both peers, the "HANDSHAKE" in P2P-CD terms is just the TCP connection setup over the WireGuard interface. There's no additional authentication step. The protocol can transition directly from "WireGuard tunnel up" to "send OFFER."

---

## 8. Manifest Example — CBOR Diagnostic Notation

This is what the Normal User archetype looks like as a CBOR-encoded OFFER message on the wire. The `peer_id` is Alice's WireGuard Curve25519 public key (32 bytes).

```cbor-diag
/ OFFER message /
{
  1: 1,                                 / message_type: offer /
  2: {                                  / discovery_manifest /
    1: 1,                               / protocol_version /
    2: h'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6
         a7b8c9d0e1f2a3b4c5d6a1b2c3d4e5f6', / peer_id: 32-byte WG public key /
    3: 1,                               / sequence_num /
    4: [                                / capabilities (sorted by name) /
      {                                 / core.session.heartbeat.1 /
        1: "core.session.heartbeat.1",
        2: 3,                           / role: both /
        3: true,                        / mutual: true /
      },
      {                                 / p2pcd.social.feed.1 /
        1: "p2pcd.social.feed.1",
        2: 2,                           / role: consume /
        5: {                            / scope /
          1: 5,                         / rate_limit: 5 req/s /
          2: 3600,                      / ttl: 1 hour /
        },
      },
      {                                 / p2pcd.social.post.1 /
        1: "p2pcd.social.post.1",
        2: 1,                           / role: provide /
        5: {                            / scope /
          1: 10,                        / rate_limit: 10 req/s /
          2: 3600,                      / ttl: 1 hour /
        },
      },
    ],
    5: h'e3b0c44298fc1c149afbf4c8996fb924
         27ae41e4649b934ca495991b7852b855', / personal_hash (sha-256, 32 bytes) /
    6: "sha-256",                       / hash_algorithm /
  },
}
```

> **Note:** Capabilities are sorted lexicographically by name. Classification is omitted from the wire — trust gates are evaluated locally using the WireGuard public key as the lookup key.

---

## 9. Intersection Scenarios

How the four archetypes interact when they discover each other:

### 9.1 Normal User ↔ Normal User

```
Alice (wg pubkey: A...): social.post/PROVIDE + social.feed/CONSUME + heartbeat/BOTH
Bob   (wg pubkey: B...): social.post/PROVIDE + social.feed/CONSUME + heartbeat/BOTH

Intersection:
  Alice.social.post/PROVIDE  ↔  Bob.social.feed/CONSUME    → match (Alice serves Bob)
  Bob.social.post/PROVIDE    ↔  Alice.social.feed/CONSUME   → match (Bob serves Alice)
  heartbeat/BOTH             ↔  heartbeat/BOTH (mutual:true) → match

Alice CONFIRM active_set: [core.session.heartbeat.1,
                           p2pcd.social.feed.1,
                           p2pcd.social.post.1]
Bob   CONFIRM active_set: [core.session.heartbeat.1,
                           p2pcd.social.feed.1,
                           p2pcd.social.post.1]

Result: ACTIVE with 3 capabilities. Bidirectional post exchange.
```

### 9.2 Normal User ↔ Lurker

```
Alice:  social.post/PROVIDE + social.feed/CONSUME + heartbeat/BOTH
Lurker: social.feed/CONSUME + heartbeat/BOTH

Intersection:
  Alice.social.post/PROVIDE  ↔  Lurker.social.feed/CONSUME  → match
  Alice.social.feed/CONSUME  ↔  (nothing from Lurker)        → no match
  heartbeat/BOTH             ↔  heartbeat/BOTH               → match

Alice CONFIRM: [core.session.heartbeat.1, p2pcd.social.post.1]
Lurker CONFIRM: [core.session.heartbeat.1, p2pcd.social.post.1]

Result: ACTIVE with 2 capabilities. Lurker reads Alice's posts.
```

### 9.3 Private User ↔ Non-Friend Normal User

```
Private (friends=[F...]): social.post/PROVIDE (FRIENDS) + social.feed/CONSUME (FRIENDS) + heartbeat/BOTH
Stranger (wg pubkey: S...): social.post/PROVIDE (PUBLIC) + social.feed/CONSUME (PUBLIC) + heartbeat/BOTH

WireGuard tunnel is up — both peers are authenticated at the transport layer.
But Private's trust gate checks: is S... in friends list? → NO

  Private's trust gate for social.post: S... ∉ friends → DENY
  Private's trust gate for social.feed: S... ∉ friends → DENY
  Heartbeat has no trust gate (always ALLOW)

Private CONFIRM: [core.session.heartbeat.1]
Stranger CONFIRM: [core.session.heartbeat.1,
                   p2pcd.social.feed.1,
                   p2pcd.social.post.1]

Reconciliation: intersection = [core.session.heartbeat.1]

Result: ACTIVE with 1 capability (heartbeat only). No post exchange.
  WireGuard authenticated the peer, but the P2P-CD trust gate
  enforced the friends-only policy at the capability layer.
```

### 9.4 Private User ↔ Friend Normal User

```
Private (friends=[F...]): social.post/PROVIDE (FRIENDS) + social.feed/CONSUME (FRIENDS) + heartbeat/BOTH
Friend  (wg pubkey: F...): social.post/PROVIDE (PUBLIC) + social.feed/CONSUME (PUBLIC) + heartbeat/BOTH

Private's trust gate: F... ∈ friends → ALLOW for both capabilities

Private CONFIRM: [core.session.heartbeat.1, p2pcd.social.feed.1,
                  p2pcd.social.post.1]
Friend CONFIRM:  [core.session.heartbeat.1, p2pcd.social.feed.1,
                  p2pcd.social.post.1]

Result: ACTIVE with 3 capabilities. Full bidirectional post exchange.
```

### 9.5 Lurker ↔ Lurker

```
LurkerA: social.feed/CONSUME + heartbeat/BOTH
LurkerB: social.feed/CONSUME + heartbeat/BOTH

Intersection:
  social.feed/CONSUME ↔ social.feed/CONSUME → no match (CONSUME+CONSUME)
  heartbeat/BOTH ↔ heartbeat/BOTH → match

Both CONFIRM: [core.session.heartbeat.1]

Result: ACTIVE with 1 capability (heartbeat only).
  Two lurkers have nothing to offer each other. Implementation MAY close.
```

---

## 10. Rust Implementation Jumpstart

The following type definitions map directly to the CDDL schemas in §5.3 of the spec. These are the core data structures for the Rust proof-of-concept.

```rust
// ─── p2pcd-types/src/lib.rs ───

use serde::{Deserialize, Serialize};

/// Protocol version. MUST be 1 for P2P-CD-01 v0.3.
pub const PROTOCOL_VERSION: u64 = 1;

/// WireGuard Curve25519 public key length in bytes.
pub const PEER_ID_LEN: usize = 32;

/// Type alias for peer identity (WireGuard public key).
pub type PeerId = [u8; PEER_ID_LEN];

/// CBOR integer map keys for discovery_manifest
pub mod manifest_keys {
    pub const PROTOCOL_VERSION: u64 = 1;
    pub const PEER_ID: u64 = 2;
    pub const SEQUENCE_NUM: u64 = 3;
    pub const CAPABILITIES: u64 = 4;
    pub const PERSONAL_HASH: u64 = 5;
    pub const HASH_ALGORITHM: u64 = 6;
}

/// CBOR integer map keys for capability_declaration
pub mod capability_keys {
    pub const NAME: u64 = 1;
    pub const ROLE: u64 = 2;
    pub const MUTUAL: u64 = 3;
    pub const CLASSIFICATION: u64 = 4;
    pub const SCOPE: u64 = 5;
}

/// CBOR integer map keys for scope_params
pub mod scope_keys {
    pub const RATE_LIMIT: u64 = 1;
    pub const TTL: u64 = 2;
}

// ─── Wire message types ───

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Offer = 1,
    Confirm = 2,
    Close = 3,
    Ping = 4,
    Pong = 5,
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    Normal = 0,
    NoMatch = 1,
    AuthFailure = 2,
    VersionUnsupported = 3,
    Timeout = 4,
    Error = 255,
}

// ─── Role enum ───

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Provide = 1,
    Consume = 2,
    Both = 3,
}

impl Role {
    /// Returns true if two roles produce a match per §7.4 intersection rules.
    pub fn matches(&self, other: &Role, self_mutual: bool, other_mutual: bool) -> bool {
        use Role::*;
        match (self, other) {
            (Provide, Consume) | (Consume, Provide) => true,
            (Both, Provide) | (Provide, Both) => true,
            (Both, Consume) | (Consume, Both) => true,
            (Both, Both) => self_mutual && other_mutual,
            (Provide, Provide) | (Consume, Consume) => false,
        }
    }
}

// ─── Classification tiers (application-level) ───

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassificationTier {
    /// Maps to UNRESTRICTED. Any peer with a valid WireGuard tunnel.
    Public,
    /// Implementation-defined. Peer's WG public key must be in friends list.
    Friends,
    /// Maps to DENIED. Peer is blocked.
    Blocked,
}

// ─── Scope parameters ───

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeParams {
    /// Requests per second, 0 = unlimited
    pub rate_limit: u64,
    /// Session TTL in seconds, 0 = no expiry
    pub ttl: u64,
}

impl ScopeParams {
    /// Reconcile two scope params per §7.3: most-restrictive-wins.
    pub fn reconcile(&self, other: &ScopeParams) -> ScopeParams {
        ScopeParams {
            rate_limit: match (self.rate_limit, other.rate_limit) {
                (0, x) | (x, 0) => x,  // 0 = unlimited, so take the other
                (a, b) => a.min(b),
            },
            ttl: match (self.ttl, other.ttl) {
                (0, x) | (x, 0) => x,  // 0 = no expiry, so take the other
                (a, b) => a.min(b),
            },
        }
    }
}

// ─── Capability declaration ───

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDeclaration {
    /// Fully qualified name per §4.4 namespace grammar
    pub name: String,
    pub role: Role,
    /// Required for BOTH+BOTH matching
    pub mutual: bool,
    /// Scope constraints advertised to remote peers
    pub scope: Option<ScopeParams>,
}

// ─── Discovery manifest ───

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryManifest {
    pub protocol_version: u64,
    /// WireGuard Curve25519 public key (32 bytes)
    pub peer_id: PeerId,
    pub sequence_num: u64,
    /// MUST be sorted lexicographically by name before serialization
    pub capabilities: Vec<CapabilityDeclaration>,
    pub personal_hash: Vec<u8>,
    pub hash_algorithm: String,
}

impl DiscoveryManifest {
    /// Ensure capabilities are sorted per §4.5 requirement.
    pub fn sort_capabilities(&mut self) {
        self.capabilities.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

// ─── Protocol messages ───

#[derive(Debug, Clone)]
pub enum ProtocolMessage {
    Offer {
        manifest: DiscoveryManifest,
    },
    Confirm {
        personal_hash: Vec<u8>,
        active_set: Vec<String>,       // sorted capability names
        accepted_params: Option<std::collections::BTreeMap<String, ScopeParams>>,
    },
    Close {
        personal_hash: Vec<u8>,
        reason: CloseReason,
    },
    Ping {
        timestamp: u64,
    },
    Pong {
        timestamp: u64,
    },
}

// ─── Trust gate (application-level) ───

/// Local peer configuration for trust gate evaluation.
/// Uses WireGuard public keys for peer identification.
#[derive(Debug, Clone)]
pub struct TrustPolicy {
    /// Default tier applied to unknown peers for a given capability
    pub default_tier: ClassificationTier,
    /// Per-peer overrides: WG public key -> tier
    pub overrides: std::collections::HashMap<PeerId, ClassificationTier>,
    /// Friends list: set of WireGuard public keys
    pub friends: std::collections::HashSet<PeerId>,
}

impl TrustPolicy {
    /// Evaluate trust gate for a specific peer and capability.
    /// remote_peer_id is the WireGuard public key of the remote peer.
    /// Returns true for ALLOW, false for DENY.
    pub fn evaluate(&self, remote_peer_id: &PeerId) -> bool {
        // Check explicit overrides first
        if let Some(tier) = self.overrides.get(remote_peer_id) {
            return match tier {
                ClassificationTier::Public => true,
                ClassificationTier::Friends => self.friends.contains(remote_peer_id),
                ClassificationTier::Blocked => false,
            };
        }
        // Apply default tier
        match self.default_tier {
            ClassificationTier::Public => true,
            ClassificationTier::Friends => self.friends.contains(remote_peer_id),
            ClassificationTier::Blocked => false,
        }
    }
}

// ─── Intersection computation (§7.4) ───

/// Compute the active set from two manifests + local trust policy.
/// Returns sorted list of capability names that matched.
pub fn compute_intersection(
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
    trust_policies: &std::collections::HashMap<String, TrustPolicy>,
) -> Vec<String> {
    let mut active = Vec::new();

    for local_cap in &local.capabilities {
        for remote_cap in &remote.capabilities {
            if local_cap.name != remote_cap.name {
                continue;
            }
            // Role match check
            if !local_cap.role.matches(
                &remote_cap.role,
                local_cap.mutual,
                remote_cap.mutual,
            ) {
                continue;
            }
            // Trust gate check (uses remote manifest's peer_id = WG public key)
            if let Some(policy) = trust_policies.get(&local_cap.name) {
                if !policy.evaluate(&remote.peer_id) {
                    continue;
                }
            }
            // Match found
            active.push(local_cap.name.clone());
            break;
        }
    }

    active.sort();
    active
}

// ─── WireGuard peer state monitoring ───

/// Represents a WireGuard peer's state as observed from the local interface.
#[derive(Debug, Clone)]
pub struct WgPeerState {
    pub public_key: PeerId,
    pub endpoint: Option<std::net::SocketAddr>,
    pub allowed_ips: Vec<String>,
    /// Timestamp of last successful handshake (0 = never)
    pub latest_handshake: u64,
    /// Bytes received since last check (used for liveness heuristic)
    pub rx_bytes: u64,
}

impl WgPeerState {
    /// Returns true if the peer has completed a WireGuard handshake.
    pub fn is_reachable(&self) -> bool {
        self.latest_handshake > 0
    }
}
```

---

## 11. Recommended Rust Crates

| Crate | Purpose | Notes |
|-------|---------|-------|
| `ciborium` | CBOR encode/decode | Pure Rust, supports deterministic encoding. Use `ciborium::ser` with `into_writer` for deterministic output. |
| `coset` | COSE structures | RFC 9052 implementation. Built on `ciborium`. Needed for Full conformance credential verification. |
| `sha2` | SHA-256 hashing | For personal_hash computation. RustCrypto family. |
| `x25519-dalek` | Curve25519 key ops | For deriving the WireGuard public key from private key if needed. The WG public key is the `peer_id`. |
| `tokio` | Async runtime | For concurrent session management, heartbeat timers, WG state polling. |
| `tokio::net::TcpStream` | TCP inside WG tunnel | P2P-CD messages sent over TCP connections on the WireGuard interface. No separate transport crate needed. |
| `toml` | Config parsing | For reading the peer configuration file. |
| `serde` | Serialization | Derive macros for config structs. CBOR wire encoding uses `ciborium` directly with integer keys. |
| `base64` | Key encoding | For parsing WireGuard public keys from base64 config format to raw bytes. |

> **Important:** The serde `Serialize`/`Deserialize` derives on the type stubs above are for the config layer (TOML) and internal use. For wire encoding, you MUST use `ciborium` directly to produce CBOR maps with integer keys as specified in §5.3. Implement `to_cbor(&self) -> Vec<u8>` methods that manually construct the CBOR map with the correct integer keys.

> **WireGuard interface interaction:** To poll WireGuard peer state, the implementation can either shell out to `wg show wg0 dump` and parse the tab-separated output, or use the `wireguard-control` crate if available. The dump format provides public key, endpoint, allowed IPs, latest handshake timestamp, and transfer bytes per peer — everything needed for PEER_VISIBLE detection.

---

## 12. Implementation Milestones

Suggested order for building the proof of concept:

| # | Milestone | What it proves |
|---|-----------|---------------|
| 1 | CBOR manifest encoding + personal hash | Two peers produce identical hashes from the same config. Deterministic encoding works. |
| 2 | WireGuard peer state monitor | Detect when WG peers become reachable. Map handshake events to PEER_VISIBLE. |
| 3 | TCP connection over WG + OFFER exchange | Two peers send/receive CBOR-encoded manifests over TCP inside the WireGuard tunnel. |
| 4 | Intersection + CONFIRM reconciliation | Two Normal Users complete the four-message exchange and agree on the active set. |
| 5 | Heartbeat (PING/PONG) | Session liveness works at application layer (independent of WG keepalive). |
| 6 | Trust gate with friends list | Private User ↔ Stranger produces heartbeat-only. Private User ↔ Friend produces full exchange. Friends list uses WG public keys. |
| 7 | social.post GET request/response | A consumer requests and receives posts from a provider over an ACTIVE session. |
| 8 | Rebroadcast on capability change | Adding/removing a peer from friends list triggers renegotiation with live sessions. |
| 9 | Auto-deny with peer cache | NONE outcomes are cached. Same-hash peers skip TCP connect. Hash change triggers retry. |

---

## Revision Note — Capability Consolidation (post-v0.3)

**Decision:** The two social capabilities (`p2pcd.social.post.1` as PROVIDE and `p2pcd.social.feed.1` as CONSUME) have been consolidated into a single capability:

```
howm.social.feed.1   role: both   mutual: true
```

**Rationale:** The original two-capability model created a spec/implementation misalignment. The P2P-CD intersection rule operates per-capability *name* — if two peers each have `social.post/PROVIDE` and `social.feed/CONSUME`, no match occurs (PROVIDE+PROVIDE and CONSUME+CONSUME are non-matching). The intended bidirectional exchange only worked with a cross-name matching model, which contradicts the spec.

The correct fix is to model social participation as a single symmetric capability. Both/mutual:true produces a match whenever at least one peer declares the capability — this covers all four interaction patterns correctly:

| Scenario | Result |
|----------|--------|
| Both peers have `howm.social.feed.1` | ACTIVE — bidirectional feed exchange |
| Only one peer has `howm.social.feed.1` | ACTIVE — one-way feed (peer without cap gets heartbeat only) |
| Neither peer has `howm.social.feed.1` | heartbeat only |
| Trust gate denies social for a peer | heartbeat only |

**Direction handling** (who fetches from whom, post ownership, read vs write) is pushed entirely to the application layer inside the social-feed capability. The daemon only gates *whether* two peers may exchange social data — not *how*.

**Impact on implementation:**
- `p2pcd-types`: `generate_default()` and `SAMPLE_TOML` updated to single `howm.social.feed.1` capability
- Intersection tests updated to reflect new model
- Phase 7 task 7.3 (social-feed capability update) should implement read/write direction logic internally based on peer WG address and local post ownership
EOF; __hermes_rc=$?; printf '__HERMES_FENCE_a9f7b3__'; exit $__hermes_rc

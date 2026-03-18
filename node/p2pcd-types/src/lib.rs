// p2pcd-types: Core P2P-CD-01 v0.3 type definitions and wire encoding.
// CBOR wire format uses integer keys per spec §5.3.
// serde derives are for config/internal use only (TOML).

pub mod cbor;
pub mod config;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Protocol version. MUST be 1 for P2P-CD-01 v0.3.
pub const PROTOCOL_VERSION: u64 = 1;

/// WireGuard Curve25519 public key length in bytes.
pub const PEER_ID_LEN: usize = 32;

/// Type alias for peer identity (WireGuard Curve25519 public key, 32 bytes).
pub type PeerId = [u8; PEER_ID_LEN];

// ─── CBOR integer map key constants ──────────────────────────────────────────

/// CBOR integer map keys for discovery_manifest (§5.3)
pub mod manifest_keys {
    pub const PROTOCOL_VERSION: u64 = 1;
    pub const PEER_ID: u64 = 2;
    pub const SEQUENCE_NUM: u64 = 3;
    pub const CAPABILITIES: u64 = 4;
    pub const PERSONAL_HASH: u64 = 5;
    pub const HASH_ALGORITHM: u64 = 6;
}

/// CBOR integer map keys for capability_declaration (§5.3)
pub mod capability_keys {
    pub const NAME: u64 = 1;
    pub const ROLE: u64 = 2;
    pub const MUTUAL: u64 = 3;
    pub const CLASSIFICATION: u64 = 4; // omitted from wire per spec
    pub const SCOPE: u64 = 5;
}

/// CBOR integer map keys for scope_params (§5.3)
pub mod scope_keys {
    pub const RATE_LIMIT: u64 = 1;
    pub const TTL: u64 = 2;
}

/// CBOR integer map keys for protocol messages (outer envelope)
pub mod message_keys {
    pub const MESSAGE_TYPE: u64 = 1;
    pub const MANIFEST: u64 = 2; // for OFFER
    pub const PERSONAL_HASH: u64 = 3; // for CONFIRM and CLOSE
    pub const ACTIVE_SET: u64 = 4; // for CONFIRM
    pub const ACCEPTED_PARAMS: u64 = 5; // for CONFIRM
    pub const REASON: u64 = 6; // for CLOSE
    pub const TIMESTAMP: u64 = 7; // for PING/PONG
}

// ─── Wire message types ───────────────────────────────────────────────────────

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Offer = 1,
    Confirm = 2,
    Close = 3,
    Ping = 4,
    Pong = 5,
}

impl MessageType {
    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            1 => Some(MessageType::Offer),
            2 => Some(MessageType::Confirm),
            3 => Some(MessageType::Close),
            4 => Some(MessageType::Ping),
            5 => Some(MessageType::Pong),
            _ => None,
        }
    }
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

impl CloseReason {
    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            0 => Some(CloseReason::Normal),
            1 => Some(CloseReason::NoMatch),
            2 => Some(CloseReason::AuthFailure),
            3 => Some(CloseReason::VersionUnsupported),
            4 => Some(CloseReason::Timeout),
            255 => Some(CloseReason::Error),
            _ => None,
        }
    }
}

// ─── Role ─────────────────────────────────────────────────────────────────────

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Provide = 1,
    Consume = 2,
    Both = 3,
}

impl Role {
    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            1 => Some(Role::Provide),
            2 => Some(Role::Consume),
            3 => Some(Role::Both),
            _ => None,
        }
    }

    /// Returns true if two roles produce a match per §7.4 intersection rules.
    /// `self_mutual` and `other_mutual` are only used for Both+Both.
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

// ─── Classification tiers (application-level, NOT on wire) ───────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClassificationTier {
    /// Maps to UNRESTRICTED. Any peer with a valid WireGuard tunnel.
    Public,
    /// Implementation-defined. Peer's WG public key must be in friends list.
    Friends,
    /// Maps to DENIED. Peer is explicitly blocked.
    Blocked,
}

// ─── Scope parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScopeParams {
    /// Requests per second (0 = unlimited)
    pub rate_limit: u64,
    /// Session TTL in seconds (0 = no expiry)
    pub ttl: u64,
}

impl ScopeParams {
    /// Reconcile two scope params per §7.3: most-restrictive-wins.
    /// For rate_limit: 0 means unlimited so we take the non-zero value;
    /// if both non-zero, take the minimum.
    /// For ttl: 0 means no expiry so we take the non-zero value;
    /// if both non-zero, take the minimum.
    pub fn reconcile(&self, other: &ScopeParams) -> ScopeParams {
        ScopeParams {
            rate_limit: match (self.rate_limit, other.rate_limit) {
                (0, x) | (x, 0) => x,
                (a, b) => a.min(b),
            },
            ttl: match (self.ttl, other.ttl) {
                (0, x) | (x, 0) => x,
                (a, b) => a.min(b),
            },
        }
    }
}

// ─── Capability declaration ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDeclaration {
    /// Fully qualified name per §4.4 namespace grammar (e.g. "p2pcd.social.post.1")
    pub name: String,
    pub role: Role,
    /// Required for Both+Both matching
    pub mutual: bool,
    /// Scope constraints advertised to remote peers.
    /// Classification is local-only and MUST NOT appear on the wire.
    pub scope: Option<ScopeParams>,
}

// ─── Discovery manifest ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryManifest {
    pub protocol_version: u64,
    /// WireGuard Curve25519 public key (32 bytes) — this IS the peer_id.
    pub peer_id: PeerId,
    /// Monotonically increasing; incremented on each rebroadcast.
    pub sequence_num: u64,
    /// MUST be sorted lexicographically by name before serialization.
    pub capabilities: Vec<CapabilityDeclaration>,
    /// SHA-256 of deterministic CBOR-encoded manifest with sequence_num=0 (§5.5).
    pub personal_hash: Vec<u8>,
    /// IANA hash algorithm name, e.g. "sha-256".
    pub hash_algorithm: String,
}

impl DiscoveryManifest {
    /// Sort capabilities lexicographically by name per §4.5.
    pub fn sort_capabilities(&mut self) {
        self.capabilities.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

// ─── Protocol messages ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ProtocolMessage {
    Offer {
        manifest: DiscoveryManifest,
    },
    Confirm {
        personal_hash: Vec<u8>,
        /// Sorted capability names in the active set.
        active_set: Vec<String>,
        /// Reconciled scope params per capability.
        accepted_params: Option<BTreeMap<String, ScopeParams>>,
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

// ─── Trust gate (application-level) ───────────────────────────────────────────

/// Local trust gate configuration per capability.
/// Uses WireGuard public keys for peer identification.
#[derive(Debug, Clone)]
pub struct TrustPolicy {
    /// Default tier applied to peers not in overrides.
    pub default_tier: ClassificationTier,
    /// Per-peer overrides: WG public key -> tier.
    pub overrides: HashMap<PeerId, ClassificationTier>,
    /// Friends list: set of WireGuard public keys.
    pub friends: HashSet<PeerId>,
}

impl TrustPolicy {
    /// Evaluate trust gate for a specific peer.
    /// Returns true for ALLOW, false for DENY.
    pub fn evaluate(&self, remote_peer_id: &PeerId) -> bool {
        if let Some(tier) = self.overrides.get(remote_peer_id) {
            return match tier {
                ClassificationTier::Public => true,
                ClassificationTier::Friends => self.friends.contains(remote_peer_id),
                ClassificationTier::Blocked => false,
            };
        }
        match self.default_tier {
            ClassificationTier::Public => true,
            ClassificationTier::Friends => self.friends.contains(remote_peer_id),
            ClassificationTier::Blocked => false,
        }
    }
}

// ─── Intersection computation (§7.4) ──────────────────────────────────────────

/// Compute the active set from two manifests + local trust policies.
/// Returns sorted list of capability names that both peers agreed on.
/// Classification is NOT on the wire — trust gates use the WG public key.
pub fn compute_intersection(
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
    trust_policies: &HashMap<String, TrustPolicy>,
) -> Vec<String> {
    let mut active = Vec::new();

    for local_cap in &local.capabilities {
        for remote_cap in &remote.capabilities {
            if local_cap.name != remote_cap.name {
                continue;
            }
            // Role match check per §7.4
            if !local_cap
                .role
                .matches(&remote_cap.role, local_cap.mutual, remote_cap.mutual)
            {
                continue;
            }
            // Trust gate check using remote manifest's peer_id (= WG public key)
            if let Some(policy) = trust_policies.get(&local_cap.name) {
                if !policy.evaluate(&remote.peer_id) {
                    continue;
                }
            }
            active.push(local_cap.name.clone());
            break;
        }
    }

    active.sort();
    active
}

// ─── WireGuard peer state ──────────────────────────────────────────────────────

/// Represents a WireGuard peer's state as parsed from `wg show <iface> dump`.
#[derive(Debug, Clone)]
pub struct WgPeerState {
    pub public_key: PeerId,
    pub endpoint: Option<std::net::SocketAddr>,
    pub allowed_ips: Vec<String>,
    /// Timestamp of last successful handshake (Unix epoch seconds; 0 = never)
    pub latest_handshake: u64,
    /// Bytes received (cumulative)
    pub rx_bytes: u64,
    /// Bytes transmitted (cumulative)
    pub tx_bytes: u64,
}

impl WgPeerState {
    /// Returns true if the peer has ever completed a WireGuard handshake.
    pub fn is_reachable(&self) -> bool {
        self.latest_handshake > 0
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Role::matches tests ──────────────────────────────────────────────────

    #[test]
    fn role_provide_consume_matches() {
        assert!(Role::Provide.matches(&Role::Consume, false, false));
        assert!(Role::Consume.matches(&Role::Provide, false, false));
    }

    #[test]
    fn role_both_provide_matches() {
        assert!(Role::Both.matches(&Role::Provide, false, false));
        assert!(Role::Provide.matches(&Role::Both, false, false));
    }

    #[test]
    fn role_both_consume_matches() {
        assert!(Role::Both.matches(&Role::Consume, false, false));
        assert!(Role::Consume.matches(&Role::Both, false, false));
    }

    #[test]
    fn role_both_both_requires_mutual() {
        // Both + Both only matches when both have mutual=true
        assert!(!Role::Both.matches(&Role::Both, false, false));
        assert!(!Role::Both.matches(&Role::Both, true, false));
        assert!(!Role::Both.matches(&Role::Both, false, true));
        assert!(Role::Both.matches(&Role::Both, true, true));
    }

    #[test]
    fn role_same_side_never_matches() {
        assert!(!Role::Provide.matches(&Role::Provide, false, false));
        assert!(!Role::Consume.matches(&Role::Consume, false, false));
    }

    // ── ScopeParams::reconcile tests ─────────────────────────────────────────

    #[test]
    fn scope_reconcile_min_wins() {
        let a = ScopeParams {
            rate_limit: 10,
            ttl: 3600,
        };
        let b = ScopeParams {
            rate_limit: 5,
            ttl: 7200,
        };
        let r = a.reconcile(&b);
        assert_eq!(r.rate_limit, 5);
        assert_eq!(r.ttl, 3600);
    }

    #[test]
    fn scope_reconcile_zero_is_unlimited() {
        let a = ScopeParams {
            rate_limit: 0,
            ttl: 0,
        };
        let b = ScopeParams {
            rate_limit: 10,
            ttl: 3600,
        };
        let r = a.reconcile(&b);
        // 0 = unlimited; take the other value
        assert_eq!(r.rate_limit, 10);
        assert_eq!(r.ttl, 3600);
    }

    #[test]
    fn scope_reconcile_both_zero_stays_zero() {
        let a = ScopeParams {
            rate_limit: 0,
            ttl: 0,
        };
        let b = ScopeParams {
            rate_limit: 0,
            ttl: 0,
        };
        let r = a.reconcile(&b);
        assert_eq!(r.rate_limit, 0);
        assert_eq!(r.ttl, 0);
    }

    // ── TrustPolicy::evaluate tests ──────────────────────────────────────────

    fn peer(b: u8) -> PeerId {
        [b; 32]
    }

    #[test]
    fn trust_public_allows_all() {
        let policy = TrustPolicy {
            default_tier: ClassificationTier::Public,
            overrides: HashMap::new(),
            friends: HashSet::new(),
        };
        assert!(policy.evaluate(&peer(0xAA)));
        assert!(policy.evaluate(&peer(0x00)));
    }

    #[test]
    fn trust_friends_blocks_stranger() {
        let policy = TrustPolicy {
            default_tier: ClassificationTier::Friends,
            overrides: HashMap::new(),
            friends: {
                let mut s = HashSet::new();
                s.insert(peer(0x01));
                s
            },
        };
        assert!(!policy.evaluate(&peer(0xFF))); // stranger
        assert!(policy.evaluate(&peer(0x01))); // friend
    }

    #[test]
    fn trust_blocked_denies_all() {
        let policy = TrustPolicy {
            default_tier: ClassificationTier::Blocked,
            overrides: HashMap::new(),
            friends: HashSet::new(),
        };
        assert!(!policy.evaluate(&peer(0xAA)));
    }

    #[test]
    fn trust_override_takes_precedence() {
        let stranger = peer(0xFF);
        let policy = TrustPolicy {
            default_tier: ClassificationTier::Blocked,
            overrides: {
                let mut m = HashMap::new();
                m.insert(stranger, ClassificationTier::Public);
                m
            },
            friends: HashSet::new(),
        };
        // Override says Public → allow even though default is Blocked
        assert!(policy.evaluate(&stranger));
    }

    // ── compute_intersection tests (from POC doc §9 scenarios) ──────────────

    fn cap(name: &str, role: Role, mutual: bool) -> CapabilityDeclaration {
        CapabilityDeclaration {
            name: name.to_string(),
            role,
            mutual,
            scope: None,
        }
    }

    fn manifest(id: u8, caps: Vec<CapabilityDeclaration>) -> DiscoveryManifest {
        DiscoveryManifest {
            protocol_version: 1,
            peer_id: peer(id),
            sequence_num: 1,
            capabilities: caps,
            personal_hash: vec![],
            hash_algorithm: "sha-256".to_string(),
        }
    }

    /// Normal peer: heartbeat + howm.social.feed.1 (Both/mutual — direction is app-layer)
    fn social_peer(id: u8) -> DiscoveryManifest {
        manifest(
            id,
            vec![
                cap("core.heartbeat.liveness.1", Role::Both, true),
                cap("howm.social.feed.1", Role::Both, true),
            ],
        )
    }

    /// Peer without social participation (heartbeat only)
    fn no_social_peer(id: u8) -> DiscoveryManifest {
        manifest(id, vec![cap("core.heartbeat.liveness.1", Role::Both, true)])
    }

    /// §9.1: Social ↔ Social → both caps active
    #[test]
    fn intersection_social_social() {
        let alice = social_peer(0xA1);
        let bob = social_peer(0xB0);
        let policies = HashMap::new();
        let active = compute_intersection(&alice, &bob, &policies);
        assert_eq!(
            active,
            vec!["core.heartbeat.liveness.1", "howm.social.feed.1"]
        );
    }

    /// §9.2: Social ↔ No-Social → heartbeat only
    #[test]
    fn intersection_social_no_social() {
        let alice = social_peer(0xA1);
        let bob = no_social_peer(0xB0);
        let policies = HashMap::new();
        let active = compute_intersection(&alice, &bob, &policies);
        assert_eq!(active, vec!["core.heartbeat.liveness.1"]);
    }

    /// §9.3: Private ↔ Stranger → heartbeat only (trust gate blocks social)
    #[test]
    fn intersection_private_stranger() {
        let stranger_id = peer(0xFF);
        let friend_id = peer(0x01);

        let private_user = social_peer(0xA1); // same caps, FRIENDS policy applied
        let stranger = DiscoveryManifest {
            peer_id: stranger_id,
            ..social_peer(0xFF)
        };

        let mut policies = HashMap::new();
        policies.insert(
            "howm.social.feed.1".to_string(),
            TrustPolicy {
                default_tier: ClassificationTier::Friends,
                overrides: HashMap::new(),
                friends: {
                    let mut s = HashSet::new();
                    s.insert(friend_id);
                    s
                },
            },
        );

        let active = compute_intersection(&private_user, &stranger, &policies);
        assert_eq!(active, vec!["core.heartbeat.liveness.1"]);
    }

    /// §9.4: Private ↔ Friend → both caps active
    #[test]
    fn intersection_private_friend() {
        let friend_id = peer(0x01);

        let private_user = social_peer(0xA1);
        let friend = DiscoveryManifest {
            peer_id: friend_id,
            ..social_peer(0x01)
        };

        let mut policies = HashMap::new();
        policies.insert(
            "howm.social.feed.1".to_string(),
            TrustPolicy {
                default_tier: ClassificationTier::Friends,
                overrides: HashMap::new(),
                friends: {
                    let mut s = HashSet::new();
                    s.insert(friend_id);
                    s
                },
            },
        );

        let active = compute_intersection(&private_user, &friend, &policies);
        assert_eq!(
            active,
            vec!["core.heartbeat.liveness.1", "howm.social.feed.1"]
        );
    }

    /// §9.5: No-Social ↔ No-Social → heartbeat only
    #[test]
    fn intersection_no_social_no_social() {
        let a = no_social_peer(0xA1);
        let b = no_social_peer(0xB0);
        let policies = HashMap::new();
        let active = compute_intersection(&a, &b, &policies);
        assert_eq!(active, vec!["core.heartbeat.liveness.1"]);
    }
}

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
    pub const APPLICABLE_SCOPE_KEYS: u64 = 6;
}

/// CBOR integer map keys for scope_params (§5.3)
pub mod scope_keys {
    pub const RATE_LIMIT: u64 = 1;
    pub const TTL: u64 = 2;
    // Core capability-specific params (keys 3-23, reserved per v0.4 spec)
    pub const HEARTBEAT_INTERVAL_MS: u64 = 3;
    pub const HEARTBEAT_TIMEOUT_MS: u64 = 4;
    pub const TIMESYNC_PRECISION_MS: u64 = 5;
    pub const LATENCY_SAMPLE_INTERVAL_MS: u64 = 6;
    pub const LATENCY_WINDOW_SIZE: u64 = 7;
    pub const ENDPOINT_INCLUDE_GEO: u64 = 8;
    pub const RELAY_MAX_CIRCUITS: u64 = 9;
    pub const RELAY_MAX_BANDWIDTH_KBPS: u64 = 10;
    pub const RELAY_TTL: u64 = 11;
    pub const PEX_MAX_PEERS: u64 = 12;
    pub const PEX_INCLUDE_CAPABILITIES: u64 = 13;
    pub const STREAM_BITRATE_KBPS: u64 = 14;
    pub const STREAM_CODEC: u64 = 15;
    pub const BLOB_MAX_BYTES: u64 = 16;
    pub const BLOB_CHUNK_SIZE: u64 = 17;
    pub const BLOB_HASH_ALGORITHM: u64 = 18;
    pub const RPC_MAX_REQUEST_BYTES: u64 = 19;
    pub const RPC_MAX_RESPONSE_BYTES: u64 = 20;
    pub const RPC_METHODS: u64 = 21;
    pub const EVENT_TOPICS: u64 = 22;
    pub const EVENT_MAX_PAYLOAD_BYTES: u64 = 23;
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

// ─── Wire message types (§5.3.6 + Appendix B.12) ────────────────────────────

/// Message type constants per spec §5.3.6 and Appendix B.12.
pub mod message_types {
    // Protocol core (1-3)
    pub const OFFER: u64 = 1;
    pub const CONFIRM: u64 = 2;
    pub const CLOSE: u64 = 3;
    // core.session.heartbeat.1 (4-5)
    pub const PING: u64 = 4;
    pub const PONG: u64 = 5;
    // core.session.attest.1 (6)
    pub const BUILD_ATTEST: u64 = 6;
    // core.session.timesync.1 (7-8)
    pub const TIME_REQ: u64 = 7;
    pub const TIME_RESP: u64 = 8;
    // core.session.latency.1 (9-10)
    pub const LAT_PING: u64 = 9;
    pub const LAT_PONG: u64 = 10;
    // core.network.endpoint.1 (11-12)
    pub const WHOAMI_REQ: u64 = 11;
    pub const WHOAMI_RESP: u64 = 12;
    // core.network.relay.1 (13-15)
    pub const CIRCUIT_OPEN: u64 = 13;
    pub const CIRCUIT_DATA: u64 = 14;
    pub const CIRCUIT_CLOSE: u64 = 15;
    // core.network.peerexchange.1 (16-17)
    pub const PEX_REQ: u64 = 16;
    pub const PEX_RESP: u64 = 17;
    // core.data.blob.1 (18-21)
    pub const BLOB_REQ: u64 = 18;
    pub const BLOB_OFFER: u64 = 19;
    pub const BLOB_CHUNK: u64 = 20;
    pub const BLOB_ACK: u64 = 21;
    // core.data.rpc.1 (22-23)
    pub const RPC_REQ: u64 = 22;
    pub const RPC_RESP: u64 = 23;
    // core.data.event.1 (24-26)
    pub const EVENT_SUB: u64 = 24;
    pub const EVENT_UNSUB: u64 = 25;
    pub const EVENT_MSG: u64 = 26;
    // 27-31: reserved for v2 core extensions
    // 32+: application-defined
}

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

    /// Returns true if this message_type is a protocol-level message (1-5).
    /// Capability messages (6+) are routed to handlers.
    pub fn is_protocol(&self) -> bool {
        (*self as u64) <= 5
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

/// A scope parameter value — covers all types the spec allows for extension keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScopeValue {
    Uint(u64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
    Array(Vec<ScopeValue>),
}

impl ScopeValue {
    /// Try to extract as u64.
    pub fn as_uint(&self) -> Option<u64> {
        match self {
            ScopeValue::Uint(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract as bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ScopeValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract as text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ScopeValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Try to extract as string array (for methods/topics lists).
    pub fn as_text_array(&self) -> Option<Vec<&str>> {
        match self {
            ScopeValue::Array(arr) => {
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    out.push(v.as_text()?);
                }
                Some(out)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScopeParams {
    /// Requests per second (0 = unlimited) — scope key 1
    pub rate_limit: u64,
    /// Session TTL in seconds (0 = no expiry) — scope key 2
    pub ttl: u64,
    /// Extension params (keys 3+), stored as integer→value pairs.
    /// Keys 3-15: reserved for core spec. 16-127: registered extensions. 128+: app-defined.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<u64, ScopeValue>,
}

impl ScopeParams {
    /// Reconcile two scope params per §7.3: most-restrictive-wins for numeric,
    /// provider-takes-precedence for non-numeric, intersection for arrays.
    ///
    /// `self` is the local (provider) side, `other` is the remote side.
    /// For rate_limit/ttl: 0 means unlimited so we take the non-zero value;
    /// if both non-zero, take the minimum.
    /// For extension keys: uint → most-restrictive-wins, bool/text → provider (self) wins,
    /// array → intersection.
    pub fn reconcile(&self, other: &ScopeParams) -> ScopeParams {
        let rate_limit = reconcile_uint_zero_unlimited(self.rate_limit, other.rate_limit);
        let ttl = reconcile_uint_zero_unlimited(self.ttl, other.ttl);

        // Merge extensions
        let mut extensions = BTreeMap::new();
        let all_keys: std::collections::BTreeSet<u64> = self
            .extensions
            .keys()
            .chain(other.extensions.keys())
            .copied()
            .collect();

        for key in all_keys {
            match (self.extensions.get(&key), other.extensions.get(&key)) {
                (Some(a), Some(b)) => {
                    extensions.insert(key, reconcile_scope_value(a, b));
                }
                (Some(v), None) | (None, Some(v)) => {
                    extensions.insert(key, v.clone());
                }
                (None, None) => unreachable!(),
            }
        }

        ScopeParams {
            rate_limit,
            ttl,
            extensions,
        }
    }

    /// Get an extension value by key.
    pub fn get_ext(&self, key: u64) -> Option<&ScopeValue> {
        self.extensions.get(&key)
    }

    /// Get an extension uint value by key.
    pub fn get_ext_uint(&self, key: u64) -> Option<u64> {
        self.extensions.get(&key).and_then(|v| v.as_uint())
    }

    /// Set an extension value.
    pub fn set_ext(&mut self, key: u64, val: ScopeValue) {
        self.extensions.insert(key, val);
    }
}

/// Most-restrictive-wins for uint where 0 = unlimited.
fn reconcile_uint_zero_unlimited(a: u64, b: u64) -> u64 {
    match (a, b) {
        (0, x) | (x, 0) => x,
        (a, b) => a.min(b),
    }
}

/// Reconcile a single ScopeValue pair:
/// - Uint: most-restrictive-wins (min of non-zero)
/// - Bool/Text/Bytes: first value wins (provider-takes-precedence)
/// - Array of Text: intersection
fn reconcile_scope_value(a: &ScopeValue, b: &ScopeValue) -> ScopeValue {
    match (a, b) {
        (ScopeValue::Uint(va), ScopeValue::Uint(vb)) => {
            ScopeValue::Uint(reconcile_uint_zero_unlimited(*va, *vb))
        }
        // Array of texts → intersection (for methods, topics)
        (ScopeValue::Array(va), ScopeValue::Array(vb)) => {
            let result: Vec<ScopeValue> = va
                .iter()
                .filter(|item| vb.contains(item))
                .cloned()
                .collect();
            ScopeValue::Array(result)
        }
        // Non-numeric: provider (first arg) takes precedence per §7.3
        _ => a.clone(),
    }
}

// ─── Capability declaration ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDeclaration {
    /// Fully qualified name per §4.4 namespace grammar (e.g. "core.session.heartbeat.1")
    pub name: String,
    pub role: Role,
    /// Required for Both+Both matching
    pub mutual: bool,
    /// Scope constraints advertised to remote peers.
    /// Classification is local-only and MUST NOT appear on the wire.
    pub scope: Option<ScopeParams>,
    /// Optional list of scope parameter keys meaningful for this capability (§4.2).
    /// If present, receiver enforces only listed keys and ignores all others.
    /// If absent, falls back to the capability's specification document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applicable_scope_keys: Option<Vec<u64>>,
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
    /// Generic capability message (type 6+). Decoded by capability handlers.
    CapabilityMsg {
        /// Message type integer (6-26 for core, 32+ for app-defined).
        message_type: u64,
        /// Raw CBOR payload (the full message map minus the message_type key).
        payload: Vec<u8>,
    },
}

// ─── Capability handler trait ─────────────────────────────────────────────────

/// Context passed to capability handlers when they are activated or receive messages.
#[derive(Debug, Clone)]
pub struct CapabilityContext {
    /// Remote peer identity.
    pub peer_id: PeerId,
    /// Negotiated scope params for this capability.
    pub params: ScopeParams,
    /// Capability name.
    pub capability_name: String,
}

/// Trait for capability message handlers.
///
/// Each capability (heartbeat, attest, timesync, etc.) implements this trait.
/// The engine dispatches incoming messages by type to the appropriate handler.
pub trait CapabilityHandler: Send + Sync {
    /// Capability name this handler serves (e.g. "core.session.heartbeat.1").
    fn capability_name(&self) -> &str;

    /// Message type integers this handler accepts (e.g. [4, 5] for heartbeat).
    fn handled_message_types(&self) -> &[u64];

    /// Called when the capability enters the active set after CONFIRM reconciliation.
    /// For capabilities with an activation exchange (e.g. attest), this is where
    /// the initial message is sent.
    fn on_activated(
        &self,
        _ctx: &CapabilityContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// Called when a message of a handled type arrives.
    fn on_message(
        &self,
        msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Called when the capability is deactivated (session close or re-exchange removal).
    fn on_deactivated(
        &self,
        _ctx: &CapabilityContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
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
            ..Default::default()
        };
        let b = ScopeParams {
            rate_limit: 5,
            ttl: 7200,
            ..Default::default()
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
            ..Default::default()
        };
        let b = ScopeParams {
            rate_limit: 10,
            ttl: 3600,
            ..Default::default()
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
            ..Default::default()
        };
        let b = ScopeParams {
            rate_limit: 0,
            ttl: 0,
            ..Default::default()
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
            applicable_scope_keys: None,
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
                cap("core.session.heartbeat.1", Role::Both, true),
                cap("howm.social.feed.1", Role::Both, true),
            ],
        )
    }

    /// Peer without social participation (heartbeat only)
    fn no_social_peer(id: u8) -> DiscoveryManifest {
        manifest(id, vec![cap("core.session.heartbeat.1", Role::Both, true)])
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
            vec!["core.session.heartbeat.1", "howm.social.feed.1"]
        );
    }

    /// §9.2: Social ↔ No-Social → heartbeat only
    #[test]
    fn intersection_social_no_social() {
        let alice = social_peer(0xA1);
        let bob = no_social_peer(0xB0);
        let policies = HashMap::new();
        let active = compute_intersection(&alice, &bob, &policies);
        assert_eq!(active, vec!["core.session.heartbeat.1"]);
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
        assert_eq!(active, vec!["core.session.heartbeat.1"]);
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
            vec!["core.session.heartbeat.1", "howm.social.feed.1"]
        );
    }

    /// §9.5: No-Social ↔ No-Social → heartbeat only
    #[test]
    fn intersection_no_social_no_social() {
        let a = no_social_peer(0xA1);
        let b = no_social_peer(0xB0);
        let policies = HashMap::new();
        let active = compute_intersection(&a, &b, &policies);
        assert_eq!(active, vec!["core.session.heartbeat.1"]);
    }
}

// p2pcd-types: Core P2P-CD-01 v0.3 type definitions and wire encoding.
// CBOR wire format uses integer keys per spec §5.3.
// serde derives are for config/internal use only (TOML).

pub mod cbor;
pub mod config;

pub mod handler;
pub mod message_types;
pub mod scope;
pub mod wire;

pub use handler::*;
pub use message_types::*;
pub use scope::*;
pub use wire::*;

use serde::{Deserialize, Serialize};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Protocol version. MUST be 1 for P2P-CD-01 v0.3.
pub const PROTOCOL_VERSION: u64 = 1;

/// WireGuard Curve25519 public key length in bytes.
pub const PEER_ID_LEN: usize = 32;

/// Type alias for peer identity (WireGuard Curve25519 public key, 32 bytes).
pub type PeerId = [u8; PEER_ID_LEN];

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
    #![allow(deprecated)]
    use super::*;
    use std::collections::{HashMap, HashSet};

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

    // ── TrustPolicy::evaluate tests (deprecated, kept for backward compat) ───

    fn peer(b: u8) -> PeerId {
        [b; 32]
    }

    #[test]
    #[allow(deprecated)]
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
    #[allow(deprecated)]
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
    #[allow(deprecated)]
    fn trust_blocked_denies_all() {
        let policy = TrustPolicy {
            default_tier: ClassificationTier::Blocked,
            overrides: HashMap::new(),
            friends: HashSet::new(),
        };
        assert!(!policy.evaluate(&peer(0xAA)));
    }

    #[test]
    #[allow(deprecated)]
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
        let allow_all = |_: &str, _: &PeerId| true;
        let active = compute_intersection(&alice, &bob, &allow_all);
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
        let allow_all = |_: &str, _: &PeerId| true;
        let active = compute_intersection(&alice, &bob, &allow_all);
        assert_eq!(active, vec!["core.session.heartbeat.1"]);
    }

    /// §9.3: Private ↔ Stranger → heartbeat only (trust gate blocks social)
    #[test]
    fn intersection_private_stranger() {
        let friend_id = peer(0x01);

        let private_user = social_peer(0xA1);
        let stranger = DiscoveryManifest {
            peer_id: peer(0xFF),
            ..social_peer(0xFF)
        };

        // Trust gate: only friend_id is allowed social access
        let gate = |cap: &str, pid: &PeerId| -> bool {
            if cap == "howm.social.feed.1" {
                *pid == friend_id
            } else {
                true
            }
        };

        let active = compute_intersection(&private_user, &stranger, &gate);
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

        // Trust gate: only friend_id is allowed social access
        let gate = |cap: &str, pid: &PeerId| -> bool {
            if cap == "howm.social.feed.1" {
                *pid == friend_id
            } else {
                true
            }
        };

        let active = compute_intersection(&private_user, &friend, &gate);
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
        let allow_all = |_: &str, _: &PeerId| true;
        let active = compute_intersection(&a, &b, &allow_all);
        assert_eq!(active, vec!["core.session.heartbeat.1"]);
    }

    // ── Phase 1 v4 conformance: extensible scope params ───────────────────────

    #[test]
    fn scope_extensions_uint_reconcile_min() {
        let mut a = ScopeParams::default();
        a.set_ext(scope_keys::HEARTBEAT_INTERVAL_MS, ScopeValue::Uint(5000));
        let mut b = ScopeParams::default();
        b.set_ext(scope_keys::HEARTBEAT_INTERVAL_MS, ScopeValue::Uint(3000));
        let r = a.reconcile(&b);
        assert_eq!(
            r.get_ext_uint(scope_keys::HEARTBEAT_INTERVAL_MS),
            Some(3000)
        );
    }

    #[test]
    fn scope_extensions_bool_provider_wins() {
        let mut a = ScopeParams::default();
        a.set_ext(scope_keys::ENDPOINT_INCLUDE_GEO, ScopeValue::Bool(true));
        let mut b = ScopeParams::default();
        b.set_ext(scope_keys::ENDPOINT_INCLUDE_GEO, ScopeValue::Bool(false));
        // Provider (a) wins for non-numeric
        let r = a.reconcile(&b);
        assert_eq!(
            r.get_ext(scope_keys::ENDPOINT_INCLUDE_GEO)
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn scope_extensions_text_array_intersection() {
        let methods_a = ScopeValue::Array(vec![
            ScopeValue::Text("ping".into()),
            ScopeValue::Text("echo".into()),
            ScopeValue::Text("status".into()),
        ]);
        let methods_b = ScopeValue::Array(vec![
            ScopeValue::Text("echo".into()),
            ScopeValue::Text("status".into()),
            ScopeValue::Text("shutdown".into()),
        ]);
        let mut a = ScopeParams::default();
        a.set_ext(scope_keys::RPC_METHODS, methods_a);
        let mut b = ScopeParams::default();
        b.set_ext(scope_keys::RPC_METHODS, methods_b);
        let r = a.reconcile(&b);
        let result = r
            .get_ext(scope_keys::RPC_METHODS)
            .unwrap()
            .as_text_array()
            .unwrap();
        assert_eq!(result, vec!["echo", "status"]);
    }

    #[test]
    fn scope_extensions_one_side_only() {
        let mut a = ScopeParams::default();
        a.set_ext(scope_keys::RELAY_MAX_CIRCUITS, ScopeValue::Uint(10));
        let b = ScopeParams::default();
        let r = a.reconcile(&b);
        assert_eq!(r.get_ext_uint(scope_keys::RELAY_MAX_CIRCUITS), Some(10));
    }

    #[test]
    fn scope_extensions_uint_zero_unlimited() {
        let mut a = ScopeParams::default();
        a.set_ext(scope_keys::RELAY_MAX_BANDWIDTH_KBPS, ScopeValue::Uint(0));
        let mut b = ScopeParams::default();
        b.set_ext(scope_keys::RELAY_MAX_BANDWIDTH_KBPS, ScopeValue::Uint(1000));
        let r = a.reconcile(&b);
        assert_eq!(
            r.get_ext_uint(scope_keys::RELAY_MAX_BANDWIDTH_KBPS),
            Some(1000)
        );
    }

    // ── Phase 1 v4 conformance: scope key registry (keys 3-23) ────────────────

    #[test]
    fn scope_key_registry_core_keys_distinct() {
        // Verify all core scope keys 3-26 are distinct
        let keys = [
            scope_keys::HEARTBEAT_INTERVAL_MS,
            scope_keys::HEARTBEAT_TIMEOUT_MS,
            scope_keys::TIMESYNC_PRECISION_MS,
            scope_keys::LATENCY_SAMPLE_INTERVAL_MS,
            scope_keys::LATENCY_WINDOW_SIZE,
            scope_keys::ENDPOINT_INCLUDE_GEO,
            scope_keys::RELAY_MAX_CIRCUITS,
            scope_keys::RELAY_MAX_BANDWIDTH_KBPS,
            scope_keys::RELAY_TTL,
            scope_keys::PEX_MAX_PEERS,
            scope_keys::PEX_INCLUDE_CAPABILITIES,
            scope_keys::STREAM_BITRATE_KBPS,
            scope_keys::STREAM_CODEC,
            scope_keys::BLOB_MAX_BYTES,
            scope_keys::BLOB_CHUNK_SIZE,
            scope_keys::BLOB_HASH_ALGORITHM,
            scope_keys::RPC_MAX_REQUEST_BYTES,
            scope_keys::RPC_MAX_RESPONSE_BYTES,
            scope_keys::RPC_METHODS,
            scope_keys::EVENT_TOPICS,
            scope_keys::EVENT_MAX_PAYLOAD_BYTES,
            scope_keys::STREAM_MAX_CONCURRENT,
            scope_keys::STREAM_MAX_FRAME_BYTES,
            scope_keys::STREAM_TIMEOUT_SECS,
        ];
        let set: HashSet<u64> = keys.iter().copied().collect();
        assert_eq!(set.len(), keys.len(), "scope keys must be unique");
        // All in range 3..=26
        for k in &keys {
            assert!(
                *k >= 3 && *k <= 26,
                "core scope key {} out of range 3-26",
                k
            );
        }
    }

    #[test]
    fn scope_key_registry_rate_limit_ttl_are_1_2() {
        assert_eq!(scope_keys::RATE_LIMIT, 1);
        assert_eq!(scope_keys::TTL, 2);
    }

    // ── Phase 1 v4 conformance: applicable_scope_keys ─────────────────────────

    #[test]
    fn applicable_scope_keys_declared() {
        let cap = CapabilityDeclaration {
            name: "core.session.heartbeat.1".to_string(),
            role: Role::Both,
            mutual: true,
            scope: None,
            applicable_scope_keys: Some(vec![
                scope_keys::HEARTBEAT_INTERVAL_MS,
                scope_keys::HEARTBEAT_TIMEOUT_MS,
            ]),
        };
        let keys = cap.applicable_scope_keys.as_ref().unwrap();
        assert!(keys.contains(&scope_keys::HEARTBEAT_INTERVAL_MS));
        assert!(keys.contains(&scope_keys::HEARTBEAT_TIMEOUT_MS));
        assert!(!keys.contains(&scope_keys::RELAY_MAX_CIRCUITS));
    }

    #[test]
    fn applicable_scope_keys_none_means_spec_fallback() {
        let cap = CapabilityDeclaration {
            name: "howm.social.feed.1".to_string(),
            role: Role::Both,
            mutual: true,
            scope: None,
            applicable_scope_keys: None,
        };
        assert!(cap.applicable_scope_keys.is_none());
    }

    // ── Phase 1 v4 conformance: CapabilityHandler trait ───────────────────────

    #[test]
    fn capability_handler_default_impls() {
        // Verify the default on_activated and on_deactivated compile and return Ok
        struct TestHandler;
        impl CapabilityHandler for TestHandler {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn capability_name(&self) -> &str {
                "test.cap.1"
            }
            fn handled_message_types(&self) -> &[u64] {
                &[99]
            }
            fn on_message(
                &self,
                _msg_type: u64,
                _payload: &[u8],
                _ctx: &CapabilityContext,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
            {
                Box::pin(async { Ok(()) })
            }
        }

        let handler = TestHandler;
        assert_eq!(handler.capability_name(), "test.cap.1");
        assert_eq!(handler.handled_message_types(), &[99]);
    }

    #[test]
    fn capability_handler_on_activated_default_ok() {
        // Verify the default on_activated/on_deactivated return Ok via poll
        struct NoopHandler;
        impl CapabilityHandler for NoopHandler {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn capability_name(&self) -> &str {
                "noop.1"
            }
            fn handled_message_types(&self) -> &[u64] {
                &[]
            }
            fn on_message(
                &self,
                _: u64,
                _: &[u8],
                _: &CapabilityContext,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
            {
                Box::pin(async { Ok(()) })
            }
        }

        let h = NoopHandler;
        let ctx = CapabilityContext {
            peer_id: [0xAA; 32],
            params: ScopeParams::default(),
            capability_name: "noop.1".to_string(),
        };
        // The default impl returns a ready future — poll it synchronously
        use std::task::{Context as TaskContext, Poll, Wake, Waker};
        struct NoopWaker;
        impl Wake for NoopWaker {
            fn wake(self: std::sync::Arc<Self>) {}
        }
        let waker = Waker::from(std::sync::Arc::new(NoopWaker));
        let mut cx = TaskContext::from_waker(&waker);

        let mut fut = h.on_activated(&ctx);
        assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
        let mut fut = h.on_deactivated(&ctx);
        assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
    }

    #[test]
    fn capability_context_fields() {
        let ctx = CapabilityContext {
            peer_id: [0xBB; 32],
            params: ScopeParams {
                rate_limit: 42,
                ttl: 100,
                ..Default::default()
            },
            capability_name: "core.session.heartbeat.1".to_string(),
        };
        assert_eq!(ctx.peer_id, [0xBB; 32]);
        assert_eq!(ctx.params.rate_limit, 42);
        assert_eq!(ctx.capability_name, "core.session.heartbeat.1");
    }

    // ── Phase 1 v4 conformance: message type registry ─────────────────────────

    #[test]
    fn message_types_core_range() {
        // Protocol core messages 1-5
        assert_eq!(message_types::OFFER, 1);
        assert_eq!(message_types::CONFIRM, 2);
        assert_eq!(message_types::CLOSE, 3);
        assert_eq!(message_types::PING, 4);
        assert_eq!(message_types::PONG, 5);
    }

    #[test]
    fn message_types_capability_range() {
        // Capability messages 6-30
        assert_eq!(message_types::BUILD_ATTEST, 6);
        assert_eq!(message_types::STREAM_CONTROL, 30);
        // All capability message types > 5
        let cap_types = [
            message_types::BUILD_ATTEST,
            message_types::TIME_REQ,
            message_types::TIME_RESP,
            message_types::LAT_PING,
            message_types::LAT_PONG,
            message_types::WHOAMI_REQ,
            message_types::WHOAMI_RESP,
            message_types::CIRCUIT_OPEN,
            message_types::CIRCUIT_DATA,
            message_types::CIRCUIT_CLOSE,
            message_types::PEX_REQ,
            message_types::PEX_RESP,
            message_types::BLOB_REQ,
            message_types::BLOB_OFFER,
            message_types::BLOB_CHUNK,
            message_types::BLOB_ACK,
            message_types::RPC_REQ,
            message_types::RPC_RESP,
            message_types::EVENT_SUB,
            message_types::EVENT_UNSUB,
            message_types::EVENT_MSG,
            message_types::STREAM_OPEN,
            message_types::STREAM_DATA,
            message_types::STREAM_CLOSE,
            message_types::STREAM_CONTROL,
        ];
        for t in &cap_types {
            assert!(
                *t >= 6 && *t <= 30,
                "capability msg type {} out of range",
                t
            );
        }
        // All unique
        let set: HashSet<u64> = cap_types.iter().copied().collect();
        assert_eq!(set.len(), cap_types.len());
    }

    #[test]
    fn message_type_is_protocol() {
        assert!(MessageType::Offer.is_protocol());
        assert!(MessageType::Pong.is_protocol());
    }
}

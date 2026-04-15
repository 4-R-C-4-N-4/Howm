// p2pcd-types: Scope types — ClassificationTier, ScopeValue, ScopeParams, and
// intersection computation helpers.
// Extracted from lib.rs — all items remain accessible at p2pcd_types::<item>.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::{DiscoveryManifest, PeerId};

// ─── Classification tiers (application-level, NOT on wire) ───────────────────

#[deprecated(
    since = "0.2.0",
    note = "replaced by howm-access group-based permissions"
)]
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

// ─── Intersection computation (§7.4) ──────────────────────────────────────────

/// Compute the active set from two manifests + a trust gate callback.
/// Returns sorted list of capability names that both peers agreed on.
///
/// The `trust_gate` closure is called for each candidate capability:
///   `trust_gate(capability_name, remote_peer_id) -> bool`
/// Return `true` to allow, `false` to exclude from the active set.
///
/// `core.session.heartbeat.1` always passes regardless of the trust gate (FR-3.4).
pub fn compute_intersection<F>(
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
    trust_gate: &F,
) -> Vec<String>
where
    F: Fn(&str, &PeerId) -> bool,
{
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
            // Trust gate check — heartbeat always passes (FR-3.4)
            if local_cap.name != "core.session.heartbeat.1"
                && !trust_gate(&local_cap.name, &remote.peer_id)
            {
                continue;
            }
            active.push(local_cap.name.clone());
            break;
        }
    }

    active.sort();
    active
}

/// Legacy compute_intersection using HashMap<String, TrustPolicy>.
/// Deprecated — use the closure-based version above.
#[deprecated(since = "0.2.0", note = "use closure-based compute_intersection")]
#[allow(deprecated)]
pub fn compute_intersection_legacy(
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
    trust_policies: &HashMap<String, TrustPolicy>,
) -> Vec<String> {
    let gate = |cap_name: &str, peer_id: &PeerId| -> bool {
        match trust_policies.get(cap_name) {
            Some(policy) => policy.evaluate(peer_id),
            None => true,
        }
    };
    compute_intersection(local, remote, &gate)
}

// ─── Trust gate (application-level) ───────────────────────────────────────────

/// Local trust gate configuration per capability.
/// Uses WireGuard public keys for peer identification.
#[derive(Debug, Clone)]
#[deprecated(
    since = "0.2.0",
    note = "replaced by howm-access group-based permissions"
)]
#[allow(deprecated)]
pub struct TrustPolicy {
    /// Default tier applied to peers not in overrides.
    pub default_tier: ClassificationTier,
    /// Per-peer overrides: WG public key -> tier.
    pub overrides: HashMap<PeerId, ClassificationTier>,
    /// Friends list: set of WireGuard public keys.
    pub friends: std::collections::HashSet<PeerId>,
}

#[allow(deprecated)]
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

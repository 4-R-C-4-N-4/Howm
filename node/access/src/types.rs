use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Well-known group UUIDs (BRD §7) ─────────────────────────────────────────

pub const GROUP_DEFAULT: Uuid = Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
]);

pub const GROUP_FRIENDS: Uuid = Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
]);

pub const GROUP_TRUSTED: Uuid = Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
]);

/// Peer identity — 32-byte WireGuard public key.
pub const PEER_ID_LEN: usize = 32;

// ── Group ────────────────────────────────────────────────────────────────────

/// A named group that carries capability access rules.
/// Built-in groups (howm.default, howm.friends, howm.trusted) have fixed
/// capability rules and cannot be deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub group_id: Uuid,
    pub name: String,
    pub built_in: bool,
    pub capabilities: Vec<CapabilityRule>,
    pub created_at: u64,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_group_id: Option<Uuid>,
}

// ── Capability rule ──────────────────────────────────────────────────────────

/// Per-capability access grant within a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRule {
    pub capability_name: String,
    pub allow: bool,
    /// Optional scope overrides — skipped in phase 1.
    pub rate_limit: Option<u64>,
    pub ttl: Option<u64>,
}

// ── Peer group membership ────────────────────────────────────────────────────

/// Links a peer (by WG public key) to a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerGroupMembership {
    pub peer_id: Vec<u8>,
    pub group_id: Uuid,
    pub assigned_at: u64,
    pub assigned_by: String,
}

// ── Permission result ────────────────────────────────────────────────────────

/// Outcome of `resolve_permission()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResult {
    Allow {
        rate_limit: Option<u64>,
        ttl: Option<u64>,
    },
    Deny,
}

impl PermissionResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PermissionResult::Allow { .. })
    }
}

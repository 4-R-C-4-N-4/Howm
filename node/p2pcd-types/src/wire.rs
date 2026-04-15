// p2pcd-types: Wire message types — MessageType, CloseReason, Role, ProtocolMessage.
// Extracted from lib.rs — all items remain accessible at p2pcd_types::<item>.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{DiscoveryManifest, ScopeParams};

// ─── Wire message types (§5.3.6 + Appendix B.12) ────────────────────────────

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
        /// Message type integer (6-30 for core, 36+ for app-defined).
        message_type: u64,
        /// Raw CBOR payload (the full message map minus the message_type key).
        payload: Vec<u8>,
    },
}

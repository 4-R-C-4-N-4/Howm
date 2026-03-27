use serde::{Deserialize, Serialize};

use crate::state::Activity;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerPresence {
    pub peer_id: String,
    pub activity: Activity,
    pub status: Option<String>,
    pub emoji: Option<String>,
    pub updated_at: u64,
    /// Internal: when we last received a gossip broadcast from this peer.
    /// Not included in API responses.
    #[serde(skip)]
    pub last_broadcast_received: u64,
}

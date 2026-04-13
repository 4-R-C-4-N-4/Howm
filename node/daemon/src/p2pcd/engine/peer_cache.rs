// engine/peer_cache.rs — SessionOutcome, PeerCacheEntry, and peer-cache impl methods.

use p2pcd::session::{Session, SessionState};
use p2pcd_types::PeerId;

use super::{short, unix_now, ProtocolEngine};

/// Peer cache TTL: entries older than this are ignored (re-negotiate).
const CACHE_TTL_SECS: u64 = 3600;

// ── Peer cache (Task 5.2) ────────────────────────────────────────────────────

/// Outcome of a completed negotiation, stored in the peer cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    Active,
    None,
    Denied,
}

/// Cached negotiation result for a peer, keyed by (peer_id, personal_hash).
/// If the remote peer's manifest hash changes, the cache entry is invalid.
#[derive(Debug, Clone)]
pub struct PeerCacheEntry {
    /// The remote peer's personal_hash at time of negotiation.
    pub personal_hash: Vec<u8>,
    pub last_outcome: SessionOutcome,
    pub timestamp: u64,
}

impl PeerCacheEntry {
    pub fn is_expired(&self) -> bool {
        unix_now().saturating_sub(self.timestamp) > CACHE_TTL_SECS
    }
}

impl ProtocolEngine {
    pub(crate) async fn record_session_outcome(&self, s: &Session) {
        let outcome = match &s.state {
            SessionState::Active => SessionOutcome::Active,
            SessionState::None => SessionOutcome::None,
            SessionState::Denied => SessionOutcome::Denied,
            _ => return,
        };
        let hash = s
            .remote_manifest
            .as_ref()
            .map(|m| m.personal_hash.clone())
            .unwrap_or_default();

        let mut cache = self.peer_cache.lock().await;
        let entry = PeerCacheEntry {
            personal_hash: hash.clone(),
            last_outcome: outcome,
            timestamp: unix_now(),
        };
        tracing::debug!(
            "engine: cache {} → {:?}",
            short(s.remote_peer_id),
            entry.last_outcome
        );
        cache.insert(s.remote_peer_id, entry);
    }

    pub async fn peer_cache_snapshot(&self) -> Vec<(PeerId, PeerCacheEntry)> {
        self.peer_cache
            .lock()
            .await
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }
}

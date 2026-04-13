use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use super::{encode_b64, BridgeState, PeerInfo, PeersQuery};

/// GET /p2pcd/bridge/peers — list active peers, optionally filtered by capability.
pub async fn handle_peers(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Query(query): Query<PeersQuery>,
) -> impl IntoResponse {
    let sessions = engine.active_sessions().await;

    let filtered: Vec<_> = sessions
        .into_iter()
        .filter(|s| {
            // Only expose sessions that are truly active — not Closed, Handshake, etc.
            if s.state != p2pcd::SessionState::Active {
                return false;
            }
            if let Some(ref cap) = query.capability {
                s.active_set.contains(cap)
            } else {
                true
            }
        })
        .collect();

    // Resolve WG addresses in parallel-ish (each is a quick local lookup or
    // cached `wg show` parse).
    let mut peers: Vec<PeerInfo> = Vec::with_capacity(filtered.len());
    for s in filtered {
        let wg_address = engine.peer_wg_ip(&s.peer_id).await;
        peers.push(PeerInfo {
            peer_id: encode_b64(&s.peer_id),
            capabilities: s.active_set,
            wg_address,
        });
    }

    (StatusCode::OK, Json(peers))
}

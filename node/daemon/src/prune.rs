use crate::peers::{self, TrustLevel};
use crate::state::AppState;
use crate::wireguard;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

pub async fn start_loop(state: AppState) {
    let interval = std::time::Duration::from_secs(3600); // 1 hour
    loop {
        tokio::time::sleep(interval).await;
        run_prune(&state).await;
    }
}

async fn run_prune(state: &AppState) {
    let prune_days = state.config.open_invite_prune_days;
    let cutoff_secs = prune_days * 86400;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut stale_peers = Vec::new();

    {
        let peers = state.peers.read().await;
        for peer in peers.iter() {
            if peer.trust == TrustLevel::Public
                && peer.last_seen > 0
                && (now - peer.last_seen) > cutoff_secs
            {
                stale_peers.push(peer.clone());
            }
        }
    }

    if stale_peers.is_empty() {
        return;
    }

    info!(
        "Pruning {} stale public peers (>{} days offline)",
        stale_peers.len(),
        prune_days
    );

    let wg_id = state.wg_container_id.read().await;

    for peer in &stale_peers {
        // Remove WG peer
        if let Some(ref container_id) = *wg_id {
            let _ = wireguard::remove_peer(
                container_id,
                &state.config.data_dir,
                &peer.wg_pubkey,
                &peer.node_id,
            )
            .await;
        }

        // Reclaim IP address
        let _ = wireguard::reclaim_address(&state.config.data_dir, &peer.wg_address);

        info!(
            "Pruned stale public peer: {} ({})",
            peer.name, peer.wg_address
        );
    }

    // Remove from peers list
    {
        let stale_pubkeys: Vec<&str> = stale_peers.iter().map(|p| p.wg_pubkey.as_str()).collect();
        let mut peers = state.peers.write().await;
        peers.retain(|p| !stale_pubkeys.contains(&p.wg_pubkey.as_str()));
        let _ = peers::save(&state.config.data_dir, &peers);
    }
}

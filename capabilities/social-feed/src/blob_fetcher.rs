// Blob fetch orchestration for inbound peer post attachments.
//
// When a peer post arrives with attachments, we:
// 1. Insert pending blob_transfer records
// 2. Trigger blob_request for each via the daemon bridge
// 3. Poll blob_status until complete (or give up)
//
// On startup, resume any pending/fetching transfers from the DB.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use p2pcd::bridge_client::BridgeClient;
use tracing::{info, warn};

use crate::db::FeedDb;
use crate::posts::Post;

/// Global monotonic transfer ID counter.
static TRANSFER_ID: AtomicU64 = AtomicU64::new(1);

fn next_transfer_id() -> u64 {
    TRANSFER_ID.fetch_add(1, Ordering::Relaxed)
}

/// Decode a base64-encoded peer_id string to a 32-byte array.
fn decode_peer_id(peer_id_b64: &str) -> Result<[u8; 32], String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let bytes = STANDARD
        .decode(peer_id_b64)
        .map_err(|e| format!("bad base64: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(id)
}

/// Decode a hex-encoded blob hash to a 32-byte array.
fn decode_blob_hash(hex_hash: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_hash).map_err(|e| format!("bad hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

/// Initiate blob fetches for all attachments on a newly ingested peer post.
///
/// Inserts blob_transfer records, then spawns async tasks to request + poll
/// each blob from the posting peer.
pub async fn fetch_post_blobs(db: FeedDb, bridge: BridgeClient, post: &Post) {
    if post.attachments.is_empty() {
        return;
    }

    // Extract peer_id from origin (format: "peer:<base64>")
    let peer_id_b64 = match post.origin.strip_prefix("peer:") {
        Some(id) => id.to_string(),
        None => {
            warn!(
                "fetch_post_blobs called for non-peer post {} (origin={})",
                post.id, post.origin
            );
            return;
        }
    };

    let peer_id = match decode_peer_id(&peer_id_b64) {
        Ok(id) => id,
        Err(e) => {
            warn!("cannot decode peer_id for post {}: {}", post.id, e);
            return;
        }
    };

    // Insert transfer records for each attachment
    for att in &post.attachments {
        if let Err(e) = db.insert_blob_transfer(&post.id, &att.blob_id) {
            warn!(
                "failed to insert blob_transfer for post {} blob {}: {}",
                post.id, att.blob_id, e
            );
        }
    }

    // Spawn a task per attachment to request + poll
    for att in &post.attachments {
        let db = db.clone();
        let bridge = bridge.clone();
        let post_id = post.id.clone();
        let blob_id = att.blob_id.clone();
        let total_size = att.size;

        tokio::spawn(async move {
            fetch_single_blob(db, bridge, &post_id, &blob_id, total_size, &peer_id).await;
        });
    }
}

/// Resume any pending/fetching transfers found in the DB at startup.
///
/// For each active transfer, look up the post origin to find the peer,
/// then spawn a fetch task.
pub async fn resume_active_transfers(db: FeedDb, bridge: BridgeClient) {
    let transfers = match db.get_active_transfers() {
        Ok(t) => t,
        Err(e) => {
            warn!("failed to load active transfers for resume: {}", e);
            return;
        }
    };

    if transfers.is_empty() {
        return;
    }

    info!("resuming {} active blob transfers", transfers.len());

    for transfer in transfers {
        let origin = match db.get_post_origin(&transfer.post_id) {
            Ok(Some(o)) => o,
            Ok(None) => {
                warn!(
                    "post {} not found for transfer {}, marking failed",
                    transfer.post_id, transfer.blob_id
                );
                let _ = db.update_blob_transfer(
                    &transfer.post_id,
                    &transfer.blob_id,
                    "failed",
                    transfer.bytes_received,
                );
                continue;
            }
            Err(e) => {
                warn!("failed to get origin for post {}: {}", transfer.post_id, e);
                continue;
            }
        };

        let peer_id_b64 = match origin.strip_prefix("peer:") {
            Some(id) => id,
            None => {
                warn!(
                    "transfer {} has non-peer origin {}, skipping",
                    transfer.blob_id, origin
                );
                continue;
            }
        };

        let peer_id = match decode_peer_id(peer_id_b64) {
            Ok(id) => id,
            Err(e) => {
                warn!("bad peer_id for transfer {}: {}", transfer.blob_id, e);
                continue;
            }
        };

        let db = db.clone();
        let bridge = bridge.clone();

        tokio::spawn(async move {
            fetch_single_blob(
                db,
                bridge,
                &transfer.post_id,
                &transfer.blob_id,
                transfer.total_size,
                &peer_id,
            )
            .await;
        });
    }
}

/// Fetch a single blob: request from peer, poll until complete or failed.
async fn fetch_single_blob(
    db: FeedDb,
    bridge: BridgeClient,
    post_id: &str,
    blob_id: &str,
    total_size: u64,
    peer_id: &[u8; 32],
) {
    let hash = match decode_blob_hash(blob_id) {
        Ok(h) => h,
        Err(e) => {
            warn!("bad blob hash {}: {}", blob_id, e);
            let _ = db.update_blob_transfer(post_id, blob_id, "failed", 0);
            return;
        }
    };

    // Check if blob already exists locally (e.g. from a previous partial transfer)
    match bridge.blob_status(&hash).await {
        Ok(status) if status.exists => {
            info!(
                "blob {} already exists locally, marking complete",
                &blob_id[..8.min(blob_id.len())]
            );
            let size = status.size.unwrap_or(total_size);
            let _ = db.update_blob_transfer(post_id, blob_id, "complete", size);
            check_post_complete(&db, post_id).await;
            return;
        }
        _ => {}
    }

    // Mark as fetching
    let _ = db.update_blob_transfer(post_id, blob_id, "fetching", 0);

    // Request the blob from the peer
    let transfer_id = next_transfer_id();
    if let Err(e) = bridge.blob_request(peer_id, &hash, transfer_id).await {
        warn!(
            "blob_request failed for {} from peer: {}",
            &blob_id[..8.min(blob_id.len())],
            e
        );
        let _ = db.update_blob_transfer(post_id, blob_id, "failed", 0);
        return;
    }

    info!(
        "requested blob {} (transfer {}), polling for completion",
        &blob_id[..8.min(blob_id.len())],
        transfer_id
    );

    // Poll until complete, failed, or timeout
    let poll_interval = Duration::from_secs(2);
    let max_polls = 150; // 5 minutes max (150 * 2s)

    for _ in 0..max_polls {
        tokio::time::sleep(poll_interval).await;

        match bridge.blob_status(&hash).await {
            Ok(status) if status.exists => {
                let size = status.size.unwrap_or(total_size);
                let _ = db.update_blob_transfer(post_id, blob_id, "complete", size);
                info!(
                    "blob {} complete ({} bytes)",
                    &blob_id[..8.min(blob_id.len())],
                    size
                );
                check_post_complete(&db, post_id).await;
                return;
            }
            Ok(_) => {
                // Not yet available, keep polling
            }
            Err(e) => {
                warn!(
                    "blob_status check failed for {}: {}",
                    &blob_id[..8.min(blob_id.len())],
                    e
                );
                // Don't fail immediately — transient network issue
            }
        }
    }

    // Timeout
    warn!(
        "blob {} timed out after {} polls",
        &blob_id[..8.min(blob_id.len())],
        max_polls
    );
    let _ = db.update_blob_transfer(post_id, blob_id, "failed", 0);
}

/// Check if all blobs for a post are complete, and log if so.
/// In the future this could emit a post.media_ready event.
async fn check_post_complete(db: &FeedDb, post_id: &str) {
    match db.are_all_transfers_complete(post_id) {
        Ok(true) => {
            info!("all blobs complete for post {}", post_id);
            // TODO: emit post.media_ready event via bridge broadcast
        }
        Ok(false) => {} // still waiting
        Err(e) => warn!(
            "failed to check transfer completeness for {}: {}",
            post_id, e
        ),
    }
}

//! Profile sync — push/receive profile metadata between peers.
//!
//! Profile metadata (name, bio, avatar hash, homepage status) is pushed to peers:
//! - On boot (background sweep of all connected peers)
//! - On profile update (broadcast to all peers)
//!
//! Peers cache received metadata locally in `<data_dir>/profile_cache/`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::{peers::Peer, profile, state::AppState};

/// Profile metadata exchanged between peers.
/// Lightweight — no binary data, just text + avatar hash.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProfileMeta {
    pub node_id: String,
    pub name: String,
    pub bio: String,
    /// SHA-256 hash of the avatar file (hex). Peers use this to know if they need to re-fetch.
    pub avatar_hash: Option<String>,
    pub has_homepage: bool,
}

/// Cached peer profile metadata (what we've received from other nodes).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PeerProfileCache {
    pub node_id: String,
    pub name: String,
    pub bio: String,
    pub avatar_hash: Option<String>,
    pub has_homepage: bool,
    /// Timestamp of last update.
    pub updated_at: u64,
}

fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("profile_cache")
}

fn cache_path(data_dir: &Path, node_id: &str) -> PathBuf {
    // Sanitize node_id for filesystem safety
    let safe_id: String = node_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir(data_dir).join(format!("{}.json", safe_id))
}

/// Build our own ProfileMeta from current state.
pub fn build_our_meta(state: &AppState, profile: &profile::Profile) -> ProfileMeta {
    let avatar_hash = profile.avatar.as_ref().and_then(|avatar_name| {
        let avatar_path = profile::ProfilePaths::new(&state.config.data_dir)
            .dir
            .join(avatar_name);
        std::fs::read(&avatar_path)
            .ok()
            .map(|data| format!("{:x}", Sha256::digest(&data)))
    });

    ProfileMeta {
        node_id: state.identity.node_id.clone(),
        name: profile.name.clone(),
        bio: profile.bio.clone(),
        avatar_hash,
        has_homepage: profile.homepage.is_some(),
    }
}

/// Save received peer profile metadata to cache.
pub fn cache_peer_profile(data_dir: &Path, meta: &ProfileMeta) -> anyhow::Result<()> {
    let dir = cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let cached = PeerProfileCache {
        node_id: meta.node_id.clone(),
        name: meta.name.clone(),
        bio: meta.bio.clone(),
        avatar_hash: meta.avatar_hash.clone(),
        has_homepage: meta.has_homepage,
        updated_at: now,
    };

    let path = cache_path(data_dir, &meta.node_id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&cached)?)?;
    std::fs::rename(&tmp, &path)?;
    debug!("Cached profile for peer {}", meta.node_id);
    Ok(())
}

/// Load cached profile for a specific peer. Returns None if not cached.
pub fn load_cached_profile(data_dir: &Path, node_id: &str) -> Option<PeerProfileCache> {
    let path = cache_path(data_dir, node_id);
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Load all cached peer profiles.
pub fn load_all_cached(data_dir: &Path) -> Vec<PeerProfileCache> {
    let dir = cache_dir(data_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return vec![],
    };

    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str::<PeerProfileCache>(&text).ok()
        })
        .collect()
}

/// Push our profile metadata to a single peer's daemon.
async fn push_to_peer(meta: &ProfileMeta, peer: &Peer) -> Result<(), String> {
    let url = format!("http://{}:{}/profile/sync", peer.wg_address, peer.port);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(&url)
        .json(meta)
        .send()
        .await
        .map_err(|e| format!("push to {} failed: {}", peer.name, e))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "peer {} returned status {}",
            peer.name,
            resp.status()
        ))
    }
}

/// Broadcast our profile metadata to all connected peers.
/// Errors are logged but don't stop the broadcast.
pub async fn broadcast_profile(state: &AppState) {
    let profile = state.profile.read().await;
    let meta = build_our_meta(state, &profile);
    drop(profile);

    let peers = state.peers.read().await.clone();
    if peers.is_empty() {
        return;
    }

    info!("Broadcasting profile to {} peers", peers.len());
    for peer in &peers {
        match push_to_peer(&meta, peer).await {
            Ok(()) => debug!("Profile pushed to {}", peer.name),
            Err(e) => debug!("Profile push skipped for {}: {}", peer.name, e),
        }
    }
}

/// Background task: sync our profile to all peers on boot.
/// Runs once after a short delay to let WG tunnels establish.
pub async fn boot_sync(state: AppState) {
    // Wait for WG tunnels to come up
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        debug!("Boot profile sync skipped — WG not active");
        return;
    }

    info!("Running boot profile sync");
    broadcast_profile(&state).await;

    // Also fetch profiles from peers
    let peers = state.peers.read().await.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    for peer in &peers {
        let url = format!("http://{}:{}/profile", peer.wg_address, peer.port);
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let meta = ProfileMeta {
                        node_id: peer.node_id.clone(),
                        name: body["name"].as_str().unwrap_or(&peer.name).to_string(),
                        bio: body["bio"].as_str().unwrap_or("").to_string(),
                        avatar_hash: None, // We don't get hash from GET /profile
                        has_homepage: body["has_homepage"].as_bool().unwrap_or(false),
                    };
                    let _ = cache_peer_profile(&state.config.data_dir, &meta);
                }
            }
            Ok(_) => debug!("Peer {} profile fetch: non-200", peer.name),
            Err(e) => debug!("Peer {} profile fetch failed: {}", peer.name, e),
        }
    }
}

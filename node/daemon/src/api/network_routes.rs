use crate::{error::AppError, peers::TrustLevel, state::AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

#[derive(Deserialize, Default)]
pub struct FeedQuery {
    pub trust: Option<String>,
}

pub async fn network_capabilities(State(state): State<AppState>) -> Json<Value> {
    let index = state.network_index.read().await;
    Json(json!({
        "capabilities": index.capabilities,
        "last_updated": index.last_updated,
    }))
}

pub async fn find_capability_providers(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let index = state.network_index.read().await;
    let providers = index.capabilities.get(&name).cloned().unwrap_or_default();
    Ok(Json(json!({ "providers": providers })))
}

pub async fn network_feed(
    Query(query): Query<FeedQuery>,
    State(state): State<AppState>,
) -> Json<Value> {
    let friends_only = query.trust.as_deref() == Some("friend");

    let mut all_posts: Vec<serde_json::Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let timeout = std::time::Duration::from_millis(state.config.peer_timeout_ms);

    // 1. Collect local posts from the social.feed capability
    {
        let caps = state.capabilities.read().await;
        if let Some(local_feed) = caps.iter().find(|c| c.name == "social.feed") {
            let url = format!("http://localhost:{}/feed", local_feed.port);
            let client = reqwest::Client::builder().timeout(timeout).build();
            if let Ok(client) = client {
                match client.get(&url).send().await {
                    Ok(resp) => {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if let Some(posts) = body["posts"].as_array() {
                                all_posts.extend(posts.iter().cloned());
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to fetch local feed: {}", e);
                        errors.push(format!("local feed unavailable: {}", e));
                    }
                }
            }
        }
    }

    // 2. Collect posts from peers (via WG tunnel), optionally filtered by trust
    let peers = state.peers.read().await.clone();
    let filtered_peers: Vec<_> = if friends_only {
        peers.iter().filter(|p| p.trust == TrustLevel::Friend).collect()
    } else {
        peers.iter().collect()
    };
    for peer in &filtered_peers {
        let url = format!("http://{}:{}/cap/social/feed", peer.wg_address, peer.port);
        let client = reqwest::Client::builder().timeout(timeout).build();
        if let Ok(client) = client {
            match client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if let Some(posts) = body["posts"].as_array() {
                            all_posts.extend(posts.iter().cloned());
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch feed from peer {}: {}", peer.name, e);
                    errors.push(format!("peer {} unreachable", peer.name));
                }
            }
        }
    }

    // 3. Deduplicate by id
    let mut seen_ids = std::collections::HashSet::new();
    all_posts.retain(|p| {
        if let Some(id) = p["id"].as_str() {
            seen_ids.insert(id.to_string())
        } else {
            true
        }
    });

    // 4. Sort by timestamp descending
    all_posts.sort_by(|a, b| {
        let ta = a["timestamp"].as_u64().unwrap_or(0);
        let tb = b["timestamp"].as_u64().unwrap_or(0);
        tb.cmp(&ta)
    });

    Json(json!({
        "posts": all_posts,
        "errors": errors,
    }))
}

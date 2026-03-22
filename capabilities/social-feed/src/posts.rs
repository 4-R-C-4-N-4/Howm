use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Post {
    pub id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: u64,
    /// Where this post came from: "local" for our own, "peer:<b64_peer_id>" for received.
    #[serde(default = "default_origin")]
    pub origin: String,
}

fn default_origin() -> String {
    "local".to_string()
}

pub const MAX_CONTENT_LEN: usize = 5000;

// ── Local posts (posts we created) ──────────────────────────────────────────

pub fn load(data_dir: &Path) -> anyhow::Result<Vec<Post>> {
    load_file(&data_dir.join("posts.json"))
}

pub fn save(data_dir: &Path, posts: &[Post]) -> anyhow::Result<()> {
    save_file(&data_dir.join("posts.json"), posts)
}

pub fn create(
    data_dir: &Path,
    content: String,
    author_id: String,
    author_name: String,
) -> anyhow::Result<Post> {
    if content.len() > MAX_CONTENT_LEN {
        return Err(anyhow::anyhow!(
            "content too long (max {} chars)",
            MAX_CONTENT_LEN
        ));
    }
    let post = Post {
        id: Uuid::new_v4().to_string(),
        author_id,
        author_name,
        content,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        origin: "local".to_string(),
    };
    let mut posts = load(data_dir)?;
    posts.push(post.clone());
    save(data_dir, &posts)?;
    Ok(post)
}

// ── Peer posts (received from other nodes) ──────────────────────────────────

pub fn load_peer_posts(data_dir: &Path) -> anyhow::Result<Vec<Post>> {
    load_file(&data_dir.join("peer_posts.json"))
}

pub fn save_peer_posts(data_dir: &Path, posts: &[Post]) -> anyhow::Result<()> {
    save_file(&data_dir.join("peer_posts.json"), posts)
}

/// Ingest a post received from a peer. Returns true if it was new (not a duplicate).
///
/// Deduplication: skips if a post with the same `id` already exists.
/// Sets origin to "peer:<peer_id_b64>" if not already set.
pub fn ingest_peer_post(
    data_dir: &Path,
    mut post: Post,
    peer_id_b64: &str,
) -> anyhow::Result<bool> {
    if post.content.len() > MAX_CONTENT_LEN {
        return Err(anyhow::anyhow!("peer post too long, rejected"));
    }

    // Ensure origin is set
    if post.origin == "local" || post.origin.is_empty() {
        post.origin = format!("peer:{}", peer_id_b64);
    }

    let mut posts = load_peer_posts(data_dir)?;

    // Dedup by post ID — also check local posts to prevent echoes
    if posts.iter().any(|p| p.id == post.id) {
        return Ok(false);
    }
    let local = load(data_dir)?;
    if local.iter().any(|p| p.id == post.id) {
        return Ok(false);
    }

    posts.push(post);
    save_peer_posts(data_dir, &posts)?;
    Ok(true)
}

// ── Deletion ────────────────────────────────────────────────────────────────

/// Delete a local post by ID. Returns true if found and removed.
/// Only local posts can be deleted (you can't delete someone else's post).
pub fn delete(data_dir: &Path, post_id: &str) -> anyhow::Result<bool> {
    let mut posts = load(data_dir)?;
    let before = posts.len();
    posts.retain(|p| p.id != post_id);
    if posts.len() == before {
        return Ok(false); // not found
    }
    save(data_dir, &posts)?;
    Ok(true)
}

/// Delete a peer post by ID. Used for moderation/cleanup.
pub fn delete_peer_post(data_dir: &Path, post_id: &str) -> anyhow::Result<bool> {
    let mut posts = load_peer_posts(data_dir)?;
    let before = posts.len();
    posts.retain(|p| p.id != post_id);
    if posts.len() == before {
        return Ok(false);
    }
    save_peer_posts(data_dir, &posts)?;
    Ok(true)
}

// ── Filtered loading ────────────────────────────────────────────────────────

/// Load posts from a specific peer (by peer_id base64 string).
pub fn load_peer_feed(data_dir: &Path, peer_id: &str) -> anyhow::Result<Vec<Post>> {
    let origin_prefix = format!("peer:{}", peer_id);
    let mut posts: Vec<Post> = load_peer_posts(data_dir)?
        .into_iter()
        .filter(|p| p.origin == origin_prefix)
        .collect();
    posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(posts)
}

/// Load only local posts (your own feed), sorted newest first.
pub fn load_mine(data_dir: &Path) -> anyhow::Result<Vec<Post>> {
    let mut posts = load(data_dir)?;
    posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(posts)
}

// ── Merged feed ─────────────────────────────────────────────────────────────

/// Load all posts (local + peer), deduplicated by id, sorted newest first.
pub fn load_all(data_dir: &Path) -> anyhow::Result<Vec<Post>> {
    let mut all = load(data_dir)?;
    let peer = load_peer_posts(data_dir)?;

    // Dedup: only add peer posts whose id doesn't already exist
    let local_ids: std::collections::HashSet<String> = all.iter().map(|p| p.id.clone()).collect();
    for p in peer {
        if !local_ids.contains(&p.id) {
            all.push(p);
        }
    }

    all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(all)
}

// ── File helpers ────────────────────────────────────────────────────────────

fn load_file(path: &Path) -> anyhow::Result<Vec<Post>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

fn save_file(path: &Path, posts: &[Post]) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(posts)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_and_load() {
        let dir = TempDir::new().unwrap();
        let post = create(dir.path(), "hello".into(), "alice".into(), "Alice".into()).unwrap();
        assert_eq!(post.origin, "local");

        let posts = load(dir.path()).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, post.id);
    }

    #[test]
    fn ingest_peer_post_dedup() {
        let dir = TempDir::new().unwrap();
        let post = Post {
            id: "test-uuid-1".into(),
            author_id: "bob".into(),
            author_name: "Bob".into(),
            content: "peer post".into(),
            timestamp: 1000,
            origin: "local".into(), // will be overwritten
        };

        // First ingest: should succeed
        let new = ingest_peer_post(dir.path(), post.clone(), "AAAA").unwrap();
        assert!(new);

        // Second ingest: duplicate
        let dup = ingest_peer_post(dir.path(), post.clone(), "AAAA").unwrap();
        assert!(!dup);

        // Verify origin was set
        let peers = load_peer_posts(dir.path()).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].origin, "peer:AAAA");
    }

    #[test]
    fn ingest_peer_post_rejects_echo() {
        let dir = TempDir::new().unwrap();
        // Create a local post
        let local = create(dir.path(), "my post".into(), "me".into(), "Me".into()).unwrap();

        // Try to ingest the same post as if from a peer
        let echo = Post {
            id: local.id.clone(),
            author_id: "me".into(),
            author_name: "Me".into(),
            content: "my post".into(),
            timestamp: local.timestamp,
            origin: "local".into(),
        };
        let new = ingest_peer_post(dir.path(), echo, "BBBB").unwrap();
        assert!(!new); // rejected as duplicate
    }

    #[test]
    fn load_all_merges_and_sorts() {
        let dir = TempDir::new().unwrap();
        // Create local post
        let local = create(dir.path(), "local".into(), "me".into(), "Me".into()).unwrap();

        // Ingest peer post with a newer timestamp
        let peer = Post {
            id: "peer-1".into(),
            author_id: "bob".into(),
            author_name: "Bob".into(),
            content: "from peer".into(),
            timestamp: local.timestamp + 100, // definitely newer
            origin: "peer:CCCC".into(),
        };
        ingest_peer_post(dir.path(), peer, "CCCC").unwrap();

        let all = load_all(dir.path()).unwrap();
        assert_eq!(all.len(), 2);
        // Newest first
        assert_eq!(all[0].id, "peer-1");
        assert!(all[0].timestamp >= all[1].timestamp);
    }

    #[test]
    fn content_too_long_rejected() {
        let dir = TempDir::new().unwrap();
        let long = "x".repeat(MAX_CONTENT_LEN + 1);
        assert!(create(dir.path(), long.clone(), "a".into(), "A".into()).is_err());

        let post = Post {
            id: "long".into(),
            author_id: "b".into(),
            author_name: "B".into(),
            content: long,
            timestamp: 0,
            origin: "peer:X".into(),
        };
        assert!(ingest_peer_post(dir.path(), post, "X").is_err());
    }

    #[test]
    fn delete_local_post() {
        let dir = TempDir::new().unwrap();
        let post = create(dir.path(), "deleteme".into(), "a".into(), "A".into()).unwrap();
        assert!(delete(dir.path(), &post.id).unwrap());
        assert!(!delete(dir.path(), &post.id).unwrap()); // already gone
        assert!(load(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn delete_peer_post_works() {
        let dir = TempDir::new().unwrap();
        let post = Post {
            id: "peer-del-1".into(),
            author_id: "bob".into(),
            author_name: "Bob".into(),
            content: "removable".into(),
            timestamp: 1000,
            origin: "peer:DDDD".into(),
        };
        ingest_peer_post(dir.path(), post, "DDDD").unwrap();
        assert!(delete_peer_post(dir.path(), "peer-del-1").unwrap());
        assert!(load_peer_posts(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn load_mine_only_local() {
        let dir = TempDir::new().unwrap();
        create(dir.path(), "local1".into(), "me".into(), "Me".into()).unwrap();
        let peer = Post {
            id: "peer-mine-test".into(),
            author_id: "bob".into(),
            author_name: "Bob".into(),
            content: "peer".into(),
            timestamp: 9999,
            origin: "peer:EEEE".into(),
        };
        ingest_peer_post(dir.path(), peer, "EEEE").unwrap();

        let mine = load_mine(dir.path()).unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].origin, "local");
    }

    #[test]
    fn load_peer_feed_filters() {
        let dir = TempDir::new().unwrap();
        let p1 = Post {
            id: "pf-1".into(),
            author_id: "a".into(),
            author_name: "A".into(),
            content: "from alice".into(),
            timestamp: 100,
            origin: "peer:ALICE".into(),
        };
        let p2 = Post {
            id: "pf-2".into(),
            author_id: "b".into(),
            author_name: "B".into(),
            content: "from bob".into(),
            timestamp: 200,
            origin: "peer:BOB".into(),
        };
        ingest_peer_post(dir.path(), p1, "ALICE").unwrap();
        ingest_peer_post(dir.path(), p2, "BOB").unwrap();

        let alice = load_peer_feed(dir.path(), "ALICE").unwrap();
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].id, "pf-1");

        let bob = load_peer_feed(dir.path(), "BOB").unwrap();
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].id, "pf-2");

        let nobody = load_peer_feed(dir.path(), "NOBODY").unwrap();
        assert!(nobody.is_empty());
    }

    #[test]
    fn default_origin_compat() {
        // Simulate old posts.json without origin field
        let dir = TempDir::new().unwrap();
        let json =
            r#"[{"id":"old","author_id":"a","author_name":"A","content":"hi","timestamp":1}]"#;
        std::fs::write(dir.path().join("posts.json"), json).unwrap();
        let posts = load(dir.path()).unwrap();
        assert_eq!(posts[0].origin, "local"); // default_origin kicks in
    }
}

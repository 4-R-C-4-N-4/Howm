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
}

pub const MAX_CONTENT_LEN: usize = 5000;

pub fn load(data_dir: &Path) -> anyhow::Result<Vec<Post>> {
    let path = data_dir.join("posts.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save(data_dir: &Path, posts: &[Post]) -> anyhow::Result<()> {
    let path = data_dir.join("posts.json");
    let tmp = data_dir.join("posts.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(posts)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
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
    };
    let mut posts = load(data_dir)?;
    posts.push(post.clone());
    save(data_dir, &posts)?;
    Ok(post)
}

use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeIdentity {
    pub node_id: String,
    pub name: String,
    pub created: u64,
    #[serde(default)]
    pub wg_pubkey: Option<String>,
    #[serde(default)]
    pub wg_address: Option<String>,   // 10.47.x.y
    #[serde(default)]
    pub wg_endpoint: Option<String>,  // public addr:port for peers to reach us
}

pub fn load_or_create(data_dir: &Path, name: Option<String>) -> anyhow::Result<NodeIdentity> {
    let path = data_dir.join("node.json");
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        let identity: NodeIdentity = serde_json::from_str(&text)?;
        return Ok(identity);
    }
    let hostname = name.unwrap_or_else(|| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string())
    });
    let identity = NodeIdentity {
        node_id: Uuid::new_v4().to_string(),
        name: hostname,
        created: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        wg_pubkey: None,
        wg_address: None,
        wg_endpoint: None,
    };
    write_identity(data_dir, &identity)?;
    Ok(identity)
}

pub fn write_identity(data_dir: &Path, identity: &NodeIdentity) -> anyhow::Result<()> {
    let path = data_dir.join("node.json");
    let tmp = data_dir.join("node.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(identity)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Peer {
    pub node_id: String,
    pub name: String,
    pub wg_pubkey: String,        // WG public key (identity)
    pub wg_address: String,       // 10.47.x.y (how to reach them on wg0)
    pub wg_endpoint: String,      // public addr:port
    pub port: u16,                // daemon API port (on their WG address)
    pub last_seen: u64,

    // Legacy field — ignored on load, not serialized
    #[serde(default, skip_serializing)]
    pub address: Option<String>,
}

pub fn load(data_dir: &Path) -> anyhow::Result<Vec<Peer>> {
    let path = data_dir.join("peers.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save(data_dir: &Path, peers: &[Peer]) -> anyhow::Result<()> {
    let path = data_dir.join("peers.json");
    let tmp = data_dir.join("peers.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(peers)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

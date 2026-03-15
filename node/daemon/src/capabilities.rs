use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CapStatus {
    Running,
    Stopped,
    Error(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CapabilityEntry {
    pub name: String,
    pub version: String,
    pub port: u16,
    pub container_id: String,
    pub image: String,
    pub status: CapStatus,
    pub visibility: String,
}

pub fn load(data_dir: &Path) -> anyhow::Result<Vec<CapabilityEntry>> {
    let path = data_dir.join("capabilities.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

/// Return the next port >= `start` that isn't already used by an installed capability.
pub fn next_available_port(caps: &[CapabilityEntry], start: u16) -> u16 {
    let used: std::collections::HashSet<u16> = caps.iter().map(|c| c.port).collect();
    let mut port = start;
    while used.contains(&port) {
        port += 1;
    }
    port
}

pub fn save(data_dir: &Path, caps: &[CapabilityEntry]) -> anyhow::Result<()> {
    let path = data_dir.join("capabilities.json");
    let tmp = data_dir.join("capabilities.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(caps)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

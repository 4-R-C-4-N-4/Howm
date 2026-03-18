use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiManifest {
    pub label: String,
    pub icon: Option<String>,
    pub entry: String,
    #[serde(default = "default_ui_style")]
    pub style: String,
}

fn default_ui_style() -> String {
    "iframe".to_string()
}

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
    pub pid: Option<u32>,
    pub binary_path: String,
    pub manifest_path: String,
    pub data_dir: String,
    pub status: CapStatus,
    pub visibility: String,
    pub ui: Option<UiManifest>,
}

/// Capability manifest read from manifest.json on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    /// Relative path to the executable binary within the capability directory.
    pub binary: String,
    pub port: Option<u16>,
    pub api: Option<ApiManifest>,
    pub permissions: Option<PermissionsManifest>,
    pub resources: Option<ResourcesManifest>,
    pub ui: Option<UiManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiManifest {
    pub base_path: Option<String>,
    pub endpoints: Option<Vec<EndpointManifest>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointManifest {
    pub name: String,
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsManifest {
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesManifest {
    pub cpu: Option<String>,
    pub memory: Option<String>,
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

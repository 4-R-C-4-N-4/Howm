use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuthKey {
    pub key: String,    // full key, e.g. "psk-abc123..."
    pub prefix: String, // first 8 chars for display
}

pub fn load_keys(data_dir: &Path) -> anyhow::Result<Vec<AuthKey>> {
    let path = data_dir.join("auth_keys.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save_keys(data_dir: &Path, keys: &[AuthKey]) -> anyhow::Result<()> {
    let path = data_dir.join("auth_keys.json");
    let tmp = data_dir.join("auth_keys.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(keys)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn add_key(data_dir: &Path, key: &str) -> anyhow::Result<AuthKey> {
    let mut keys = load_keys(data_dir)?;
    let prefix = key.chars().take(8).collect::<String>();
    let auth_key = AuthKey {
        key: key.to_string(),
        prefix,
    };
    keys.push(auth_key.clone());
    save_keys(data_dir, &keys)?;
    Ok(auth_key)
}

pub fn remove_key(data_dir: &Path, prefix: &str) -> anyhow::Result<bool> {
    let mut keys = load_keys(data_dir)?;
    let len_before = keys.len();
    keys.retain(|k| !k.prefix.starts_with(prefix));
    if keys.len() == len_before {
        return Ok(false);
    }
    save_keys(data_dir, &keys)?;
    Ok(true)
}

pub fn validate_key(data_dir: &Path, key: &str) -> anyhow::Result<bool> {
    let keys = load_keys(data_dir)?;
    Ok(keys.iter().any(|k| k.key == key))
}

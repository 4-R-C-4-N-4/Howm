use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{error::AppError, state::AppState};

pub async fn get_node_settings(State(state): State<AppState>) -> Json<Value> {
    let cfg = &state.config;
    Json(json!({
        "port": cfg.port,
        "data_dir": cfg.data_dir.to_string_lossy(),
        "name": cfg.name,
        "wg_enabled": cfg.wg_enabled(),
        "wg_port": cfg.wg_port,
        "wg_endpoint": cfg.wg_endpoint,
        "wg_address": cfg.wg_address,
        "peer_timeout_ms": cfg.peer_timeout_ms,
        "discovery_interval_s": cfg.discovery_interval_s,
        "invite_ttl_s": cfg.invite_ttl_s,
        "dev": cfg.dev,
        "debug": cfg.debug,
    }))
}

pub async fn get_identity(State(state): State<AppState>) -> Json<Value> {
    let id = &state.identity;
    Json(json!({
        "node_id": id.node_id,
        "name": id.name,
        "wg_pubkey": id.wg_pubkey,
        "wg_address": id.wg_address,
        "wg_endpoint": id.wg_endpoint,
    }))
}

pub async fn get_p2pcd_config(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let toml_path = state.config.data_dir.join("p2pcd-peer.toml");
    if !toml_path.exists() {
        return Ok(Json(json!(null)));
    }
    let content = std::fs::read_to_string(&toml_path)
        .map_err(|e| AppError::Internal(format!("Failed to read p2pcd-peer.toml: {e}")))?;
    let config: p2pcd_types::config::PeerConfig = toml::from_str(&content)
        .map_err(|e| AppError::Internal(format!("Failed to parse p2pcd-peer.toml: {e}")))?;
    Ok(Json(serde_json::to_value(config).unwrap_or(json!(null))))
}

pub async fn update_p2pcd_config(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let toml_path = state.config.data_dir.join("p2pcd-peer.toml");

    // Load current config as JSON for merging
    let current: Value = if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path)
            .map_err(|e| AppError::Internal(format!("Failed to read p2pcd-peer.toml: {e}")))?;
        let cfg: p2pcd_types::config::PeerConfig = toml::from_str(&content)
            .map_err(|e| AppError::Internal(format!("Failed to parse p2pcd-peer.toml: {e}")))?;
        serde_json::to_value(cfg).unwrap_or(json!({}))
    } else {
        let default = p2pcd_types::config::PeerConfig::generate_default(&state.config.data_dir);
        serde_json::to_value(default).unwrap_or(json!({}))
    };

    let merged = merge_json(current, req);

    // Validate by round-tripping through PeerConfig
    let new_config: p2pcd_types::config::PeerConfig = serde_json::from_value(merged.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid config: {e}")))?;

    let toml_str = toml::to_string_pretty(&new_config)
        .map_err(|e| AppError::Internal(format!("Failed to serialize config: {e}")))?;

    let tmp = toml_path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml_str)
        .map_err(|e| AppError::Internal(format!("Failed to write config: {e}")))?;
    std::fs::rename(&tmp, &toml_path)
        .map_err(|e| AppError::Internal(format!("Failed to rename config: {e}")))?;

    tracing::info!("Updated p2pcd-peer.toml (changes take effect on restart)");
    Ok(Json(merged))
}

fn merge_json(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                let entry = base_map.entry(k).or_insert(Value::Null);
                *entry = merge_json(std::mem::replace(entry, Value::Null), v);
            }
            Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}

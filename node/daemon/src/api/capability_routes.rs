use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{info, warn};

use crate::{
    capabilities::{self, CapStatus, CapabilityEntry, CapabilityManifest},
    error::AppError,
    executor,
    state::AppState,
};

// ── List ─────────────────────────────────────────────────────────────────────

pub async fn list_capabilities(State(state): State<AppState>) -> Json<Value> {
    let caps = state.capabilities.read().await;
    Json(json!({ "capabilities": *caps }))
}

// ── Install ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct InstallRequest {
    pub path: String,
}

pub async fn install_capability(
    State(state): State<AppState>,
    Json(req): Json<InstallRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    // S8: Rate limiting
    if !state.install_rate_limiter.check("install") {
        return Err(AppError::BadRequest(
            "rate limit exceeded — try again later".to_string(),
        ));
    }

    let cap_dir = std::path::Path::new(&req.path);
    if !cap_dir.is_dir() {
        return Err(AppError::BadRequest(format!(
            "capability directory not found: {}",
            req.path
        )));
    }

    // 1. Read manifest.json from the capability directory
    let manifest_path = cap_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Err(AppError::BadRequest(format!(
            "manifest.json not found in {}",
            req.path
        )));
    }

    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| AppError::Internal(format!("Failed to read manifest.json: {}", e)))?;
    let manifest: CapabilityManifest = serde_json::from_str(&manifest_text)
        .map_err(|e| AppError::BadRequest(format!("Invalid manifest.json: {}", e)))?;

    info!(
        "Installing capability '{}' v{} from {}",
        manifest.name, manifest.version, req.path
    );

    // 2. Resolve binary path — check manifest location first, then common cargo output dirs
    let binary_name = std::path::Path::new(&manifest.binary)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| manifest.binary.clone());
    let binary_candidates = [
        cap_dir.join(&manifest.binary),
        cap_dir.join("target/release").join(&binary_name),
        cap_dir.join("target/debug").join(&binary_name),
    ];
    let binary_path = binary_candidates
        .iter()
        .find(|p| p.exists())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "binary not found — checked: {}",
                binary_candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?
        .clone();
    let binary_path_str = binary_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve binary path: {}", e)))?
        .to_string_lossy()
        .to_string();

    // Derive route_name early so we can check for collisions before doing anything else.
    // Source: manifest api.base_path (e.g. "/cap/feed" → "feed"),
    // falling back to last dot-segment of the capability name (e.g. "social.feed" → "feed").
    let route_name: Option<String> = manifest
        .api
        .as_ref()
        .and_then(|a| a.base_path.as_deref())
        .and_then(|bp| bp.trim_matches('/').rsplit('/').next())
        .or_else(|| manifest.name.rsplit('.').next())
        .map(|s| s.to_string());

    // 3. Check for duplicate name AND duplicate route_name
    {
        let caps = state.capabilities.read().await;
        if caps.iter().any(|c| c.name == manifest.name) {
            return Err(AppError::BadRequest(format!(
                "capability '{}' already installed",
                manifest.name
            )));
        }
        if let Some(ref rn) = route_name {
            if let Some(existing) = caps.iter().find(|c| c.route_name.as_deref() == Some(rn)) {
                return Err(AppError::BadRequest(format!(
                    "route name '{}' conflicts with installed capability '{}' — \
                     set a unique api.base_path in manifest.json",
                    rn, existing.name
                )));
            }
        }
    }

    // 4. Assign a host port
    let host_port = {
        let caps = state.capabilities.read().await;
        let default_port = manifest.port.unwrap_or(7001);
        capabilities::next_available_port(&caps, default_port)
    };

    // 5. Prepare data directory
    let data_dir = state.config.data_dir.join("cap-data").join(&manifest.name);
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| AppError::Internal(format!("Failed to create data dir: {}", e)))?;
    let data_dir_str = data_dir.to_string_lossy().to_string();

    // 6. Start the process
    let pid = executor::start_capability(
        &binary_path_str,
        &manifest.name,
        host_port,
        &data_dir_str,
        HashMap::new(),
    )
    .await
    .map_err(|e| AppError::Internal(format!("Failed to start capability: {}", e)))?;

    let visibility = manifest
        .permissions
        .as_ref()
        .and_then(|p| p.visibility.clone())
        .unwrap_or_else(|| "private".to_string());

    let manifest_path_str = manifest_path
        .canonicalize()
        .unwrap_or(manifest_path)
        .to_string_lossy()
        .to_string();

    // route_name already derived above (before duplicate check).

    // Derive P2P-CD fully-qualified name: howm.{manifest.name}.{protocol_version}
    //
    // The manifest `version` field is the *software* version (e.g. "0.1.0").
    // The P2P-CD protocol version is declared in p2pcd-peer.toml separately and
    // is always "1" for all current capabilities — the protocol is stable even
    // when the software is pre-1.0. Using the software major ("0") would produce
    // "howm.social.messaging.0" which never matches the negotiated "howm.social.messaging.1"
    // in active_sets, breaking capability notifications and online detection.
    //
    // Rule: protocol version = max(software_major, 1) — pre-1.0 software still speaks v1.
    let protocol_version = manifest
        .version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok())
        .map(|m| m.max(1))
        .unwrap_or(1);
    let p2pcd_name = Some(format!("howm.{}.{}", manifest.name, protocol_version));

    let entry = CapabilityEntry {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        port: host_port,
        pid: Some(pid),
        binary_path: binary_path_str,
        manifest_path: manifest_path_str,
        data_dir: data_dir_str,
        status: CapStatus::Running,
        visibility,
        ui: manifest.ui.clone(),
        route_name,
        p2pcd_name,
    };

    // 7. Persist
    {
        let mut caps = state.capabilities.write().await;
        caps.push(entry.clone());
        capabilities::save(&state.config.data_dir, &caps)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // 8. Register with the capability notifier under the p2pcd name so
    //    peer-active / peer-inactive callbacks reach this newly installed cap.
    if let Some(ref p2pcd_name) = entry.p2pcd_name {
        if let Some(ref engine) = state.p2pcd_engine {
            engine
                .register_capability(p2pcd_name.clone(), host_port)
                .await;
        }
    }

    info!(
        "Installed capability '{}' on port {} (pid={})",
        manifest.name, host_port, pid
    );
    Ok((StatusCode::OK, Json(json!({ "capability": entry }))))
}

// ── Stop ─────────────────────────────────────────────────────────────────────

pub async fn stop_capability(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let pid = {
        let caps = state.capabilities.read().await;
        let cap = caps
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| AppError::NotFound(format!("capability '{}' not found", name)))?;
        cap.pid
    };

    // Mark Stopped BEFORE sending SIGTERM so the PID health check loop cannot
    // race-restart the capability in the window between kill() and process exit.
    {
        let mut caps = state.capabilities.write().await;
        if let Some(cap) = caps.iter_mut().find(|c| c.name == name) {
            cap.status = CapStatus::Stopped;
            cap.pid = None;
        }
        capabilities::save(&state.config.data_dir, &caps)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    if let Some(pid) = pid {
        executor::stop_capability(pid)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    info!("Stopped capability '{}'", name);
    Ok(Json(json!({ "status": "stopped", "name": name })))
}

// ── Start ────────────────────────────────────────────────────────────────────

pub async fn start_capability(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let (binary_path, port, data_dir, p2pcd_name) = {
        let caps = state.capabilities.read().await;
        let cap = caps
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| AppError::NotFound(format!("capability '{}' not found", name)))?;
        (
            cap.binary_path.clone(),
            cap.port,
            cap.data_dir.clone(),
            cap.p2pcd_name.clone(),
        )
    };

    let pid = executor::start_capability(&binary_path, &name, port, &data_dir, HashMap::new())
        .await
        .map_err(|e| AppError::Internal(format!("Failed to start capability: {}", e)))?;

    {
        let mut caps = state.capabilities.write().await;
        if let Some(cap) = caps.iter_mut().find(|c| c.name == name) {
            cap.status = CapStatus::Running;
            cap.pid = Some(pid);
        }
        capabilities::save(&state.config.data_dir, &caps)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // Re-register with the notifier under the p2pcd name after (re)start.
    if let Some(ref p2pcd_name) = p2pcd_name {
        if let Some(ref engine) = state.p2pcd_engine {
            engine.register_capability(p2pcd_name.clone(), port).await;
        }
    }

    info!("Started capability '{}' (pid={})", name, pid);
    Ok(Json(
        json!({ "status": "started", "name": name, "pid": pid }),
    ))
}

// ── Uninstall ────────────────────────────────────────────────────────────────

pub async fn uninstall_capability(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let (pid, p2pcd_name) = {
        let caps = state.capabilities.read().await;
        let cap = caps
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| AppError::NotFound(format!("capability '{}' not found", name)))?;
        (cap.pid, cap.p2pcd_name.clone())
    };

    // Mark Stopped BEFORE sending SIGTERM so the PID health check loop cannot
    // race-restart the capability in the window between kill() and process exit.
    // Without this, the health check sees the pid go dead and spawns a new process
    // on the same port; the subsequent install attempt then hits EADDRINUSE.
    {
        let mut caps = state.capabilities.write().await;
        if let Some(cap) = caps.iter_mut().find(|c| c.name == name) {
            cap.status = CapStatus::Stopped;
            cap.pid = None;
        }
        // Don't persist yet — uninstall will remove the entry below.
    }

    // Best-effort stop
    if let Some(pid) = pid {
        if let Err(e) = executor::stop_capability(pid).await {
            warn!("Stop before uninstall failed (ignoring): {}", e);
        }
    }

    // Unregister from notifier so no further peer-active/inactive calls are sent
    // to a now-dead process.
    if let Some(ref p2pcd_name) = p2pcd_name {
        if let Some(ref engine) = state.p2pcd_engine {
            engine.unregister_capability(p2pcd_name).await;
        }
    }

    {
        let mut caps = state.capabilities.write().await;
        caps.retain(|c| c.name != name);
        capabilities::save(&state.config.data_dir, &caps)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    info!("Uninstalled capability '{}'", name);
    Ok(Json(json!({ "status": "uninstalled", "name": name })))
}

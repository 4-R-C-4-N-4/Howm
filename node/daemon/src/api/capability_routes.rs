use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

use crate::{
    capabilities::{self, CapStatus, CapabilityEntry},
    docker,
    error::AppError,
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
    pub image: String,
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

    info!("Installing capability from image: {}", req.image);

    // S9: Image allowlist check
    let allowed_file = state.config.data_dir.join("allowed_images.json");
    if allowed_file.exists() {
        let text = std::fs::read_to_string(&allowed_file)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let allowed: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
        if !allowed.is_empty()
            && !allowed.iter().any(|pattern| {
                if pattern.contains('*') {
                    // Simple glob: "registry/*" matches "registry/anything"
                    let prefix = pattern.trim_end_matches('*');
                    req.image.starts_with(prefix)
                } else {
                    req.image == *pattern
                }
            })
        {
            return Err(AppError::Forbidden(format!(
                "image '{}' not in allowed list",
                req.image
            )));
        }
    }

    // 1. Pull the image
    docker::pull_image(&req.image)
        .await
        .map_err(|e| AppError::DockerError(format!("Failed to pull image: {}", e)))?;

    // 2. Assign a host port
    let host_port = {
        let caps = state.capabilities.read().await;
        capabilities::next_available_port(&caps, 7001)
    };

    // 3. Prepare data volume directory
    let data_volume = state
        .config
        .data_dir
        .join("cap-data")
        .join(req.image.replace(['/', ':'], "-"));
    std::fs::create_dir_all(&data_volume)
        .map_err(|e| AppError::Internal(format!("Failed to create data volume dir: {}", e)))?;

    // 4. Start a temporary container to read the manifest first
    let temp_container_id =
        docker::start_capability(&req.image, host_port, data_volume.clone(), 7001, None)
            .await
            .map_err(|e| AppError::DockerError(format!("Failed to start container: {}", e)))?;

    // 5. Give the process a moment to initialise before reading the manifest
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 6. Read capability.yaml from inside the container
    let manifest = match docker::read_manifest(&temp_container_id).await {
        Ok(m) => m,
        Err(e) => {
            warn!(
                "Failed to read manifest from container {}: {}. Rolling back.",
                temp_container_id, e
            );
            let _ = docker::stop_capability(&temp_container_id).await;
            let _ = docker::remove_container(&temp_container_id).await;
            return Err(AppError::DockerError(format!(
                "Failed to read capability manifest: {}",
                e
            )));
        }
    };

    // S12: Read port from manifest (default 7001)
    let container_port = manifest.port.unwrap_or(7001);

    // If manifest specifies a different port or resources, restart with proper config
    let container_id = if container_port != 7001 || manifest.resources.is_some() {
        // Stop temp container and restart with correct settings
        let _ = docker::stop_capability(&temp_container_id).await;
        let _ = docker::remove_container(&temp_container_id).await;

        docker::start_capability(
            &req.image,
            host_port,
            data_volume,
            container_port,
            manifest.resources.as_ref(),
        )
        .await
        .map_err(|e| AppError::DockerError(format!("Failed to restart container: {}", e)))?
    } else {
        temp_container_id
    };

    let visibility = manifest
        .permissions
        .as_ref()
        .and_then(|p| p.visibility.clone())
        .unwrap_or_else(|| "private".to_string());

    let entry = CapabilityEntry {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        port: host_port,
        container_id: container_id.clone(),
        image: req.image.clone(),
        status: CapStatus::Running,
        visibility,
    };

    // 7. Persist
    {
        let mut caps = state.capabilities.write().await;
        caps.push(entry.clone());
        capabilities::save(&state.config.data_dir, &caps)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    info!(
        "Installed capability '{}' on port {}",
        manifest.name, host_port
    );
    Ok((StatusCode::OK, Json(json!({ "capability": entry }))))
}

// ── Stop ─────────────────────────────────────────────────────────────────────

pub async fn stop_capability(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let container_id = {
        let caps = state.capabilities.read().await;
        caps.iter()
            .find(|c| c.name == name)
            .map(|c| c.container_id.clone())
            .ok_or_else(|| AppError::NotFound(format!("capability '{}' not found", name)))?
    };

    docker::stop_capability(&container_id)
        .await
        .map_err(|e| AppError::DockerError(e.to_string()))?;

    {
        let mut caps = state.capabilities.write().await;
        if let Some(cap) = caps.iter_mut().find(|c| c.name == name) {
            cap.status = CapStatus::Stopped;
        }
        capabilities::save(&state.config.data_dir, &caps)
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
    let container_id = {
        let caps = state.capabilities.read().await;
        caps.iter()
            .find(|c| c.name == name)
            .map(|c| c.container_id.clone())
            .ok_or_else(|| AppError::NotFound(format!("capability '{}' not found", name)))?
    };

    let docker = docker::connect().map_err(|e| AppError::DockerError(e.to_string()))?;
    docker
        .start_container(
            &container_id,
            None::<bollard::container::StartContainerOptions<String>>,
        )
        .await
        .map_err(|e| AppError::DockerError(e.to_string()))?;

    {
        let mut caps = state.capabilities.write().await;
        if let Some(cap) = caps.iter_mut().find(|c| c.name == name) {
            cap.status = CapStatus::Running;
        }
        capabilities::save(&state.config.data_dir, &caps)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    info!("Started capability '{}'", name);
    Ok(Json(json!({ "status": "started", "name": name })))
}

// ── Uninstall ────────────────────────────────────────────────────────────────

pub async fn uninstall_capability(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let container_id = {
        let caps = state.capabilities.read().await;
        caps.iter()
            .find(|c| c.name == name)
            .map(|c| c.container_id.clone())
            .ok_or_else(|| AppError::NotFound(format!("capability '{}' not found", name)))?
    };

    // Best-effort stop then remove
    if let Err(e) = docker::stop_capability(&container_id).await {
        warn!("Stop before uninstall failed (ignoring): {}", e);
    }
    if let Err(e) = docker::remove_container(&container_id).await {
        warn!("Remove container failed (ignoring): {}", e);
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

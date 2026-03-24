use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{error::AppError, state::AppState};
use howm_access::{CapabilityRule, PermissionResult};

// ── List all groups ──────────────────────────────────────────────────────────

pub async fn list_groups(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let groups = state
        .access_db
        .list_groups()
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;
    Ok(Json(json!(groups)))
}

// ── Create a custom group ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<CapabilityRuleInput>,
}

#[derive(Deserialize)]
pub struct CapabilityRuleInput {
    pub capability_name: String,
    #[serde(default = "default_true")]
    pub allow: bool,
    pub rate_limit: Option<u64>,
    pub ttl: Option<u64>,
}

fn default_true() -> bool {
    true
}

impl From<&CapabilityRuleInput> for CapabilityRule {
    fn from(input: &CapabilityRuleInput) -> Self {
        CapabilityRule {
            capability_name: input.capability_name.clone(),
            allow: input.allow,
            rate_limit: input.rate_limit,
            ttl: input.ttl,
        }
    }
}

pub async fn create_group(
    State(state): State<AppState>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<Json<Value>, AppError> {
    if req.name.is_empty() || req.name.len() > 64 {
        return Err(AppError::BadRequest(
            "group name must be 1-64 characters".to_string(),
        ));
    }

    let rules: Vec<CapabilityRule> = req.capabilities.iter().map(Into::into).collect();

    let group = state
        .access_db
        .create_group(&req.name, req.description.as_deref(), &rules)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;

    Ok(Json(json!(group)))
}

// ── Get group detail ─────────────────────────────────────────────────────────

pub async fn get_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let uuid = parse_uuid(&group_id)?;
    let group = state
        .access_db
        .get_group(&uuid)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?
        .ok_or_else(|| AppError::NotFound(format!("group {} not found", group_id)))?;
    Ok(Json(json!(group)))
}

// ── Update group ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub capabilities: Option<Vec<CapabilityRuleInput>>,
}

pub async fn update_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
    Json(req): Json<UpdateGroupRequest>,
) -> Result<Json<Value>, AppError> {
    let uuid = parse_uuid(&group_id)?;

    if let Some(ref name) = req.name {
        if name.is_empty() || name.len() > 64 {
            return Err(AppError::BadRequest(
                "group name must be 1-64 characters".to_string(),
            ));
        }
    }

    let rules: Option<Vec<CapabilityRule>> = req
        .capabilities
        .as_ref()
        .map(|caps| caps.iter().map(Into::into).collect());

    let group = state
        .access_db
        .update_group(
            &uuid,
            req.name.as_deref(),
            req.description.as_ref().map(|d| d.as_deref()),
            rules.as_deref(),
        )
        .map_err(|e| {
            // Built-in group rule modification returns QueryReturnedNoRows
            AppError::BadRequest(format!(
                "cannot modify built-in group capability rules: {}",
                e
            ))
        })?
        .ok_or_else(|| AppError::NotFound(format!("group {} not found", group_id)))?;

    Ok(Json(json!(group)))
}

// ── Delete group ─────────────────────────────────────────────────────────────

pub async fn delete_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let uuid = parse_uuid(&group_id)?;

    state
        .access_db
        .delete_group(&uuid)
        .map_err(|e| AppError::BadRequest(format!("cannot delete built-in group: {}", e)))?;

    Ok(Json(json!({ "status": "deleted", "group_id": group_id })))
}

// ── List group members (peer IDs) ─────────────────────────────────────────────

pub async fn list_group_members(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let uuid = parse_uuid(&group_id)?;

    // Verify group exists
    state
        .access_db
        .get_group(&uuid)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?
        .ok_or_else(|| AppError::NotFound(format!("group {} not found", group_id)))?;

    let member_ids = state
        .access_db
        .list_group_member_ids(&uuid)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;

    let hex_ids: Vec<String> = member_ids.iter().map(hex::encode).collect();

    Ok(Json(json!({
        "group_id": group_id,
        "members": hex_ids,
    })))
}

// ── List peer groups ─────────────────────────────────────────────────────────

pub async fn list_peer_groups(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let peer_bytes = parse_peer_id(&peer_id)?;
    let groups = state
        .access_db
        .list_peer_groups(&peer_bytes)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;
    Ok(Json(json!(groups)))
}

// ── Assign peer to group ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AssignPeerRequest {
    pub group_id: String,
}

pub async fn assign_peer_to_group(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
    Json(req): Json<AssignPeerRequest>,
) -> Result<Json<Value>, AppError> {
    let peer_bytes = parse_peer_id(&peer_id)?;
    let group_uuid = parse_uuid(&req.group_id)?;

    // Verify group exists
    state
        .access_db
        .get_group(&group_uuid)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?
        .ok_or_else(|| AppError::NotFound(format!("group {} not found", req.group_id)))?;

    let membership = state
        .access_db
        .assign_peer_to_group(&peer_bytes, &group_uuid)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;

    // Phase 5: Trigger rebroadcast so the peer's capabilities are re-evaluated
    if let Some(engine) = &state.p2pcd_engine {
        if let Ok(pid) = try_peer_id_array(&peer_bytes) {
            engine.on_membership_changed(&pid).await;
        }
    }

    Ok(Json(json!({
        "status": "assigned",
        "peer_id": peer_id,
        "group_id": req.group_id,
        "assigned_at": membership.assigned_at,
    })))
}

// ── Remove peer from group ───────────────────────────────────────────────────

pub async fn remove_peer_from_group(
    State(state): State<AppState>,
    Path((peer_id, group_id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let peer_bytes = parse_peer_id(&peer_id)?;
    let group_uuid = parse_uuid(&group_id)?;

    let removed = state
        .access_db
        .remove_peer_from_group(&peer_bytes, &group_uuid)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;

    if removed {
        // Phase 5: Trigger rebroadcast so the peer's capabilities are re-evaluated
        if let Some(engine) = &state.p2pcd_engine {
            if let Ok(pid) = try_peer_id_array(&peer_bytes) {
                engine.on_membership_changed(&pid).await;
            }
        }

        Ok(Json(
            json!({ "status": "removed", "peer_id": peer_id, "group_id": group_id }),
        ))
    } else {
        Err(AppError::NotFound(format!(
            "peer {} is not in group {}",
            peer_id, group_id
        )))
    }
}

// ── Get effective permissions ────────────────────────────────────────────────

pub async fn get_effective_permissions(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let peer_bytes = parse_peer_id(&peer_id)?;
    let perms = state
        .access_db
        .get_peer_effective_permissions(&peer_bytes)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;

    let result: serde_json::Map<String, Value> = perms
        .into_iter()
        .map(|(cap, perm)| {
            let val = match perm {
                PermissionResult::Allow { rate_limit, ttl } => json!({
                    "allowed": true,
                    "rate_limit": rate_limit,
                    "ttl": ttl,
                }),
                PermissionResult::Deny => json!({ "allowed": false }),
            };
            (cap, val)
        })
        .collect();

    Ok(Json(json!({
        "peer_id": peer_id,
        "permissions": result,
    })))
}

// ── Deny peer — revoke access, close session, cache as Denied ────────────────

pub async fn deny_peer(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let peer_bytes = parse_peer_id(&peer_id)?;

    // 1. Remove from all groups
    let removed = state
        .access_db
        .remove_peer_from_all_groups(&peer_bytes)
        .map_err(|e| AppError::Internal(format!("access_db: {}", e)))?;

    // 2. Close active P2P-CD session with AuthFailure + cache as Denied
    let mut session_closed = false;
    if let Some(engine) = &state.p2pcd_engine {
        if let Ok(pid) = try_peer_id_array(&peer_bytes) {
            engine.deny_session(&pid).await;
            session_closed = true;
        }
    }

    Ok(Json(json!({
        "status": "denied",
        "peer_id": peer_id,
        "groups_removed": removed,
        "session_closed": session_closed,
    })))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn parse_uuid(s: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| AppError::BadRequest(format!("invalid UUID: {}", s)))
}

fn parse_peer_id(hex_str: &str) -> Result<Vec<u8>, AppError> {
    let bytes = hex::decode(hex_str)
        .map_err(|_| AppError::BadRequest("invalid hex peer_id".to_string()))?;
    if bytes.len() != 32 {
        return Err(AppError::BadRequest(
            "peer_id must be 32 bytes (64 hex chars)".to_string(),
        ));
    }
    Ok(bytes)
}

/// Convert Vec<u8> to fixed-size [u8; 32] PeerId for engine calls.
fn try_peer_id_array(bytes: &[u8]) -> Result<p2pcd_types::PeerId, AppError> {
    if bytes.len() != 32 {
        return Err(AppError::Internal("peer_id not 32 bytes".to_string()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    Ok(arr)
}

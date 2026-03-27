//! Profile API routes.
//!
//! - `GET  /profile`          — get own profile (local+wg)
//! - `PUT  /profile`          — update name/bio (authenticated)
//! - `PUT  /profile/avatar`   — upload avatar image (authenticated)
//! - `PUT  /profile/homepage` — set homepage path (authenticated)
//! - `GET  /profile/avatar`   — serve avatar image (public — peers fetch this)
//! - `GET  /profile/home`     — serve homepage HTML (public)
//! - `GET  /profile/home/*`   — serve homepage assets (public)
//! - `GET  /peer/{id}/profile` — fetch a peer's profile (wg)

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use crate::{error::AppError, profile, state::AppState};

// ── GET /profile ──────────────────────────────────────────────────────────────

/// Return the current node's profile.
pub async fn get_profile(State(state): State<AppState>) -> Json<Value> {
    let p = state.profile.read().await;
    Json(json!({
        "name": p.name,
        "bio": p.bio,
        "avatar": p.avatar,
        "homepage": p.homepage,
        "has_avatar": p.avatar.is_some(),
        "has_homepage": p.homepage.is_some(),
    }))
}

// ── PUT /profile ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateProfile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
}

/// Update profile name and/or bio.
pub async fn update_profile(
    State(state): State<AppState>,
    Json(req): Json<UpdateProfile>,
) -> Result<Json<Value>, AppError> {
    let mut p = state.profile.write().await;

    if let Some(name) = req.name {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::BadRequest("Name cannot be empty".to_string()));
        }
        if name.len() > 64 {
            return Err(AppError::BadRequest(
                "Name too long (max 64 chars)".to_string(),
            ));
        }
        p.name = name;
    }

    if let Some(bio) = req.bio {
        if bio.len() > 280 {
            return Err(AppError::BadRequest(
                "Bio too long (max 280 chars)".to_string(),
            ));
        }
        p.bio = bio;
    }

    profile::save(&state.config.data_dir, &p)
        .map_err(|e| AppError::Internal(format!("Failed to save profile: {}", e)))?;

    info!("Profile updated: {}", p.name);

    Ok(Json(json!({
        "name": p.name,
        "bio": p.bio,
    })))
}

// ── PUT /profile/avatar ───────────────────────────────────────────────────────

/// Upload avatar image (multipart or raw body with Content-Type).
pub async fn upload_avatar(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppError> {
    if body.is_empty() {
        return Err(AppError::BadRequest("Empty body".to_string()));
    }

    if body.len() > profile::MAX_AVATAR_SIZE {
        return Err(AppError::BadRequest(format!(
            "Avatar too large: {} bytes (max {})",
            body.len(),
            profile::MAX_AVATAR_SIZE
        )));
    }

    // Determine extension from Content-Type header
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png");

    let ext = match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unsupported content type '{}' — use image/png, image/jpeg, or image/webp",
                content_type
            )))
        }
    };

    let filename = format!("avatar.{}", ext);
    let stored = profile::save_avatar(&state.config.data_dir, &filename, &body)
        .map_err(|e| AppError::Internal(format!("Failed to save avatar: {}", e)))?;

    // Update profile
    let mut p = state.profile.write().await;
    p.avatar = Some(stored.clone());
    profile::save(&state.config.data_dir, &p)
        .map_err(|e| AppError::Internal(format!("Failed to save profile: {}", e)))?;

    info!("Avatar uploaded: {}", stored);

    Ok(Json(json!({
        "avatar": stored,
    })))
}

// ── PUT /profile/homepage ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SetHomepage {
    /// Relative path within the profile directory, e.g. "homepage/index.html".
    /// Set to null or empty to remove the homepage.
    pub path: Option<String>,
}

/// Set the homepage HTML path.
pub async fn set_homepage(
    State(state): State<AppState>,
    Json(req): Json<SetHomepage>,
) -> Result<Json<Value>, AppError> {
    let mut p = state.profile.write().await;

    match req.path {
        Some(path) if !path.trim().is_empty() => {
            let path = path.trim().to_string();

            // Verify the file exists
            let resolved = profile::ProfilePaths::new(&state.config.data_dir)
                .dir
                .join(&path);
            if !resolved.exists() {
                return Err(AppError::NotFound(format!(
                    "Homepage file not found: {} (place it in {})",
                    path,
                    profile::ProfilePaths::new(&state.config.data_dir)
                        .dir
                        .display()
                )));
            }

            p.homepage = Some(path.clone());
            info!("Homepage set: {}", path);
        }
        _ => {
            p.homepage = None;
            info!("Homepage removed");
        }
    }

    profile::save(&state.config.data_dir, &p)
        .map_err(|e| AppError::Internal(format!("Failed to save profile: {}", e)))?;

    Ok(Json(json!({
        "homepage": p.homepage,
    })))
}

// ── GET /profile/avatar ───────────────────────────────────────────────────────

/// Serve the avatar image. Public endpoint — peers fetch this.
pub async fn serve_avatar(State(state): State<AppState>) -> Response {
    let p = state.profile.read().await;
    match profile::read_avatar(&state.config.data_dir, &p) {
        Some((data, content_type)) => {
            ([(header::CONTENT_TYPE, content_type)], Body::from(data)).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── GET /profile/home ─────────────────────────────────────────────────────────

/// Serve the homepage HTML. Public endpoint with strict CSP sandbox.
pub async fn serve_homepage(State(state): State<AppState>) -> Response {
    let p = state.profile.read().await;
    match profile::resolve_homepage(&state.config.data_dir, &p) {
        Some(path) => match std::fs::read(&path) {
            Ok(data) => (
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    // Strict CSP: sandbox the page, block access to howm API
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; \
                         frame-ancestors 'self'; \
                         connect-src 'none'",
                    ),
                ],
                Body::from(data),
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── GET /profile/home/*rest ───────────────────────────────────────────────────

/// Serve homepage assets (CSS, images, JS, fonts, etc).
pub async fn serve_homepage_asset(
    State(state): State<AppState>,
    Path(rest): Path<String>,
) -> Response {
    let p = state.profile.read().await;
    match profile::resolve_homepage_asset(&state.config.data_dir, &p, &rest) {
        Some(path) => match std::fs::read(&path) {
            Ok(data) => {
                let content_type = mime_for_path(&path);
                ([(header::CONTENT_TYPE, content_type)], Body::from(data)).into_response()
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── GET /peer/{id}/profile ────────────────────────────────────────────────────

/// Fetch a peer's profile by proxying to their daemon.
pub async fn get_peer_profile(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // Find the peer
    let peers = state.peers.read().await;
    let peer = peers
        .iter()
        .find(|p| p.node_id == node_id)
        .ok_or_else(|| AppError::NotFound(format!("Peer not found: {}", node_id)))?;

    let url = format!("http://{}:{}/profile", peer.wg_address, peer.port);
    let wg_address = peer.wg_address.clone();
    drop(peers);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let resp = client.get(&url).send().await.map_err(|e| {
        AppError::PeerUnreachable(format!(
            "Cannot reach peer {} at {} — {}",
            node_id, wg_address, e
        ))
    })?;

    if !resp.status().is_success() {
        return Err(AppError::PeerUnreachable(format!(
            "Peer {} returned status {}",
            node_id,
            resp.status()
        )));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Invalid profile response: {}", e)))?;

    Ok(Json(body))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("ico") => "image/x-icon",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}

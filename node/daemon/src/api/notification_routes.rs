use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::{
    error::AppError,
    notifications::{BadgeUpdate, BadgesResponse, PollResponse, PushRequest},
    state::AppState,
};

// ── POST /notifications/badge ────────────────────────────────────────────────

pub async fn set_badge(
    State(state): State<AppState>,
    Json(req): Json<BadgeUpdate>,
) -> Result<impl IntoResponse, AppError> {
    // Validate capability name against installed capabilities
    {
        let caps = state.capabilities.read().await;
        if !caps.iter().any(|c| c.name == req.capability) {
            return Err(AppError::NotFound(format!(
                "unknown capability: {}",
                req.capability
            )));
        }
    }

    let mut badges = state.badges.write().await;
    if req.count == 0 {
        badges.remove(&req.capability);
    } else {
        badges.insert(req.capability, req.count);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── GET /notifications/badges ────────────────────────────────────────────────

pub async fn get_badges(State(state): State<AppState>) -> Json<BadgesResponse> {
    let badges = state.badges.read().await;
    Json(BadgesResponse {
        badges: badges.clone(),
    })
}

// ── POST /notifications/push ─────────────────────────────────────────────────

pub async fn push_notification(
    State(state): State<AppState>,
    Json(req): Json<PushRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate capability name
    {
        let caps = state.capabilities.read().await;
        if !caps.iter().any(|c| c.name == req.capability) {
            return Err(AppError::NotFound(format!(
                "unknown capability: {}",
                req.capability
            )));
        }
    }

    // Rate limit: 10 pushes per capability per 10s
    {
        let mut limiter = state.push_rate_limiter.write().await;
        if !limiter.check_and_record(&req.capability) {
            return Err(AppError::TooManyRequests(
                "notification push rate limit exceeded".to_string(),
            ));
        }
    }

    let mut buf = state.notifications.write().await;
    buf.push(req);

    Ok(StatusCode::NO_CONTENT)
}

// ── GET /notifications/poll ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PollParams {
    #[serde(default)]
    pub since: Option<u64>,
}

pub async fn poll_notifications(
    State(state): State<AppState>,
    Query(params): Query<PollParams>,
) -> Json<PollResponse> {
    let since = params.since.unwrap_or(0);

    let mut buf = state.notifications.write().await;
    let notifications = buf.poll(since);

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Json(PollResponse {
        notifications,
        timestamp,
    })
}

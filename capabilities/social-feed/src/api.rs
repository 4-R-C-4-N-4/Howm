use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::info;

use crate::posts;

#[derive(Clone)]
pub struct FeedState {
    pub data_dir: PathBuf,
}

pub async fn get_feed(State(state): State<FeedState>) -> Json<Value> {
    let mut posts = posts::load(&state.data_dir).unwrap_or_default();
    // Newest first
    posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Json(json!({ "posts": posts }))
}

#[derive(Deserialize)]
pub struct CreatePostRequest {
    pub content: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
}

pub async fn create_post(
    State(state): State<FeedState>,
    headers: HeaderMap,
    Json(req): Json<CreatePostRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    // Author comes from the request body; fall back to injected node identity headers.
    let author_id = req
        .author_id
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("X-Node-Id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "anonymous".to_string());

    let author_name = req
        .author_name
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("X-Node-Name")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Anonymous".to_string());

    match posts::create(&state.data_dir, req.content, author_id, author_name) {
        Ok(post) => {
            info!("Created post: {}", post.id);
            Ok((StatusCode::CREATED, Json(json!({ "post": post }))))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )),
    }
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    BadRequest(String),
    PeerUnreachable(String),
    DockerError(String),
    Internal(String),
    Forbidden(String),
    Gone(String),
    Conflict(String),
    TooManyRequests(String),
    InsufficientStorage(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::PeerUnreachable(m) => (StatusCode::BAD_GATEWAY, m),
            AppError::DockerError(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            AppError::Gone(m) => (StatusCode::GONE, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m),
            AppError::InsufficientStorage(m) => (StatusCode::INSUFFICIENT_STORAGE, m),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

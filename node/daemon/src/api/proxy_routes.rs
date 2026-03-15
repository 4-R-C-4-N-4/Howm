use axum::{
    body::Body,
    extract::{Path, State},
    http::Request,
    response::Response,
};

use crate::{error::AppError, proxy, state::AppState};

pub async fn proxy_handler(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, AppError> {
    // In axum 0.7 the `*rest` wildcard captures the path WITHOUT a leading slash,
    // but we pass it straight through — proxy_request strips any leading slash itself.
    proxy::proxy_request(&state, &name, &rest, req).await
}

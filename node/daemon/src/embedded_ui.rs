use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

static UI_DIST: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/web/dist");

/// Axum fallback handler — serves embedded UI assets with SPA index.html fallback.
pub async fn serve_embedded(req: Request<Body>) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    serve_file(path)
}

fn serve_file(path: &str) -> Response {
    match UI_DIST.get_file(path) {
        Some(file) => (
            [(header::CONTENT_TYPE, mime_for(path))],
            Body::from(file.contents()),
        )
            .into_response(),
        None => {
            // SPA fallback: unknown paths return index.html so client-side routing works
            match UI_DIST.get_file("index.html") {
                Some(index) => (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    Body::from(index.contents()),
                )
                    .into_response(),
                None => (StatusCode::NOT_FOUND, "UI not built — run cd ui/web && npm run build")
                    .into_response(),
            }
        }
    }
}

fn mime_for(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else {
        "application/octet-stream"
    }
}

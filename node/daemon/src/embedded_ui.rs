use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};
use std::net::SocketAddr;

use crate::{api::is_local_or_wg, state::AppState};

static UI_DIST: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/web/dist");

/// Axum fallback handler — serves embedded UI assets with SPA index.html fallback.
/// Only injects the API token for requests from localhost or WireGuard subnet.
pub async fn serve_embedded(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<Body>,
) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let inject_token = is_local_or_wg(&addr.ip());

    // Serve the exact file if it exists
    if let Some(file) = UI_DIST.get_file(path) {
        if path == "index.html" {
            return serve_index(file.contents(), inject_token.then_some(&state.api_token));
        }
        return (
            [(header::CONTENT_TYPE, mime_for(path))],
            Body::from(file.contents()),
        )
            .into_response();
    }

    // SPA fallback: unknown paths return index.html so client-side routing works
    match UI_DIST.get_file("index.html") {
        Some(index) => serve_index(index.contents(), inject_token.then_some(&state.api_token)),
        None => (
            StatusCode::NOT_FOUND,
            "UI not built — run cd ui/web && npm run build",
        )
            .into_response(),
    }
}

/// Serve index.html, optionally injecting the API token as a <meta> tag.
fn serve_index(html_bytes: &[u8], token: Option<&String>) -> Response {
    let html = String::from_utf8_lossy(html_bytes);
    let injected = match token {
        Some(t) => {
            // HTML-escape the token value to prevent injection if the token
            // ever contains special characters (currently hex/base64, but
            // defense-in-depth).
            let escaped = t
                .replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let meta = format!(r#"<meta name="howm-token" content="{}">"#, escaped);
            html.replacen("</head>", &format!("  {}\n  </head>", meta), 1)
        }
        None => html.into_owned(),
    };
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Body::from(injected),
    )
        .into_response()
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

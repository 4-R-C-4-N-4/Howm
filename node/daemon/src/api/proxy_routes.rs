use axum::{
    body::Body,
    extract::{ConnectInfo, Path, State},
    http::Request,
    response::Response,
};
use std::net::SocketAddr;

use crate::{error::AppError, proxy, state::AppState};

pub async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, rest)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, AppError> {
    // S7: Capability visibility enforcement
    let source_ip = addr.ip().to_string();
    let is_local = source_ip == "127.0.0.1" || source_ip == "::1";

    {
        let caps = state.capabilities.read().await;
        if let Some(cap) = caps.iter().find(|c| {
            let first_seg = c.name.split('.').next().unwrap_or(&c.name);
            first_seg == name || c.name == name
        }) {
            match cap.visibility.as_str() {
                "private" => {
                    if !is_local {
                        return Err(AppError::Forbidden(
                            "capability is private — local access only".to_string(),
                        ));
                    }
                }
                "friends" => {
                    if !is_local {
                        // Check if source IP is a known peer's WG address
                        let peers = state.peers.read().await;
                        let is_known_peer = peers.iter().any(|p| p.wg_address == source_ip);
                        if !is_known_peer {
                            return Err(AppError::Forbidden(
                                "capability is friends-only".to_string(),
                            ));
                        }
                    }
                }
                _ => {
                    // "public" or unknown — allow all (anyone on the WG network)
                }
            }
        }
    }

    proxy::proxy_request(&state, &name, &rest, req).await
}

//! `/network/*` API routes for the Connection page.
//!
//! Aggregates WireGuard status, NAT profile, IPv6 detection, relay config,
//! and reachability into a unified endpoint the UI can poll.

use axum::{extract::State, Json};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{net_detect, state::AppState, stun};

// ── Types ────────────────────────────────────────────────────────────────────

/// Reachability tier computed from NAT + IPv6 state.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reachability {
    /// Has public IPv4/IPv6 or open NAT — one-way invites work for everyone.
    Direct,
    /// Cone NAT — two-way exchange needed for NAT peers.
    Punchable,
    /// Symmetric NAT — needs relay for NAT peers.
    RelayOnly,
    /// Detection not run yet.
    Unknown,
}

// ── GET /network/status ──────────────────────────────────────────────────────

/// Aggregated network status for the Connection page.
/// Combines WG status, NAT profile, IPv6 detection, relay config, and
/// computed reachability into a single response.
pub async fn network_status(State(state): State<AppState>) -> Json<Value> {
    // WireGuard status
    let wg_active = *state.wg_active.read().await;
    let wg = json!({
        "status": if wg_active { "connected" } else { "disconnected" },
        "public_key": state.identity.wg_pubkey,
        "address": state.identity.wg_address,
        "endpoint": state.identity.wg_endpoint,
        "listen_port": state.identity.wg_listen_port.unwrap_or(state.config.wg_port),
        "active_tunnels": state.peers.read().await.len(),
    });

    // NAT profile (cached on disk)
    let nat_profile = stun::load_nat_profile(&state.config.data_dir);
    let nat = match &nat_profile {
        Some(p) => json!({
            "detected": true,
            "nat_type": p.nat_type,
            "external_ipv4": p.external_ip,
            "external_port": p.external_port,
            "observed_stride": p.observed_stride,
            "detected_at": p.detected_at,
        }),
        None => json!({
            "detected": false,
            "nat_type": "unknown",
            "external_ipv4": null,
            "external_port": null,
            "observed_stride": 0,
            "detected_at": null,
        }),
    };

    // IPv6 GUA detection
    let guas = net_detect::detect_ipv6_guas();
    let ipv6 = json!({
        "available": !guas.is_empty(),
        "global_addresses": guas.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
        "preferred": !guas.is_empty(),
    });

    // Reachability computation
    let reachability = compute_reachability(&nat_profile, &guas, &state.identity.wg_endpoint);

    // Relay config
    let allow_relay = *state.allow_relay.read().await;
    let relay_capable_peers = count_relay_capable_peers(&state).await;
    let active_matchmakes = *state.matchmake_counter.read().await;
    let relay = json!({
        "allow_relay": allow_relay,
        "relay_capable_peers": relay_capable_peers,
        "active_matchmakes": active_matchmakes,
    });

    let peer_count = state.peers.read().await.len();

    Json(json!({
        "wireguard": wg,
        "nat": nat,
        "ipv6": ipv6,
        "reachability": reachability,
        "relay": relay,
        "peer_count": peer_count,
    }))
}

// ── POST /network/detect ─────────────────────────────────────────────────────

/// Run NAT detection (STUN test battery). Proxies to existing stun module.
pub async fn network_detect(
    State(state): State<AppState>,
) -> Result<Json<Value>, crate::error::AppError> {
    let wg_port = state
        .identity
        .wg_listen_port
        .unwrap_or(state.config.wg_port);

    let data_dir = state.config.data_dir.clone();
    let profile = tokio::task::spawn_blocking(move || stun::refresh_mapping(&data_dir, wg_port))
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("NAT detection task failed: {e}")))?;

    Ok(Json(json!({
        "detected": true,
        "nat_type": profile.nat_type,
        "external_ipv4": profile.external_ip,
        "external_port": profile.external_port,
        "observed_stride": profile.observed_stride,
        "detected_at": profile.detected_at,
    })))
}

// ── GET /network/nat-profile ─────────────────────────────────────────────────

/// Return cached NAT profile without re-running detection.
pub async fn network_nat_profile(State(state): State<AppState>) -> Json<Value> {
    match stun::load_nat_profile(&state.config.data_dir) {
        Some(p) => Json(json!({
            "detected": true,
            "nat_type": p.nat_type,
            "external_ipv4": p.external_ip,
            "external_port": p.external_port,
            "observed_stride": p.observed_stride,
            "detected_at": p.detected_at,
        })),
        None => Json(json!({
            "detected": false,
            "nat_type": "unknown",
            "external_ipv4": null,
            "external_port": null,
            "observed_stride": 0,
            "detected_at": null,
        })),
    }
}

// ── PUT /network/relay ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RelayUpdate {
    pub allow_relay: bool,
}

/// Update the runtime relay toggle.
pub async fn network_relay_update(
    State(state): State<AppState>,
    Json(req): Json<RelayUpdate>,
) -> Json<Value> {
    *state.allow_relay.write().await = req.allow_relay;

    tracing::info!(
        "Relay signaling {}",
        if req.allow_relay {
            "enabled"
        } else {
            "disabled"
        }
    );

    let relay_capable_peers = count_relay_capable_peers(&state).await;
    Json(json!({
        "allow_relay": req.allow_relay,
        "relay_capable_peers": relay_capable_peers,
    }))
}

// ── GET /network/pending ─────────────────────────────────────────────────────

/// List pending two-way invite exchanges.
///
/// For now returns an empty list — the pending exchange tracker will be
/// added when the two-way invite flow is wired into the daemon's invite
/// creation path. The endpoint exists so the UI can poll without errors.
pub async fn network_pending(State(_state): State<AppState>) -> Json<Value> {
    // TODO: Wire into invite creation to track pending Tier 2 exchanges.
    // The daemon knows when it generates an invite with NAT info attached
    // (i.e., reachability != direct). Those become pending exchanges that
    // resolve when the corresponding accept token is redeemed.
    Json(json!({
        "pending": []
    }))
}

// ── GET /network/matchmake/status ───────────────────────────────────────────

/// Current matchmake relay status.
pub async fn matchmake_status(State(state): State<AppState>) -> Json<Value> {
    let active = *state.matchmake_counter.read().await;
    let relay_capable_peers = count_relay_capable_peers(&state).await;
    let allow_relay = *state.allow_relay.read().await;

    Json(json!({
        "active_matchmakes": active,
        "relay_capable_peers": relay_capable_peers,
        "allow_relay": allow_relay,
    }))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn compute_reachability(
    nat: &Option<stun::NatProfile>,
    guas: &[std::net::Ipv6Addr],
    wg_endpoint: &Option<String>,
) -> Reachability {
    // If we have IPv6 GUAs, we're directly reachable
    if !guas.is_empty() {
        return Reachability::Direct;
    }

    match nat {
        Some(profile) => match profile.nat_type {
            stun::NatType::Open => Reachability::Direct,
            stun::NatType::Cone => {
                // If we have a configured endpoint, we might be directly reachable
                // (user port-forwarded). Otherwise, punchable.
                if wg_endpoint.is_some() {
                    Reachability::Direct
                } else {
                    Reachability::Punchable
                }
            }
            stun::NatType::Symmetric => Reachability::RelayOnly,
            stun::NatType::Unknown => {
                // Detection ran but failed — treat as unknown
                Reachability::Unknown
            }
        },
        None => {
            // No detection run. If they have a manual endpoint, assume direct.
            if wg_endpoint.is_some() {
                Reachability::Direct
            } else {
                Reachability::Unknown
            }
        }
    }
}

/// Count how many connected peers have relay capability active.
/// For now this checks if the p2pcd engine has peers with core.network.relay.1
/// in their active capability set.
async fn count_relay_capable_peers(state: &AppState) -> usize {
    match &state.p2pcd_engine {
        Some(engine) => {
            let sessions = engine.active_sessions().await;
            sessions
                .iter()
                .filter(|s| s.active_set.iter().any(|c| c == "core.network.relay.1"))
                .count()
        }
        None => 0,
    }
}

/// Collect base64-encoded WG public keys of connected peers that have
/// relay capability active. Used to populate relay_candidates in invite tokens.
pub async fn collect_relay_candidate_pubkeys(state: &AppState) -> Vec<String> {
    match &state.p2pcd_engine {
        Some(engine) => {
            let sessions = engine.active_sessions().await;
            sessions
                .iter()
                .filter(|s| s.active_set.iter().any(|c| c == "core.network.relay.1"))
                .map(|s| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.peer_id))
                .collect()
        }
        None => vec![],
    }
}

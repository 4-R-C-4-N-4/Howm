//! Matchmake relay — Phase 4 NAT traversal via peer signaling.
//!
//! When hole punching fails (Tier 2 timeout) or both peers are behind symmetric
//! NAT, a mutual friend on the mesh can relay endpoint information so both peers
//! can attempt a direct WireGuard connection. This is STUN-over-mesh.
//!
//! The matchmake protocol uses p2pcd relay circuits (CIRCUIT_OPEN/DATA/CLOSE)
//! as transport. Three CBOR messages ride on a short-lived circuit:
//!
//!   1. MatchmakeRequest  — initiator's endpoint info (sent to target via relay)
//!   2. MatchmakeExchange — target's endpoint info (sent back via relay)
//!   3. Both sides configure WG and attempt direct punch
//!
//! The relay peer (Carol) never parses or understands matchmake messages.
//! She's a dumb pipe for < 1 second.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use ciborium::value::Value;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::punch::{self, PunchConfig};
use crate::state::AppState;
use crate::stun::{self, NatType};

// ── CBOR keys ────────────────────────────────────────────────────────────────

mod cbor_keys {
    pub const MSG_TYPE: u64 = 1;
    pub const WG_PUBKEY: u64 = 2;
    pub const EXTERNAL_IP: u64 = 3;
    pub const EXTERNAL_PORT: u64 = 4;
    pub const WG_PORT: u64 = 5;
    pub const NAT_TYPE: u64 = 6;
    pub const STRIDE: u64 = 7;
    pub const IPV6_GUAS: u64 = 8;
    pub const PSK: u64 = 9;
    pub const ASSIGNED_IP: u64 = 10;
    pub const WG_ADDRESS: u64 = 11;
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Endpoint info gathered for matchmaking.
#[derive(Debug, Clone)]
pub struct EndpointInfo {
    pub wg_pubkey: String,
    pub external_ip: String,
    pub external_port: u16,
    pub wg_port: u16,
    pub nat_type: NatType,
    pub observed_stride: i32,
    pub ipv6_guas: Vec<String>,
    pub wg_address: String,
}

/// A decoded matchmake request (initiator's info arriving at the target).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchmakeRequest {
    pub wg_pubkey: String,
    pub external_ip: String,
    pub external_port: u16,
    pub wg_port: u16,
    pub nat_type: NatType,
    pub observed_stride: i32,
    pub ipv6_guas: Vec<String>,
    pub psk: String,
    pub assigned_ip: String,
    pub wg_address: String,
}

/// A decoded matchmake exchange (target's response to the initiator).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchmakeExchangeMsg {
    pub wg_pubkey: String,
    pub external_ip: String,
    pub external_port: u16,
    pub wg_port: u16,
    pub nat_type: NatType,
    pub observed_stride: i32,
    pub ipv6_guas: Vec<String>,
    pub wg_address: String,
}

/// Outcome of a matchmake attempt.
#[derive(Debug)]
pub enum MatchmakeResult {
    /// WG handshake succeeded after relay-assisted exchange.
    Connected,
    /// Endpoint info exchanged but punch still failed.
    PunchFailed,
}

/// Errors during matchmaking.
#[derive(Debug)]
pub enum MatchmakeError {
    NoMutualRelay,
    CircuitFailed(String),
    ExchangeTimeout,
    PunchError(String),
    InvalidMessage(String),
}

impl std::fmt::Display for MatchmakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMutualRelay => write!(f, "no mutual relay peer found"),
            Self::CircuitFailed(s) => write!(f, "relay circuit failed: {s}"),
            Self::ExchangeTimeout => write!(f, "matchmake exchange timed out"),
            Self::PunchError(s) => write!(f, "punch failed after exchange: {s}"),
            Self::InvalidMessage(s) => write!(f, "invalid matchmake message: {s}"),
        }
    }
}

impl std::error::Error for MatchmakeError {}

/// Wrapper enum for dispatching incoming circuit data.
#[derive(Debug)]
pub enum MatchmakeMessage {
    Request(MatchmakeRequest),
    Exchange(MatchmakeExchangeMsg),
}

// ── CBOR Encode/Decode ───────────────────────────────────────────────────────

// Inline CBOR helpers (same patterns as p2pcd::cbor_helpers, but we can't
// import those from the daemon crate).

fn cbor_encode(pairs: Vec<(u64, Value)>) -> Vec<u8> {
    let map: Vec<(Value, Value)> = pairs
        .into_iter()
        .map(|(k, v)| (Value::Integer(ciborium::value::Integer::from(k)), v))
        .collect();
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Value::Map(map), &mut out).expect("CBOR encode");
    out
}

fn cbor_decode(data: &[u8]) -> Result<Vec<(Value, Value)>> {
    let val: Value =
        ciborium::de::from_reader(data).map_err(|e| anyhow::anyhow!("CBOR decode: {e}"))?;
    match val {
        Value::Map(m) => Ok(m),
        _ => anyhow::bail!("expected CBOR map"),
    }
}

fn get_text(map: &[(Value, Value)], key: u64) -> Option<String> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Text(s) = v {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

fn get_int(map: &[(Value, Value)], key: u64) -> Option<u64> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Integer(vi) = v {
                    return u64::try_from(*vi).ok();
                }
            }
        }
    }
    None
}

fn get_signed(map: &[(Value, Value)], key: u64) -> Option<i32> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Integer(vi) = v {
                    return i128::from(*vi).try_into().ok();
                }
            }
        }
    }
    None
}

fn get_text_array(map: &[(Value, Value)], key: u64) -> Vec<String> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Array(arr) = v {
                    return arr
                        .iter()
                        .filter_map(|v| {
                            if let Value::Text(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                }
            }
        }
    }
    vec![]
}

fn parse_nat_type(s: &str) -> NatType {
    match s {
        "open" => NatType::Open,
        "cone" => NatType::Cone,
        "symmetric" => NatType::Symmetric,
        _ => NatType::Unknown,
    }
}

/// Encode a MatchmakeRequest into CBOR bytes for CIRCUIT_DATA payload.
pub fn encode_request(info: &EndpointInfo, psk: &str, assigned_ip: &str) -> Vec<u8> {
    let guas: Vec<Value> = info
        .ipv6_guas
        .iter()
        .map(|s| Value::Text(s.clone()))
        .collect();

    cbor_encode(vec![
        (cbor_keys::MSG_TYPE, Value::Text("matchmake-request".into())),
        (cbor_keys::WG_PUBKEY, Value::Text(info.wg_pubkey.clone())),
        (
            cbor_keys::EXTERNAL_IP,
            Value::Text(info.external_ip.clone()),
        ),
        (
            cbor_keys::EXTERNAL_PORT,
            Value::Integer(info.external_port.into()),
        ),
        (cbor_keys::WG_PORT, Value::Integer(info.wg_port.into())),
        (cbor_keys::NAT_TYPE, Value::Text(info.nat_type.to_string())),
        (
            cbor_keys::STRIDE,
            Value::Integer(ciborium::value::Integer::from(info.observed_stride as i64)),
        ),
        (cbor_keys::IPV6_GUAS, Value::Array(guas)),
        (cbor_keys::PSK, Value::Text(psk.to_string())),
        (cbor_keys::ASSIGNED_IP, Value::Text(assigned_ip.to_string())),
        (cbor_keys::WG_ADDRESS, Value::Text(info.wg_address.clone())),
    ])
}

/// Encode a MatchmakeExchange into CBOR bytes for CIRCUIT_DATA payload.
pub fn encode_exchange(info: &EndpointInfo) -> Vec<u8> {
    let guas: Vec<Value> = info
        .ipv6_guas
        .iter()
        .map(|s| Value::Text(s.clone()))
        .collect();

    cbor_encode(vec![
        (
            cbor_keys::MSG_TYPE,
            Value::Text("matchmake-exchange".into()),
        ),
        (cbor_keys::WG_PUBKEY, Value::Text(info.wg_pubkey.clone())),
        (
            cbor_keys::EXTERNAL_IP,
            Value::Text(info.external_ip.clone()),
        ),
        (
            cbor_keys::EXTERNAL_PORT,
            Value::Integer(info.external_port.into()),
        ),
        (cbor_keys::WG_PORT, Value::Integer(info.wg_port.into())),
        (cbor_keys::NAT_TYPE, Value::Text(info.nat_type.to_string())),
        (
            cbor_keys::STRIDE,
            Value::Integer(ciborium::value::Integer::from(info.observed_stride as i64)),
        ),
        (cbor_keys::IPV6_GUAS, Value::Array(guas)),
        (cbor_keys::WG_ADDRESS, Value::Text(info.wg_address.clone())),
    ])
}

/// Decode a matchmake message from CBOR bytes.
pub fn decode_message(data: &[u8]) -> Result<MatchmakeMessage, MatchmakeError> {
    let map = cbor_decode(data).map_err(|e| MatchmakeError::InvalidMessage(e.to_string()))?;

    let msg_type = get_text(&map, cbor_keys::MSG_TYPE)
        .ok_or_else(|| MatchmakeError::InvalidMessage("missing msg_type".into()))?;

    match msg_type.as_str() {
        "matchmake-request" => {
            let req = MatchmakeRequest {
                wg_pubkey: get_text(&map, cbor_keys::WG_PUBKEY).unwrap_or_default(),
                external_ip: get_text(&map, cbor_keys::EXTERNAL_IP).unwrap_or_default(),
                external_port: get_int(&map, cbor_keys::EXTERNAL_PORT).unwrap_or(0) as u16,
                wg_port: get_int(&map, cbor_keys::WG_PORT).unwrap_or(0) as u16,
                nat_type: get_text(&map, cbor_keys::NAT_TYPE)
                    .map(|s| parse_nat_type(&s))
                    .unwrap_or(NatType::Unknown),
                observed_stride: get_signed(&map, cbor_keys::STRIDE).unwrap_or(0),
                ipv6_guas: get_text_array(&map, cbor_keys::IPV6_GUAS),
                psk: get_text(&map, cbor_keys::PSK).unwrap_or_default(),
                assigned_ip: get_text(&map, cbor_keys::ASSIGNED_IP).unwrap_or_default(),
                wg_address: get_text(&map, cbor_keys::WG_ADDRESS).unwrap_or_default(),
            };
            Ok(MatchmakeMessage::Request(req))
        }
        "matchmake-exchange" => {
            let exch = MatchmakeExchangeMsg {
                wg_pubkey: get_text(&map, cbor_keys::WG_PUBKEY).unwrap_or_default(),
                external_ip: get_text(&map, cbor_keys::EXTERNAL_IP).unwrap_or_default(),
                external_port: get_int(&map, cbor_keys::EXTERNAL_PORT).unwrap_or(0) as u16,
                wg_port: get_int(&map, cbor_keys::WG_PORT).unwrap_or(0) as u16,
                nat_type: get_text(&map, cbor_keys::NAT_TYPE)
                    .map(|s| parse_nat_type(&s))
                    .unwrap_or(NatType::Unknown),
                observed_stride: get_signed(&map, cbor_keys::STRIDE).unwrap_or(0),
                ipv6_guas: get_text_array(&map, cbor_keys::IPV6_GUAS),
                wg_address: get_text(&map, cbor_keys::WG_ADDRESS).unwrap_or_default(),
            };
            Ok(MatchmakeMessage::Exchange(exch))
        }
        other => Err(MatchmakeError::InvalidMessage(format!(
            "unknown msg_type: {}",
            other
        ))),
    }
}

// ── Relay Discovery ──────────────────────────────────────────────────────────

/// Find a mutual relay peer from the invite's relay candidates.
///
/// `their_candidates` are base64-encoded WG pubkeys from the invite token.
/// `our_peers` are the WG pubkeys of our currently connected peers.
pub fn find_mutual_relay(
    their_candidates: &[String],
    our_peers: &HashSet<String>,
) -> Result<String, MatchmakeError> {
    for candidate in their_candidates {
        if our_peers.contains(candidate) {
            info!(
                "matchmake: found mutual relay peer {}…",
                &candidate[..candidate.len().min(8)]
            );
            return Ok(candidate.clone());
        }
    }
    warn!("matchmake: no mutual relay peer found");
    Err(MatchmakeError::NoMutualRelay)
}

// ── Orchestration ───────────────────────────────────────────────────────────

/// Gather our endpoint info from STUN profile + identity for matchmake exchange.
///
/// Refreshes the STUN mapping if stale (> 60s) or missing, then collects
/// our WG pubkey, external IP/port, NAT type, stride, IPv6 GUAs, and WG address.
pub async fn gather_endpoint_info(state: &AppState) -> Result<EndpointInfo, MatchmakeError> {
    let wg_pubkey = state
        .identity
        .wg_pubkey
        .clone()
        .ok_or_else(|| MatchmakeError::PunchError("no WG pubkey on identity".into()))?;

    let wg_port = state.identity.wg_listen_port.unwrap_or(41641);
    let wg_address = state.identity.wg_address.clone().unwrap_or_default();

    let data_dir = state.config.data_dir.clone();

    // Load existing profile or refresh
    let profile = match stun::load_nat_profile(&data_dir) {
        Some(p) => {
            // If older than 60s, refresh in background but use cached for now
            let age = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .saturating_sub(p.detected_at);
            if age > 60 {
                debug!("matchmake: NAT profile stale ({}s), refreshing", age);
                let dd = data_dir.clone();
                let port = wg_port;
                tokio::task::spawn_blocking(move || stun::refresh_mapping(&dd, port))
                    .await
                    .map_err(|e| MatchmakeError::PunchError(format!("STUN refresh failed: {e}")))?
            } else {
                p
            }
        }
        None => {
            let dd = data_dir.clone();
            let port = wg_port;
            tokio::task::spawn_blocking(move || stun::refresh_mapping(&dd, port))
                .await
                .map_err(|e| MatchmakeError::PunchError(format!("STUN refresh failed: {e}")))?
        }
    };

    let ipv6_guas = crate::net_detect::detect_ipv6_guas()
        .iter()
        .map(|a| a.to_string())
        .collect();

    Ok(EndpointInfo {
        wg_pubkey,
        external_ip: profile.external_ip.clone(),
        external_port: profile.external_port,
        wg_port,
        nat_type: profile.nat_type,
        observed_stride: profile.observed_stride,
        ipv6_guas,
        wg_address,
    })
}

/// Build a PunchConfig from a received matchmake exchange message.
///
/// Called by the initiator after receiving the target's MatchmakeExchangeMsg,
/// and by the target after receiving the initiator's MatchmakeRequest.
#[allow(clippy::too_many_arguments)]
pub fn build_punch_config_from_exchange(
    peer_pubkey: &str,
    peer_external_ip: &str,
    peer_external_port: u16,
    peer_stride: i32,
    peer_wg_port: u16,
    peer_nat_type: NatType,
    our_nat_type: NatType,
    psk: Option<String>,
    allowed_ip: &str,
) -> PunchConfig {
    let we_initiate = punch::should_we_initiate(our_nat_type, peer_nat_type);
    PunchConfig {
        peer_pubkey: peer_pubkey.to_string(),
        peer_external_ip: peer_external_ip.to_string(),
        peer_external_port,
        peer_stride,
        peer_wg_port,
        peer_nat_type,
        our_nat_type,
        psk,
        allowed_ip: allowed_ip.to_string(),
        we_initiate,
    }
}

/// Build PunchConfig from a MatchmakeExchangeMsg (initiator side).
pub fn punch_config_from_exchange_msg(
    exch: &MatchmakeExchangeMsg,
    our_nat_type: NatType,
    psk: Option<String>,
    allowed_ip: &str,
) -> PunchConfig {
    build_punch_config_from_exchange(
        &exch.wg_pubkey,
        &exch.external_ip,
        exch.external_port,
        exch.observed_stride,
        exch.wg_port,
        exch.nat_type,
        our_nat_type,
        psk,
        allowed_ip,
    )
}

/// Build PunchConfig from a MatchmakeRequest (target side).
pub fn punch_config_from_request(req: &MatchmakeRequest, our_nat_type: NatType) -> PunchConfig {
    build_punch_config_from_exchange(
        &req.wg_pubkey,
        &req.external_ip,
        req.external_port,
        req.observed_stride,
        req.wg_port,
        req.nat_type,
        our_nat_type,
        Some(req.psk.clone()),
        &req.assigned_ip,
    )
}

/// Initiate a matchmake exchange via a relay circuit.
///
/// Steps:
///   1. Gather our endpoint info
///   2. Open a relay circuit through `relay_pubkey` to `target_pubkey`
///   3. Send MatchmakeRequest with our info + PSK + assigned_ip
///   4. Wait for MatchmakeExchange response
///   5. Build PunchConfig and run punch
///
/// Returns MatchmakeResult indicating whether the WG handshake succeeded.
pub async fn initiate_matchmake(
    state: &AppState,
    relay_pubkey: &str,
    target_pubkey: &str,
    psk: &str,
    assigned_ip: &str,
    counter: Arc<RwLock<u64>>,
) -> Result<MatchmakeResult, MatchmakeError> {
    info!(
        "matchmake: initiating via relay {}… to target {}…",
        &relay_pubkey[..relay_pubkey.len().min(8)],
        &target_pubkey[..target_pubkey.len().min(8)],
    );

    // Increment active count
    {
        let mut c = counter.write().await;
        *c += 1;
    }

    let result = do_initiate_matchmake(state, relay_pubkey, target_pubkey, psk, assigned_ip).await;

    // Decrement active count
    {
        let mut c = counter.write().await;
        *c = c.saturating_sub(1);
    }

    result
}

async fn do_initiate_matchmake(
    state: &AppState,
    relay_pubkey: &str,
    target_pubkey: &str,
    psk: &str,
    assigned_ip: &str,
) -> Result<MatchmakeResult, MatchmakeError> {
    let engine = state
        .p2pcd_engine
        .as_ref()
        .ok_or_else(|| MatchmakeError::CircuitFailed("p2pcd engine not running".into()))?;

    // Gather our endpoint info
    let our_info = gather_endpoint_info(state).await?;
    let our_nat = our_info.nat_type;

    // Get relay handler
    let handler = engine
        .cap_router()
        .handler_by_name("core.network.relay.1")
        .ok_or_else(|| MatchmakeError::CircuitFailed("relay handler not found".into()))?;
    let relay_handler = handler
        .as_any()
        .downcast_ref::<::p2pcd::capabilities::relay::RelayHandler>()
        .ok_or_else(|| MatchmakeError::CircuitFailed("relay handler type mismatch".into()))?;

    // Parse peer IDs
    let relay_peer = p2pcd_types::config::parse_wg_pubkey(relay_pubkey)
        .ok_or_else(|| MatchmakeError::CircuitFailed("bad relay pubkey".to_string()))?;
    let target_peer = p2pcd_types::config::parse_wg_pubkey(target_pubkey)
        .ok_or_else(|| MatchmakeError::CircuitFailed("bad target pubkey".to_string()))?;

    // Set up event channel
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    relay_handler.set_event_callback(tx).await;

    // Open circuit
    let circuit_id = relay_handler
        .initiate_circuit(&relay_peer, &target_peer)
        .await;

    // Wait for circuit to open
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(ev) = rx.recv().await {
            if let ::p2pcd::capabilities::relay::CircuitEvent::Opened {
                circuit_id: cid, ..
            } = ev
            {
                if cid == circuit_id {
                    return Ok(());
                }
            }
        }
        Err(MatchmakeError::CircuitFailed("channel closed".into()))
    })
    .await
    .map_err(|_| MatchmakeError::ExchangeTimeout)?
    .map_err(|e: MatchmakeError| e)?;

    // Send our request
    let request_bytes = encode_request(&our_info, psk, assigned_ip);
    relay_handler
        .send_circuit_data(circuit_id, request_bytes)
        .await
        .map_err(|e| MatchmakeError::CircuitFailed(format!("send_circuit_data: {e}")))?;

    debug!("matchmake: request sent on circuit {}", circuit_id);

    // Wait for exchange response
    let exchange = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(ev) = rx.recv().await {
            match ev {
                ::p2pcd::capabilities::relay::CircuitEvent::Data {
                    circuit_id: cid,
                    data,
                    ..
                } if cid == circuit_id => match decode_message(&data)? {
                    MatchmakeMessage::Exchange(exch) => return Ok(exch),
                    _ => {
                        return Err(MatchmakeError::InvalidMessage(
                            "expected exchange, got request".into(),
                        ))
                    }
                },
                ::p2pcd::capabilities::relay::CircuitEvent::Closed {
                    circuit_id: cid,
                    reason,
                    ..
                } if cid == circuit_id => {
                    return Err(MatchmakeError::CircuitFailed(format!(
                        "circuit closed: reason code {reason}"
                    )));
                }
                _ => continue,
            }
        }
        Err(MatchmakeError::CircuitFailed("channel closed".into()))
    })
    .await
    .map_err(|_| MatchmakeError::ExchangeTimeout)?
    .map_err(|e: MatchmakeError| e)?;

    // Close circuit — we have what we need
    let _ = relay_handler.close_endpoint_circuit(circuit_id).await;

    info!(
        "matchmake: exchange received from {}…, running punch",
        &exchange.wg_pubkey[..exchange.wg_pubkey.len().min(8)]
    );

    // Build punch config and run
    let config =
        punch_config_from_exchange_msg(&exchange, our_nat, Some(psk.to_string()), assigned_ip);
    let punch_result = punch::run_punch_system(
        &config,
        &state.config.data_dir,
        "howm0",
        std::time::Duration::from_secs(15),
    )
    .await;

    match punch_result {
        punch::PunchResult::Success { elapsed, .. } => {
            info!(
                "matchmake: punch succeeded in {:.1}s",
                elapsed.as_secs_f64()
            );
            Ok(MatchmakeResult::Connected)
        }
        punch::PunchResult::Timeout { .. } | punch::PunchResult::Error(_) => {
            warn!("matchmake: punch failed after exchange");
            Ok(MatchmakeResult::PunchFailed)
        }
    }
}

/// Handle an incoming matchmake request arriving on a relay circuit.
///
/// Called by the circuit event handler when we receive a MatchmakeRequest.
/// We gather our own endpoint info, send it back as a MatchmakeExchange,
/// then run the punch from our side.
pub async fn handle_incoming_matchmake(
    state: &AppState,
    circuit_id: u64,
    request: MatchmakeRequest,
    counter: Arc<RwLock<u64>>,
) -> Result<MatchmakeResult, MatchmakeError> {
    info!(
        "matchmake: incoming request from {}… on circuit {}",
        &request.wg_pubkey[..request.wg_pubkey.len().min(8)],
        circuit_id,
    );

    // Increment active count
    {
        let mut c = counter.write().await;
        *c += 1;
    }

    let result = do_handle_incoming(state, circuit_id, &request).await;

    // Decrement active count
    {
        let mut c = counter.write().await;
        *c = c.saturating_sub(1);
    }

    result
}

async fn do_handle_incoming(
    state: &AppState,
    circuit_id: u64,
    request: &MatchmakeRequest,
) -> Result<MatchmakeResult, MatchmakeError> {
    let engine = state
        .p2pcd_engine
        .as_ref()
        .ok_or_else(|| MatchmakeError::CircuitFailed("p2pcd engine not running".into()))?;

    // Gather our endpoint info
    let our_info = gather_endpoint_info(state).await?;
    let our_nat = our_info.nat_type;

    // Get relay handler to send response
    let handler = engine
        .cap_router()
        .handler_by_name("core.network.relay.1")
        .ok_or_else(|| MatchmakeError::CircuitFailed("relay handler not found".into()))?;
    let relay_handler = handler
        .as_any()
        .downcast_ref::<::p2pcd::capabilities::relay::RelayHandler>()
        .ok_or_else(|| MatchmakeError::CircuitFailed("relay handler type mismatch".into()))?;

    // Send our exchange response
    let exchange_bytes = encode_exchange(&our_info);
    relay_handler
        .send_circuit_data(circuit_id, exchange_bytes)
        .await
        .map_err(|e| MatchmakeError::CircuitFailed(format!("send exchange: {e}")))?;

    debug!("matchmake: exchange sent on circuit {}", circuit_id);

    // Close our end of the circuit
    let _ = relay_handler.close_endpoint_circuit(circuit_id).await;

    // Build punch config from request and run punch
    let config = punch_config_from_request(request, our_nat);
    let punch_result = punch::run_punch_system(
        &config,
        &state.config.data_dir,
        "howm0",
        std::time::Duration::from_secs(15),
    )
    .await;

    match punch_result {
        punch::PunchResult::Success { elapsed, .. } => {
            info!(
                "matchmake: incoming punch succeeded in {:.1}s",
                elapsed.as_secs_f64()
            );
            Ok(MatchmakeResult::Connected)
        }
        punch::PunchResult::Timeout { .. } | punch::PunchResult::Error(_) => {
            warn!("matchmake: incoming punch failed after exchange");
            Ok(MatchmakeResult::PunchFailed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_endpoint_info() -> EndpointInfo {
        EndpointInfo {
            wg_pubkey: "dGVzdF9wdWJrZXk".to_string(),
            external_ip: "203.0.113.5".to_string(),
            external_port: 41641,
            wg_port: 41641,
            nat_type: NatType::Cone,
            observed_stride: 0,
            ipv6_guas: vec!["2001:db8::1".to_string()],
            wg_address: "100.222.0.1".to_string(),
        }
    }

    #[test]
    fn request_roundtrip() {
        let info = sample_endpoint_info();
        let encoded = encode_request(&info, "test_psk", "100.222.0.2");
        let decoded = decode_message(&encoded).unwrap();

        match decoded {
            MatchmakeMessage::Request(req) => {
                assert_eq!(req.wg_pubkey, info.wg_pubkey);
                assert_eq!(req.external_ip, "203.0.113.5");
                assert_eq!(req.external_port, 41641);
                assert_eq!(req.wg_port, 41641);
                assert_eq!(req.nat_type, NatType::Cone);
                assert_eq!(req.observed_stride, 0);
                assert_eq!(req.ipv6_guas, vec!["2001:db8::1"]);
                assert_eq!(req.psk, "test_psk");
                assert_eq!(req.assigned_ip, "100.222.0.2");
                assert_eq!(req.wg_address, "100.222.0.1");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn exchange_roundtrip() {
        let info = EndpointInfo {
            wg_pubkey: "Ym9iX3B1YmtleQ".to_string(),
            external_ip: "198.51.100.10".to_string(),
            external_port: 41642,
            wg_port: 41641,
            nat_type: NatType::Symmetric,
            observed_stride: 2,
            ipv6_guas: vec![],
            wg_address: "100.222.0.3".to_string(),
        };
        let encoded = encode_exchange(&info);
        let decoded = decode_message(&encoded).unwrap();

        match decoded {
            MatchmakeMessage::Exchange(exch) => {
                assert_eq!(exch.wg_pubkey, "Ym9iX3B1YmtleQ");
                assert_eq!(exch.external_ip, "198.51.100.10");
                assert_eq!(exch.external_port, 41642);
                assert_eq!(exch.wg_port, 41641);
                assert_eq!(exch.nat_type, NatType::Symmetric);
                assert_eq!(exch.observed_stride, 2);
                assert!(exch.ipv6_guas.is_empty());
                assert_eq!(exch.wg_address, "100.222.0.3");
            }
            _ => panic!("expected Exchange"),
        }
    }

    #[test]
    fn decode_unknown_msg_type() {
        let data = cbor_encode(vec![(
            cbor_keys::MSG_TYPE,
            Value::Text("matchmake-bogus".into()),
        )]);
        let err = decode_message(&data).unwrap_err();
        assert!(matches!(err, MatchmakeError::InvalidMessage(_)));
    }

    #[test]
    fn find_mutual_relay_with_overlap() {
        let their = vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()];
        let mut ours = HashSet::new();
        ours.insert("bbb".to_string());
        ours.insert("ddd".to_string());

        let result = find_mutual_relay(&their, &ours).unwrap();
        assert_eq!(result, "bbb");
    }

    #[test]
    fn find_mutual_relay_no_overlap() {
        let their = vec!["aaa".to_string()];
        let mut ours = HashSet::new();
        ours.insert("bbb".to_string());

        let err = find_mutual_relay(&their, &ours).unwrap_err();
        assert!(matches!(err, MatchmakeError::NoMutualRelay));
    }

    #[test]
    fn negative_stride_roundtrip() {
        let info = EndpointInfo {
            observed_stride: -3,
            ..sample_endpoint_info()
        };
        let encoded = encode_request(&info, "psk", "ip");
        let decoded = decode_message(&encoded).unwrap();
        match decoded {
            MatchmakeMessage::Request(req) => assert_eq!(req.observed_stride, -3),
            _ => panic!("expected Request"),
        }
    }

    // ── Orchestration tests ─────────────────────────────────────────────

    #[test]
    fn build_punch_config_from_exchange_basic() {
        let config = build_punch_config_from_exchange(
            "dGVzdF9wdWJrZXk",
            "203.0.113.5",
            41641,
            0,
            41641,
            NatType::Cone,
            NatType::Symmetric,
            Some("test_psk".to_string()),
            "100.222.0.2",
        );

        assert_eq!(config.peer_pubkey, "dGVzdF9wdWJrZXk");
        assert_eq!(config.peer_external_ip, "203.0.113.5");
        assert_eq!(config.peer_external_port, 41641);
        assert_eq!(config.peer_stride, 0);
        assert_eq!(config.peer_wg_port, 41641);
        assert_eq!(config.peer_nat_type, NatType::Cone);
        assert_eq!(config.our_nat_type, NatType::Symmetric);
        assert_eq!(config.psk, Some("test_psk".to_string()));
        assert_eq!(config.allowed_ip, "100.222.0.2");
        // Symmetric vs Cone: we should NOT initiate (cone initiates against symmetric)
        assert!(!config.we_initiate);
    }

    #[test]
    fn build_punch_config_cone_vs_symmetric_initiates() {
        let config = build_punch_config_from_exchange(
            "key",
            "1.2.3.4",
            5000,
            2,
            41641,
            NatType::Symmetric,
            NatType::Cone,
            None,
            "100.222.0.5",
        );

        // We are cone, they are symmetric — we should initiate
        assert!(config.we_initiate);
        assert_eq!(config.peer_stride, 2);
        assert!(config.psk.is_none());
    }

    #[test]
    fn punch_config_from_exchange_msg_helper() {
        let exch = MatchmakeExchangeMsg {
            wg_pubkey: "bob_key".to_string(),
            external_ip: "198.51.100.10".to_string(),
            external_port: 41642,
            wg_port: 41641,
            nat_type: NatType::Cone,
            observed_stride: 1,
            ipv6_guas: vec![],
            wg_address: "100.222.0.3".to_string(),
        };

        let config = punch_config_from_exchange_msg(
            &exch,
            NatType::Cone,
            Some("psk123".to_string()),
            "100.222.0.3",
        );

        assert_eq!(config.peer_pubkey, "bob_key");
        assert_eq!(config.peer_external_port, 41642);
        assert_eq!(config.psk, Some("psk123".to_string()));
    }

    #[test]
    fn punch_config_from_request_helper() {
        let req = MatchmakeRequest {
            wg_pubkey: "alice_key".to_string(),
            external_ip: "203.0.113.1".to_string(),
            external_port: 12345,
            wg_port: 41641,
            nat_type: NatType::Open,
            observed_stride: 0,
            ipv6_guas: vec!["2001:db8::1".to_string()],
            psk: "req_psk".to_string(),
            assigned_ip: "100.222.0.10".to_string(),
            wg_address: "100.222.0.1".to_string(),
        };

        let config = punch_config_from_request(&req, NatType::Cone);

        assert_eq!(config.peer_pubkey, "alice_key");
        assert_eq!(config.peer_external_ip, "203.0.113.1");
        assert_eq!(config.psk, Some("req_psk".to_string()));
        assert_eq!(config.allowed_ip, "100.222.0.10");
    }

    #[test]
    fn tier_ladder_flow_encode_decode_request_then_exchange() {
        // Simulate the full matchmake message flow:
        // 1. Initiator encodes request with their endpoint info
        // 2. Target decodes it, builds punch config
        // 3. Target encodes exchange with their endpoint info
        // 4. Initiator decodes it, builds punch config

        let initiator_info = EndpointInfo {
            wg_pubkey: "initiator_pubkey".to_string(),
            external_ip: "203.0.113.1".to_string(),
            external_port: 41641,
            wg_port: 41641,
            nat_type: NatType::Cone,
            observed_stride: 0,
            ipv6_guas: vec![],
            wg_address: "100.222.0.1".to_string(),
        };

        let target_info = EndpointInfo {
            wg_pubkey: "target_pubkey".to_string(),
            external_ip: "198.51.100.5".to_string(),
            external_port: 41642,
            wg_port: 41641,
            nat_type: NatType::Symmetric,
            observed_stride: 3,
            ipv6_guas: vec!["2001:db8::2".to_string()],
            wg_address: "100.222.0.2".to_string(),
        };

        // Step 1: Initiator sends request
        let req_bytes = encode_request(&initiator_info, "shared_psk", "100.222.0.2");
        let decoded_req = decode_message(&req_bytes).unwrap();
        let req = match decoded_req {
            MatchmakeMessage::Request(r) => r,
            _ => panic!("expected Request"),
        };

        // Step 2: Target builds punch config from request
        let target_punch = punch_config_from_request(&req, NatType::Symmetric);
        assert_eq!(target_punch.peer_pubkey, "initiator_pubkey");
        assert_eq!(target_punch.peer_nat_type, NatType::Cone);
        assert_eq!(target_punch.our_nat_type, NatType::Symmetric);

        // Step 3: Target sends exchange
        let exch_bytes = encode_exchange(&target_info);
        let decoded_exch = decode_message(&exch_bytes).unwrap();
        let exch = match decoded_exch {
            MatchmakeMessage::Exchange(e) => e,
            _ => panic!("expected Exchange"),
        };

        // Step 4: Initiator builds punch config from exchange
        let init_punch = punch_config_from_exchange_msg(
            &exch,
            NatType::Cone,
            Some("shared_psk".to_string()),
            "100.222.0.2",
        );
        assert_eq!(init_punch.peer_pubkey, "target_pubkey");
        assert_eq!(init_punch.peer_external_ip, "198.51.100.5");
        assert_eq!(init_punch.peer_stride, 3);
        assert_eq!(init_punch.peer_nat_type, NatType::Symmetric);
        assert_eq!(init_punch.our_nat_type, NatType::Cone);
        // Cone initiates against symmetric
        assert!(init_punch.we_initiate);
    }
}

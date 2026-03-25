use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use tracing::{info, warn};

use super::{bad_request, base64_decode, base64_encode, hex_to_hash, AppState};

// ── Inbound RPC messages ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InboundMessage {
    pub peer_id: String,
    pub message_type: u64,
    pub payload: String,
    #[serde(default)]
    pub capability: String,
}

// CBOR keys for catalogue RPC envelopes
pub(crate) const CBOR_KEY_METHOD: u64 = 1;
pub(crate) const CBOR_KEY_CURSOR: u64 = 2;
pub(crate) const CBOR_KEY_LIMIT: u64 = 3;
pub(crate) const CBOR_KEY_BLOB_IDS: u64 = 4;

// Response keys
pub(crate) const CBOR_KEY_OFFERINGS: u64 = 10;
pub(crate) const CBOR_KEY_NEXT_CURSOR: u64 = 11;
pub(crate) const CBOR_KEY_TOTAL: u64 = 12;
pub(crate) const CBOR_KEY_HAS: u64 = 13;

/// Handle inbound RPC messages from peers (forwarded by cap_notify).
pub async fn inbound_message(
    State(state): State<AppState>,
    Json(msg): Json<InboundMessage>,
) -> impl IntoResponse {
    info!(
        "inbound: type={} from {} (cap: {})",
        msg.message_type,
        &msg.peer_id[..8.min(msg.peer_id.len())],
        msg.capability
    );

    // Decode CBOR payload
    let payload_bytes = match base64_decode(&msg.payload) {
        Some(b) => b,
        None => {
            warn!(
                "Failed to decode base64 payload from {}",
                &msg.peer_id[..8.min(msg.peer_id.len())]
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid payload encoding" })),
            );
        }
    };

    // Parse method from CBOR
    let method = match decode_rpc_method(&payload_bytes) {
        Some(m) => m,
        None => {
            warn!("Failed to decode RPC method from payload");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing or invalid method" })),
            );
        }
    };

    match method.as_str() {
        "catalogue.list" => {
            let response = handle_catalogue_list(&state, &msg.peer_id, &payload_bytes).await;
            // Send response back via bridge RPC
            let response_b64 = base64_encode(&response);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "response": response_b64 })),
            )
        }
        "catalogue.has_blob" => {
            let response = handle_catalogue_has_blob(&state, &payload_bytes).await;
            let response_b64 = base64_encode(&response);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "response": response_b64 })),
            )
        }
        _ => {
            warn!("Unknown RPC method: {}", method);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown method: {}", method) })),
            )
        }
    }
}

/// Handle catalogue.list RPC — returns filtered, paginated catalogue.
async fn handle_catalogue_list(state: &AppState, peer_id_b64: &str, payload: &[u8]) -> Vec<u8> {
    // Parse cursor and limit from CBOR
    let (cursor, limit) = decode_catalogue_list_params(payload);
    let limit = limit.clamp(1, 100);

    // Get peer's cached group memberships
    let groups = {
        let active = state.active_peers.read().await;
        match active.get(peer_id_b64) {
            Some(peer) => peer.groups.clone(),
            None => vec![], // unknown peer gets no groups
        }
    };

    // Query filtered offerings
    let (offerings, total) =
        match state
            .db
            .list_offerings_for_peer_paginated(peer_id_b64, &groups, cursor, limit)
        {
            Ok(result) => result,
            Err(e) => {
                warn!("Failed to list offerings for peer: {}", e);
                (vec![], 0)
            }
        };

    // Compute next cursor
    let next_cursor = if cursor + offerings.len() < total {
        Some(cursor + offerings.len())
    } else {
        None
    };

    // Encode response as CBOR
    encode_catalogue_list_response(&offerings, next_cursor, total)
}

/// Handle catalogue.has_blob RPC — check which blobs we have locally.
async fn handle_catalogue_has_blob(state: &AppState, payload: &[u8]) -> Vec<u8> {
    let blob_ids = decode_has_blob_params(payload);

    if blob_ids.is_empty() {
        return encode_has_blob_response(&[]);
    }

    // Check which blobs exist via bulk status
    let mut has: Vec<String> = Vec::new();

    // Convert hex blob_ids to [u8; 32] and check via bridge
    for blob_hex in &blob_ids {
        if let Some(hash) = hex_to_hash(blob_hex) {
            match state.bridge.blob_status(&hash).await {
                Ok(status) if status.exists => {
                    has.push(blob_hex.clone());
                }
                _ => {} // doesn't exist or error
            }
        }
    }

    encode_has_blob_response(&has)
}

// ── Peer catalogue browsing & download handlers (FEAT-003-E) ─────────────────

#[derive(Debug, Deserialize)]
pub struct CatalogueQuery {
    #[serde(default)]
    pub cursor: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// GET /peer/{peer_id}/catalogue — browse a remote peer's catalogue via RPC.
pub async fn peer_catalogue(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
    Query(query): Query<CatalogueQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Verify peer is active
    {
        let active = state.active_peers.read().await;
        if !active.contains_key(&peer_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "peer not active" })),
            ));
        }
    }

    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(20).clamp(1, 100);

    // Decode peer_id from base64 to bytes
    let peer_id_bytes_vec = match base64_decode(&peer_id) {
        Some(b) if b.len() == 32 => b,
        _ => {
            return Err(bad_request("invalid peer_id (expected base64 of 32 bytes)"));
        }
    };
    let mut peer_id_bytes = [0u8; 32];
    peer_id_bytes.copy_from_slice(&peer_id_bytes_vec);

    // Build CBOR catalogue.list request
    let cbor_payload = encode_catalogue_list_request(cursor, limit);

    // RPC call to remote peer
    let response_bytes = state
        .bridge
        .rpc_call(&peer_id_bytes, "catalogue.list", &cbor_payload, Some(10000))
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("RPC call failed: {}", e) })),
            )
        })?;

    // Decode CBOR response
    let (offerings_json, total, next_cursor) =
        decode_catalogue_list_response_to_json(&response_bytes);

    Ok(Json(serde_json::json!({
        "offerings": offerings_json,
        "total": total,
        "next_cursor": next_cursor,
    })))
}

/// Decode a CBOR catalogue.list response into JSON-friendly values.
pub(crate) fn decode_catalogue_list_response_to_json(
    data: &[u8],
) -> (Vec<serde_json::Value>, i64, Option<i64>) {
    use ciborium::value::Value;

    let value: Value = match ciborium::from_reader(data) {
        Ok(v) => v,
        Err(_) => return (vec![], 0, None),
    };
    let map = match value {
        Value::Map(m) => m,
        _ => return (vec![], 0, None),
    };

    let mut offerings = vec![];
    let mut total: i64 = 0;
    let mut next_cursor: Option<i64> = None;

    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            match key as u64 {
                CBOR_KEY_OFFERINGS => {
                    if let Value::Array(arr) = v {
                        for item in arr {
                            if let Value::Map(fields) = item {
                                let mut obj = serde_json::Map::new();
                                for (fk, fv) in fields {
                                    if let Value::Text(field_name) = fk {
                                        let json_val = cbor_value_to_json(fv);
                                        obj.insert(field_name, json_val);
                                    }
                                }
                                offerings.push(serde_json::Value::Object(obj));
                            }
                        }
                    }
                }
                CBOR_KEY_TOTAL => {
                    if let Value::Integer(val) = v {
                        let n: i128 = val.into();
                        total = n as i64;
                    }
                }
                CBOR_KEY_NEXT_CURSOR => match v {
                    Value::Integer(val) => {
                        let n: i128 = val.into();
                        next_cursor = Some(n as i64);
                    }
                    Value::Null => {
                        next_cursor = None;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    (offerings, total, next_cursor)
}

/// Convert a ciborium Value to a serde_json Value.
pub(crate) fn cbor_value_to_json(v: ciborium::value::Value) -> serde_json::Value {
    use ciborium::value::Value;
    match v {
        Value::Text(s) => serde_json::Value::String(s),
        Value::Integer(i) => {
            let n: i128 = i.into();
            serde_json::json!(n as i64)
        }
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::Float(f) => serde_json::json!(f),
        Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(cbor_value_to_json).collect())
        }
        _ => serde_json::Value::Null,
    }
}

/// Decode the RPC method name from a CBOR payload.
pub(crate) fn decode_rpc_method(data: &[u8]) -> Option<String> {
    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(data).ok()?;
    let map = match value {
        Value::Map(m) => m,
        _ => return None,
    };
    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            if key as u64 == CBOR_KEY_METHOD {
                if let Value::Text(t) = v {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// Decode cursor and limit from a catalogue.list CBOR request.
pub(crate) fn decode_catalogue_list_params(data: &[u8]) -> (usize, usize) {
    use ciborium::value::Value;
    let value: Value = match ciborium::from_reader(data) {
        Ok(v) => v,
        Err(_) => return (0, 100),
    };
    let map = match value {
        Value::Map(m) => m,
        _ => return (0, 100),
    };

    let mut cursor: usize = 0;
    let mut limit: usize = 100;

    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            match key as u64 {
                CBOR_KEY_CURSOR => {
                    if let Value::Integer(val) = v {
                        let n: i128 = val.into();
                        cursor = n.max(0) as usize;
                    }
                }
                CBOR_KEY_LIMIT => {
                    if let Value::Integer(val) = v {
                        let n: i128 = val.into();
                        limit = n.clamp(1, 100) as usize;
                    }
                }
                _ => {}
            }
        }
    }

    (cursor, limit)
}

/// Decode blob_ids from a catalogue.has_blob CBOR request.
pub(crate) fn decode_has_blob_params(data: &[u8]) -> Vec<String> {
    use ciborium::value::Value;
    let value: Value = match ciborium::from_reader(data) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let map = match value {
        Value::Map(m) => m,
        _ => return vec![],
    };

    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            if key as u64 == CBOR_KEY_BLOB_IDS {
                if let Value::Array(arr) = v {
                    return arr
                        .into_iter()
                        .filter_map(|item| {
                            if let Value::Text(t) = item {
                                Some(t)
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

/// Encode a catalogue.list CBOR response.
pub(crate) fn encode_catalogue_list_response(
    offerings: &[crate::db::Offering],
    next_cursor: Option<usize>,
    total: usize,
) -> Vec<u8> {
    use ciborium::value::Value;

    let offering_values: Vec<Value> = offerings
        .iter()
        .map(|o| {
            Value::Map(vec![
                (
                    Value::Text("offering_id".to_string()),
                    Value::Text(o.offering_id.clone()),
                ),
                (Value::Text("name".to_string()), Value::Text(o.name.clone())),
                (
                    Value::Text("description".to_string()),
                    match &o.description {
                        Some(d) => Value::Text(d.clone()),
                        None => Value::Null,
                    },
                ),
                (
                    Value::Text("mime_type".to_string()),
                    Value::Text(o.mime_type.clone()),
                ),
                (
                    Value::Text("size".to_string()),
                    Value::Integer(o.size.into()),
                ),
                (
                    Value::Text("blob_id".to_string()),
                    Value::Text(o.blob_id.clone()),
                ),
                (
                    Value::Text("seeders".to_string()),
                    Value::Integer(1.into()), // initially just the operator
                ),
            ])
        })
        .collect();

    let mut map = vec![
        (
            Value::Integer(CBOR_KEY_OFFERINGS.into()),
            Value::Array(offering_values),
        ),
        (
            Value::Integer(CBOR_KEY_TOTAL.into()),
            Value::Integer((total as i64).into()),
        ),
    ];

    match next_cursor {
        Some(c) => {
            map.push((
                Value::Integer(CBOR_KEY_NEXT_CURSOR.into()),
                Value::Integer((c as i64).into()),
            ));
        }
        None => {
            map.push((Value::Integer(CBOR_KEY_NEXT_CURSOR.into()), Value::Null));
        }
    }

    let mut buf = Vec::new();
    ciborium::into_writer(&Value::Map(map), &mut buf).expect("CBOR catalogue response");
    buf
}

/// Encode a catalogue.has_blob CBOR response.
pub(crate) fn encode_has_blob_response(has: &[String]) -> Vec<u8> {
    use ciborium::value::Value;

    let has_values: Vec<Value> = has.iter().map(|s| Value::Text(s.clone())).collect();
    let map = Value::Map(vec![(
        Value::Integer(CBOR_KEY_HAS.into()),
        Value::Array(has_values),
    )]);

    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).expect("CBOR has_blob response");
    buf
}

// ── CBOR encode helpers for requests (used by tests + peer catalogue in FEAT-003-E) ──

/// Encode a catalogue.list CBOR request.
pub fn encode_catalogue_list_request(cursor: usize, limit: usize) -> Vec<u8> {
    use ciborium::value::Value;
    let map = Value::Map(vec![
        (
            Value::Integer(CBOR_KEY_METHOD.into()),
            Value::Text("catalogue.list".to_string()),
        ),
        (
            Value::Integer(CBOR_KEY_CURSOR.into()),
            Value::Integer((cursor as i64).into()),
        ),
        (
            Value::Integer(CBOR_KEY_LIMIT.into()),
            Value::Integer((limit as i64).into()),
        ),
    ]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).expect("CBOR catalogue list request");
    buf
}

/// Encode a catalogue.has_blob CBOR request.
#[allow(dead_code)]
pub fn encode_has_blob_request(blob_ids: &[String]) -> Vec<u8> {
    use ciborium::value::Value;
    let ids: Vec<Value> = blob_ids.iter().map(|s| Value::Text(s.clone())).collect();
    let map = Value::Map(vec![
        (
            Value::Integer(CBOR_KEY_METHOD.into()),
            Value::Text("catalogue.has_blob".to_string()),
        ),
        (Value::Integer(CBOR_KEY_BLOB_IDS.into()), Value::Array(ids)),
    ]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).expect("CBOR has_blob request");
    buf
}

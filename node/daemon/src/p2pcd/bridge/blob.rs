use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use super::{
    decode_hex_hash, decode_payload, decode_peer_id, get_blob_store, BlobDataQuery,
    BlobRequestRequest, BlobRequestResponse, BlobStatusQuery, BlobStatusResponse, BlobStoreRequest,
    BlobStoreResponse, BridgeState, BulkBlobStatusRequest,
};

/// POST /p2pcd/bridge/blob/store — store a blob by hash.
pub async fn handle_blob_store(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<BlobStoreRequest>,
) -> impl IntoResponse {
    let hash = match decode_hex_hash(&req.hash) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobStoreResponse {
                    ok: false,
                    size: None,
                    error: Some(e),
                }),
            )
        }
    };

    let data = match decode_payload(&req.data) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobStoreResponse {
                    ok: false,
                    size: None,
                    error: Some(e),
                }),
            )
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BlobStoreResponse {
                    ok: false,
                    size: None,
                    error: Some(e),
                }),
            )
        }
    };

    let mut writer = store.begin_write(hash);
    if let Err(e) = writer.write(&data).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BlobStoreResponse {
                ok: false,
                size: None,
                error: Some(format!("write failed: {e}")),
            }),
        );
    }

    match writer.finalize().await {
        Ok(size) => (
            StatusCode::OK,
            Json(BlobStoreResponse {
                ok: true,
                size: Some(size),
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BlobStoreResponse {
                ok: false,
                size: None,
                error: Some(format!("finalize failed: {e}")),
            }),
        ),
    }
}

/// POST /p2pcd/bridge/blob/request — request a blob from a remote peer.
pub async fn handle_blob_request(
    State(BridgeState {
        engine,
        callback_registry,
        ..
    }): State<BridgeState>,
    Json(req): Json<BlobRequestRequest>,
) -> impl IntoResponse {
    let peer_id = match decode_peer_id(&req.peer_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobRequestResponse {
                    ok: false,
                    error: Some(e),
                }),
            )
        }
    };

    let hash = match decode_hex_hash(&req.hash) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobRequestResponse {
                    ok: false,
                    error: Some(e),
                }),
            )
        }
    };

    // Register callback if provided
    if let Some(url) = req.callback_url {
        callback_registry.register(req.transfer_id, url).await;
    }

    // Build BLOB_REQ message: { 1: transfer_id, 2: blob_hash }
    use p2pcd::cbor_helpers::{cbor_encode_map, make_capability_msg};
    let payload = cbor_encode_map(vec![
        (
            1, // TRANSFER_ID
            ciborium::value::Value::Integer(req.transfer_id.into()),
        ),
        (
            2, // BLOB_HASH
            ciborium::value::Value::Bytes(hash.to_vec()),
        ),
    ]);
    let msg = make_capability_msg(p2pcd_types::message_types::BLOB_REQ, payload);

    match engine.send_to_peer(&peer_id, msg).await {
        Ok(()) => (
            StatusCode::OK,
            Json(BlobRequestResponse {
                ok: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(BlobRequestResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        ),
    }
}

/// GET /p2pcd/bridge/blob/status — check if a blob exists locally.
pub async fn handle_blob_status(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Query(query): Query<BlobStatusQuery>,
) -> impl IntoResponse {
    let hash = match decode_hex_hash(&query.hash) {
        Ok(h) => h,
        Err(_e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobStatusResponse {
                    exists: false,
                    size: None,
                }),
            )
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BlobStatusResponse {
                    exists: false,
                    size: None,
                }),
            )
        }
    };

    if store.has(&hash).await {
        let size = store.size(&hash).await;
        (
            StatusCode::OK,
            Json(BlobStatusResponse { exists: true, size }),
        )
    } else {
        (
            StatusCode::OK,
            Json(BlobStatusResponse {
                exists: false,
                size: None,
            }),
        )
    }
}

/// GET /p2pcd/bridge/blob/data — read blob data.
pub async fn handle_blob_data(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Query(query): Query<BlobDataQuery>,
) -> axum::response::Response {
    use axum::http::header;

    let hash = match decode_hex_hash(&query.hash) {
        Ok(h) => h,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    };

    if !store.has(&hash).await {
        return (StatusCode::NOT_FOUND, "blob not found").into_response();
    }

    // Determine read length
    let total_size = store.size(&hash).await.unwrap_or(0);
    let offset = query.offset;
    let length = if query.length == 0 {
        total_size.saturating_sub(offset)
    } else {
        query.length
    };

    match store.read_chunk(&hash, offset, length).await {
        Ok(data) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            data,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read failed: {e}"),
        )
            .into_response(),
    }
}

/// POST /p2pcd/bridge/blob/status/bulk — check multiple blobs at once.
pub async fn handle_bulk_blob_status(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<BulkBlobStatusRequest>,
) -> impl IntoResponse {
    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };

    let mut results = serde_json::Map::new();
    for hex_hash in &req.hashes {
        if let Ok(hash) = decode_hex_hash(hex_hash) {
            let exists = store.has(&hash).await;
            let size = if exists {
                store.size(&hash).await
            } else {
                None
            };
            results.insert(
                hex_hash.clone(),
                serde_json::json!({ "exists": exists, "size": size }),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "results": results })),
    )
}

/// DELETE /p2pcd/bridge/blob/{hash} — delete a blob from the store.
pub async fn handle_blob_delete(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Path(hash_hex): Path<String>,
) -> impl IntoResponse {
    let hash = match decode_hex_hash(&hash_hex) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": e })),
            )
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "ok": false, "error": e })),
            )
        }
    };

    match store.delete(&hash).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "deleted": deleted })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

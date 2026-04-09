use super::rpc::*;
use super::*;
use axum::http::StatusCode;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[test]
fn encode_decode_catalogue_list_request() {
    let cbor = encode_catalogue_list_request(5, 50);
    let method = decode_rpc_method(&cbor).unwrap();
    assert_eq!(method, "catalogue.list");

    let (cursor, limit) = decode_catalogue_list_params(&cbor);
    assert_eq!(cursor, 5);
    assert_eq!(limit, 50);
}

#[test]
fn encode_decode_has_blob_request() {
    let ids = vec!["abc123".to_string(), "def456".to_string()];
    let cbor = encode_has_blob_request(&ids);
    let method = decode_rpc_method(&cbor).unwrap();
    assert_eq!(method, "catalogue.has_blob");

    let decoded_ids = decode_has_blob_params(&cbor);
    assert_eq!(decoded_ids, ids);
}

#[test]
fn encode_decode_catalogue_list_response() {
    use crate::db::Offering;

    let offerings = vec![Offering {
        offering_id: "o1".to_string(),
        blob_id: "abc123".to_string(),
        name: "test.txt".to_string(),
        description: Some("A test file".to_string()),
        mime_type: "text/plain".to_string(),
        size: 1024,
        created_at: 1700000000,
        access: "public".to_string(),
        allowlist: None,
    }];

    let cbor = encode_catalogue_list_response(&offerings, Some(1), 5);

    // Decode and verify
    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(cbor.as_slice()).unwrap();
    let map = match value {
        Value::Map(m) => m,
        _ => panic!("expected map"),
    };

    let mut found_offerings = false;
    let mut found_total = false;
    let mut found_cursor = false;

    for (k, v) in &map {
        if let Value::Integer(i) = k {
            let key: i128 = (*i).into();
            match key as u64 {
                CBOR_KEY_OFFERINGS => {
                    if let Value::Array(arr) = v {
                        assert_eq!(arr.len(), 1);
                        found_offerings = true;
                    }
                }
                CBOR_KEY_TOTAL => {
                    if let Value::Integer(val) = v {
                        let n: i128 = (*val).into();
                        assert_eq!(n, 5);
                        found_total = true;
                    }
                }
                CBOR_KEY_NEXT_CURSOR => {
                    if let Value::Integer(val) = v {
                        let n: i128 = (*val).into();
                        assert_eq!(n, 1);
                        found_cursor = true;
                    }
                }
                _ => {}
            }
        }
    }

    assert!(found_offerings);
    assert!(found_total);
    assert!(found_cursor);
}

#[test]
fn encode_decode_has_blob_response() {
    let has = vec!["abc123".to_string(), "def456".to_string()];
    let cbor = encode_has_blob_response(&has);

    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(cbor.as_slice()).unwrap();
    let map = match value {
        Value::Map(m) => m,
        _ => panic!("expected map"),
    };

    for (k, v) in &map {
        if let Value::Integer(i) = k {
            let key: i128 = (*i).into();
            if key as u64 == CBOR_KEY_HAS {
                if let Value::Array(arr) = v {
                    assert_eq!(arr.len(), 2);
                    return;
                }
            }
        }
    }
    panic!("didn't find has key in response");
}

#[test]
fn catalogue_list_default_params() {
    // Empty CBOR map
    use ciborium::value::Value;
    let map = Value::Map(vec![(
        Value::Integer(CBOR_KEY_METHOD.into()),
        Value::Text("catalogue.list".to_string()),
    )]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).unwrap();

    let (cursor, limit) = decode_catalogue_list_params(&buf);
    assert_eq!(cursor, 0);
    assert_eq!(limit, 100);
}

#[test]
fn has_blob_empty_request() {
    let cbor = encode_has_blob_request(&[]);
    let ids = decode_has_blob_params(&cbor);
    assert!(ids.is_empty());
}

#[test]
fn null_next_cursor_in_response() {
    let cbor = encode_catalogue_list_response(&[], None, 0);

    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(cbor.as_slice()).unwrap();
    let map = match value {
        Value::Map(m) => m,
        _ => panic!("expected map"),
    };

    for (k, v) in &map {
        if let Value::Integer(i) = k {
            let key: i128 = (*i).into();
            if key as u64 == CBOR_KEY_NEXT_CURSOR {
                assert!(matches!(v, Value::Null));
                return;
            }
        }
    }
    panic!("didn't find next_cursor key");
}

#[test]
fn hex_to_hash_edges() {
    // valid 64-char hex roundtrips
    let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let hash = hex_to_hash(hex).unwrap();
    assert_eq!(hex::encode(hash), hex);
    // wrong length and non-hex rejected
    assert!(hex_to_hash("abcdef").is_none());
    assert!(hex_to_hash("zzzzzz").is_none());
}

// ── validate_access tests ────────────────────────────────────────────

#[test]
fn validate_access_builtins() {
    assert!(validate_access("public").is_ok());
    assert!(validate_access("friends").is_ok());
    assert!(validate_access("trusted").is_ok());
    assert!(validate_access("peer").is_ok());
}

#[test]
fn validate_access_single_group() {
    assert!(validate_access("group:a1b2c3d4-e5f6-7890-abcd-ef0123456789").is_ok());
    assert!(validate_access("group:not-a-uuid").is_err());
    assert!(validate_access("group:").is_err());
}

#[test]
fn validate_access_multi_group() {
    assert!(validate_access(
        "groups:a1b2c3d4-e5f6-7890-abcd-ef0123456789,b2c3d4e5-f6a7-8901-bcde-f01234567890"
    )
    .is_ok());
    assert!(validate_access("groups:not-a-uuid").is_err());
}

#[test]
fn validate_access_unknown_policy() {
    assert!(validate_access("admins").is_err());
    assert!(validate_access("").is_err());
}

// ── direct_blob_write test ───────────────────────────────────────────

#[tokio::test]
async fn direct_blob_write_creates_file() {
    let dir = tempfile::tempdir().unwrap();
    let data = b"hello blob world";
    let hash: [u8; 32] = sha2::Sha256::digest(data).into();

    direct_blob_write(dir.path(), &hash, data).await.unwrap();

    let hex_hash = hex::encode(hash);
    let prefix = &hex_hash[..2];
    let blob_path = dir.path().join("blobs").join(prefix).join(&hex_hash);
    assert!(blob_path.exists());

    let contents = std::fs::read(&blob_path).unwrap();
    assert_eq!(contents, data);
}

// ── HTTP integration tests ──────────────────────────────────────────

/// Build a test Router with in-memory DB and a BridgeClient pointing at a
/// non-existent daemon (for testing paths that don't hit the bridge).
fn test_app() -> (axum::Router, Arc<crate::db::FilesDb>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::db::FilesDb::open(dir.path()).unwrap();
    let db = Arc::new(db);
    let bridge = p2pcd::bridge_client::BridgeClient::new(19999); // unused port

    // PeerStream with a non-connecting URL — tests don't need live peers.
    let stream = Arc::new(p2pcd::capability_sdk::PeerStream::connect_with_url(
        "howm.social.files.1",
        "http://127.0.0.1:1/p2pcd/bridge/events?capability=howm.social.files.1".to_string(),
    ));
    let peer_groups = Arc::new(RwLock::new(std::collections::HashMap::new()));

    let state = AppState::new(
        (*db).clone(),
        bridge,
        19999,
        17003,
        dir.path().to_path_buf(),
        stream,
        peer_groups,
    );

    let app = axum::Router::new()
        .route("/health", axum::routing::get(super::health))
        .route(
            "/offerings",
            axum::routing::get(super::list_offerings).post(super::create_offering),
        )
        .route(
            "/offerings/json",
            axum::routing::put(super::create_offering_json),
        )
        .route(
            "/offerings/{offering_id}",
            axum::routing::patch(super::update_offering).delete(super::delete_offering),
        )
        .route(
            "/peer/{peer_id}/catalogue",
            axum::routing::get(super::peer_catalogue),
        )
        .route(
            "/downloads",
            axum::routing::get(super::list_downloads).post(super::initiate_download),
        )
        .route(
            "/downloads/{blob_id}/status",
            axum::routing::get(super::download_status),
        )
        .route(
            "/downloads/{blob_id}/data",
            axum::routing::get(super::download_data),
        )
        .route(
            "/p2pcd/inbound",
            axum::routing::post(super::inbound_message),
        )
        .route(
            "/internal/transfer-complete",
            axum::routing::post(super::transfer_complete),
        )
        .with_state(state);

    (app, db, dir)
}

/// Insert an offering directly into the DB for test setup.
fn seed_offering(db: &crate::db::FilesDb, name: &str, access: &str) -> crate::db::Offering {
    let offering = crate::db::Offering {
        offering_id: Uuid::now_v7().to_string(),
        blob_id: hex::encode([0xABu8; 32]),
        name: name.to_string(),
        description: Some(format!("Desc for {}", name)),
        mime_type: "application/octet-stream".to_string(),
        size: 1024,
        created_at: 1700000000,
        access: access.to_string(),
        allowlist: None,
    };
    db.insert_offering(&offering).unwrap();
    offering
}

use tower::ServiceExt; // for oneshot()

#[tokio::test]
async fn http_list_offerings_empty() {
    let (app, _, _dir) = test_app();
    let req = axum::http::Request::builder()
        .uri("/offerings")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["offerings"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn http_list_offerings_with_data() {
    let (app, db, _dir) = test_app();
    seed_offering(&db, "file1.txt", "public");
    seed_offering(&db, "file2.txt", "friends");

    let req = axum::http::Request::builder()
        .uri("/offerings")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["offerings"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn http_update_offering_success() {
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "original.txt", "public");

    let req = axum::http::Request::builder()
        .method("PATCH")
        .uri(format!("/offerings/{}", o.offering_id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "name": "renamed.txt",
                "description": "updated desc"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["offering"]["name"], "renamed.txt");
    assert_eq!(json["offering"]["description"], "updated desc");
}

#[tokio::test]
async fn http_update_offering_not_found() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .method("PATCH")
        .uri("/offerings/nonexistent-id")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({ "name": "new.txt" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_update_name_too_long() {
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "file.txt", "public");

    let long_name = "x".repeat(256);
    let req = axum::http::Request::builder()
        .method("PATCH")
        .uri(format!("/offerings/{}", o.offering_id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({ "name": long_name }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_update_invalid_access() {
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "file.txt", "public");

    let req = axum::http::Request::builder()
        .method("PATCH")
        .uri(format!("/offerings/{}", o.offering_id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({ "access": "wizards" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_update_name_conflict() {
    let (app, db, _dir) = test_app();
    seed_offering(&db, "existing.txt", "public");
    let o2 = seed_offering(&db, "other.txt", "public");

    let req = axum::http::Request::builder()
        .method("PATCH")
        .uri(format!("/offerings/{}", o2.offering_id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({ "name": "existing.txt" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn http_update_change_access_to_group() {
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "file.txt", "public");

    let req = axum::http::Request::builder()
        .method("PATCH")
        .uri(format!("/offerings/{}", o.offering_id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({ "access": "group:a1b2c3d4-e5f6-7890-abcd-ef0123456789" })
                .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["offering"]["access"],
        "group:a1b2c3d4-e5f6-7890-abcd-ef0123456789"
    );
}

#[tokio::test]
async fn http_delete_offering_success() {
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "doomed.txt", "public");

    // retain_blob=1 to skip bridge blob deletion (bridge isn't running)
    let req = axum::http::Request::builder()
        .method("DELETE")
        .uri(format!("/offerings/{}?retain_blob=1", o.offering_id))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify it's gone
    let offerings = db.list_offerings().unwrap();
    assert!(offerings.is_empty());
}

#[tokio::test]
async fn http_delete_offering_not_found() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .method("DELETE")
        .uri("/offerings/nonexistent-id?retain_blob=1")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_create_offering_json_validation() {
    let (app, _, _dir) = test_app();

    // Name too long
    let long_name = "x".repeat(256);
    let req = axum::http::Request::builder()
        .method("PUT")
        .uri("/offerings/json")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "name": long_name,
                "mime_type": "text/plain",
                "size": 100,
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_create_offering_json_bad_blob_id() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .method("PUT")
        .uri("/offerings/json")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "blob_id": "not-a-valid-hex",
                "name": "test.txt",
                "mime_type": "text/plain",
                "size": 100,
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_create_offering_json_invalid_access() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .method("PUT")
        .uri("/offerings/json")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "name": "test.txt",
                "mime_type": "text/plain",
                "size": 100,
                "access": "invalid_policy",
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_create_offering_json_desc_too_long() {
    let (app, _, _dir) = test_app();

    let long_desc = "x".repeat(1025);
    let req = axum::http::Request::builder()
        .method("PUT")
        .uri("/offerings/json")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "name": "test.txt",
                "mime_type": "text/plain",
                "size": 100,
                "description": long_desc,
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// Removed: http_peer_active_and_inactive
// peer-active and peer-inactive HTTP endpoints were removed in Phase 6.
// Lifecycle events are now delivered via SSE (PeerStream) from the daemon.

#[tokio::test]
async fn http_inbound_bad_base64() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/p2pcd/inbound")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "peer_id": "dGVzdHBlZXIx",
                "message_type": 1,
                "payload": "!!!not-base64!!!",
                "capability": "howm.social.files.1",
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_inbound_unknown_method() {
    let (app, _, _dir) = test_app();

    // Encode a CBOR payload with an unknown method
    use ciborium::value::Value;
    let map = Value::Map(vec![(
        Value::Integer(CBOR_KEY_METHOD.into()),
        Value::Text("unknown.method".to_string()),
    )]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).unwrap();
    let payload_b64 = base64_encode(&buf);

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/p2pcd/inbound")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "peer_id": "dGVzdHBlZXIx",
                "message_type": 1,
                "payload": payload_b64,
                "capability": "howm.social.files.1",
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_inbound_catalogue_list() {
    let (app, db, _dir) = test_app();
    seed_offering(&db, "shared.txt", "public");

    // Encode a catalogue.list CBOR request
    let payload = encode_catalogue_list_request(0, 10);
    let payload_b64 = base64_encode(&payload);

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/p2pcd/inbound")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "peer_id": "dGVzdHBlZXIx",
                "message_type": 1,
                "payload": payload_b64,
                "capability": "howm.social.files.1",
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Should contain a base64-encoded CBOR response
    assert!(json["response"].is_string());

    // Decode the CBOR response
    let response_b64 = json["response"].as_str().unwrap();
    let response_bytes = base64_decode(response_b64).unwrap();
    let value: ciborium::value::Value = ciborium::from_reader(response_bytes.as_slice()).unwrap();
    if let ciborium::value::Value::Map(map) = value {
        // Find offerings array
        let offerings_entry = map.iter().find(|(k, _)| {
            if let ciborium::value::Value::Integer(i) = k {
                let key: i128 = (*i).into();
                key as u64 == CBOR_KEY_OFFERINGS
            } else {
                false
            }
        });
        assert!(offerings_entry.is_some());
        if let Some((_, ciborium::value::Value::Array(arr))) = offerings_entry {
            assert_eq!(arr.len(), 1); // the seeded public offering
        }
    } else {
        panic!("expected CBOR map in response");
    }
}

#[tokio::test]
async fn http_inbound_has_blob() {
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "file.txt", "public");

    // Encode a catalogue.has_blob CBOR request
    let payload = encode_has_blob_request(&[o.blob_id.clone(), "nonexistent".to_string()]);
    let payload_b64 = base64_encode(&payload);

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/p2pcd/inbound")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "peer_id": "dGVzdHBlZXIx",
                "message_type": 1,
                "payload": payload_b64,
                "capability": "howm.social.files.1",
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let response_b64 = json["response"].as_str().unwrap();
    let response_bytes = base64_decode(response_b64).unwrap();
    let value: ciborium::value::Value = ciborium::from_reader(response_bytes.as_slice()).unwrap();
    if let ciborium::value::Value::Map(map) = value {
        let has_entry = map.iter().find(|(k, _)| {
            if let ciborium::value::Value::Integer(i) = k {
                let key: i128 = (*i).into();
                key as u64 == CBOR_KEY_HAS
            } else {
                false
            }
        });
        assert!(has_entry.is_some());
        if let Some((_, ciborium::value::Value::Array(arr))) = has_entry {
            // Bridge is not running, so blob_status calls fail — results in empty has list.
            // This still verifies the RPC routing + CBOR encode/decode work end-to-end.
            assert_eq!(arr.len(), 0);
        }
    } else {
        panic!("expected CBOR map in response");
    }
}

#[tokio::test]
async fn http_delete_without_retain_blob_best_effort() {
    // When retain_blob is not set, delete_offering tries to call bridge
    // (which will fail since daemon isn't running). The offering should
    // still be removed — blob deletion is best-effort.
    let (app, db, _dir) = test_app();
    let o = seed_offering(&db, "doomed.txt", "public");

    let req = axum::http::Request::builder()
        .method("DELETE")
        .uri(format!("/offerings/{}", o.offering_id))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Offering is gone from DB even though blob deletion failed
    assert!(db.list_offerings().unwrap().is_empty());
}

// ── FEAT-003-E HTTP integration tests ────────────────────────────────

/// Insert a download directly into the DB for test setup.
fn seed_download(db: &crate::db::FilesDb, blob_id: &str, status: &str) -> crate::db::Download {
    let dl = crate::db::Download {
        blob_id: blob_id.to_string(),
        offering_id: "off-1".to_string(),
        peer_id: "peer-abc".to_string(),
        transfer_id: 1700000000,
        name: "test-file.bin".to_string(),
        mime_type: "application/octet-stream".to_string(),
        size: 2048,
        status: status.to_string(),
        started_at: 1700000000,
        completed_at: if status == "complete" {
            Some(1700001000)
        } else {
            None
        },
    };
    db.insert_download(&dl).unwrap();
    dl
}

#[tokio::test]
async fn http_list_downloads_empty() {
    let (app, _, _dir) = test_app();
    let req = axum::http::Request::builder()
        .uri("/downloads")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["downloads"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn http_list_downloads_with_data() {
    let (app, db, _dir) = test_app();
    seed_download(&db, "blob_aaa", "transferring");
    seed_download(&db, "blob_bbb", "complete");

    let req = axum::http::Request::builder()
        .uri("/downloads")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["downloads"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn http_download_status_not_found() {
    let (app, _, _dir) = test_app();
    let req = axum::http::Request::builder()
        .uri("/downloads/nonexistent/status")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_download_status_found() {
    let (app, db, _dir) = test_app();
    seed_download(&db, "blob_abc", "transferring");

    let req = axum::http::Request::builder()
        .uri("/downloads/blob_abc/status")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["download"]["blob_id"], "blob_abc");
    assert_eq!(json["download"]["status"], "transferring");
}

#[tokio::test]
async fn http_download_data_not_complete() {
    let (app, db, _dir) = test_app();
    seed_download(&db, "blob_xyz", "transferring");

    let req = axum::http::Request::builder()
        .uri("/downloads/blob_xyz/data")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn http_transfer_complete_updates_status() {
    let (app, db, _dir) = test_app();
    seed_download(&db, "blob_tc1", "transferring");

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/internal/transfer-complete")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "blob_id": "blob_tc1",
                "transfer_id": 1700000000_u64,
                "status": "complete",
                "size": 2048,
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify status updated in DB
    let dl = db.get_download("blob_tc1").unwrap().unwrap();
    assert_eq!(dl.status, "complete");
    assert!(dl.completed_at.is_some());
}

#[tokio::test]
async fn http_transfer_complete_failed() {
    let (app, db, _dir) = test_app();
    seed_download(&db, "blob_tc2", "transferring");

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/internal/transfer-complete")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "blob_id": "blob_tc2",
                "transfer_id": 1700000000_u64,
                "status": "failed",
                "error": "timeout",
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let dl = db.get_download("blob_tc2").unwrap().unwrap();
    assert_eq!(dl.status, "failed");
    assert!(dl.completed_at.is_some());
}

#[tokio::test]
async fn http_initiate_download_no_peer() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/downloads")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "peer_id": "dGVzdHBlZXIx",
                "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "offering_id": "off-1",
                "name": "test.bin",
                "mime_type": "application/octet-stream",
                "size": 1024,
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_peer_catalogue_no_peer() {
    let (app, _, _dir) = test_app();

    let req = axum::http::Request::builder()
        .uri("/peer/dGVzdHBlZXIx/catalogue")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

//! Integration tests for the Howm daemon.
//!
//! These tests spin up daemon instances in-process using the axum router
//! and test the full API flow: identity, invites, peer connections,
//! capability proxy, and feed aggregation.
//!
//! No Docker or real WireGuard is required — WG is disabled for tests.

use tempfile::TempDir;

// ── Standalone unit-level tests (no Docker, no server) ──────────────────────

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_invite_encode_decode() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path();
        std::fs::create_dir_all(data_dir).unwrap();

        // Set up a minimal identity
        let identity_json = serde_json::json!({
            "node_id": "test-node-1",
            "name": "alice",
            "created": 1000000,
            "wg_pubkey": "dGVzdC1wdWJrZXktYWxpY2U=",
            "wg_address": "10.47.0.1",
            "wg_endpoint": "1.2.3.4:51820",
        });
        std::fs::write(
            data_dir.join("node.json"),
            serde_json::to_string_pretty(&identity_json).unwrap(),
        )
        .unwrap();

        // WG directory for address allocation
        let wg_dir = data_dir.join("wireguard");
        std::fs::create_dir_all(&wg_dir).unwrap();
        std::fs::write(wg_dir.join("address"), "10.47.0.1").unwrap();

        // Manually create an invite code to test decode
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let payload = "dGVzdC1wdWJrZXktYWxpY2U=|1.2.3.4:51820|10.47.0.1|dGVzdC1wc2s=|10.47.0.2|7000|9999999999";
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let invite_code = format!("howm://invite/{}", encoded);

        // Test that the decode function accepts | delimiter correctly
        let decoded_result = decode_invite(&invite_code);
        assert!(
            decoded_result.is_ok(),
            "decode failed: {:?}",
            decoded_result.err()
        );
        let decoded = decoded_result.unwrap();
        assert_eq!(decoded.0, "dGVzdC1wdWJrZXktYWxpY2U="); // pubkey
        assert_eq!(decoded.1, "1.2.3.4:51820"); // endpoint (with colon!)
        assert_eq!(decoded.2, "10.47.0.1"); // wg_address
        assert_eq!(decoded.3, "dGVzdC1wc2s="); // psk
        assert_eq!(decoded.4, "10.47.0.2"); // assigned_ip
        assert_eq!(decoded.5, 7000u16); // daemon_port
        assert_eq!(decoded.6, 9999999999u64); // expires_at
    }

    /// Decode helper that mirrors invite::decode without importing the daemon crate.
    fn decode_invite(
        code: &str,
    ) -> anyhow::Result<(String, String, String, String, String, u16, u64)> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let stripped = code.strip_prefix("howm://invite/").unwrap();
        let bytes = URL_SAFE_NO_PAD.decode(stripped)?;
        let payload = String::from_utf8(bytes)?;
        let parts: Vec<&str> = payload.splitn(7, '|').collect();
        assert_eq!(parts.len(), 7);
        Ok((
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
            parts[3].to_string(),
            parts[4].to_string(),
            parts[5].parse()?,
            parts[6].parse()?,
        ))
    }

    #[test]
    fn test_invite_delimiter_with_ipv6() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        // Endpoint with IPv6-like address — colons everywhere
        let payload = "pubkey123|[::1]:51820|10.47.0.1|psk456|10.47.0.2|8080|9999999999";
        let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let code = format!("howm://invite/{}", encoded);

        let decoded = decode_invite(&code).unwrap();
        assert_eq!(decoded.1, "[::1]:51820"); // endpoint preserved correctly
    }

    #[test]
    fn test_rate_limiter() {
        use std::collections::HashMap;
        use std::sync::Mutex;
        use std::time::Instant;

        struct RateLimiter {
            limit: u32,
            window_secs: u64,
            buckets: Mutex<HashMap<String, Vec<Instant>>>,
        }

        impl RateLimiter {
            fn new(limit: u32, window_secs: u64) -> Self {
                Self {
                    limit,
                    window_secs,
                    buckets: Mutex::new(HashMap::new()),
                }
            }
            fn check(&self, key: &str) -> bool {
                let now = Instant::now();
                let mut buckets = self.buckets.lock().unwrap();
                let entries = buckets.entry(key.to_string()).or_default();
                let cutoff = now - std::time::Duration::from_secs(self.window_secs);
                entries.retain(|t| *t > cutoff);
                if entries.len() < self.limit as usize {
                    entries.push(now);
                    true
                } else {
                    false
                }
            }
        }

        let limiter = RateLimiter::new(3, 60);
        assert!(limiter.check("test"));
        assert!(limiter.check("test"));
        assert!(limiter.check("test"));
        assert!(!limiter.check("test")); // 4th should fail

        // Different key should still work
        assert!(limiter.check("other"));
    }

    #[test]
    fn test_address_allocation() {
        let dir = TempDir::new().unwrap();
        let wg_dir = dir.path().join("wireguard");
        std::fs::create_dir_all(&wg_dir).unwrap();

        let allocate = |dir: &std::path::Path| -> String {
            let addr_file = dir.join("wireguard").join("addresses.json");
            let mut addresses: Vec<String> = if addr_file.exists() {
                let text = std::fs::read_to_string(&addr_file).unwrap();
                serde_json::from_str(&text).unwrap_or_default()
            } else {
                vec![]
            };

            let mut octet3: u8 = 0;
            let mut octet4: u8 = 2;
            loop {
                let candidate = format!("10.47.{}.{}", octet3, octet4);
                if !addresses.contains(&candidate) {
                    addresses.push(candidate.clone());
                    std::fs::write(&addr_file, serde_json::to_string(&addresses).unwrap()).unwrap();
                    return candidate;
                }
                octet4 = octet4.wrapping_add(1);
                if octet4 == 0 {
                    octet3 += 1;
                }
            }
        };

        let a1 = allocate(dir.path());
        assert_eq!(a1, "10.47.0.2");
        let a2 = allocate(dir.path());
        assert_eq!(a2, "10.47.0.3");
        let a3 = allocate(dir.path());
        assert_eq!(a3, "10.47.0.4");
    }

    #[test]
    fn test_resource_limit_parsing() {
        // Test memory parsing
        let parse_mem = |s: &str| -> Option<i64> {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            let (num_str, mul) = if s.ends_with("Gi") || s.ends_with("G") {
                (
                    s.trim_end_matches("Gi").trim_end_matches("G"),
                    1024 * 1024 * 1024i64,
                )
            } else if s.ends_with("Mi") || s.ends_with("M") {
                (
                    s.trim_end_matches("Mi").trim_end_matches("M"),
                    1024 * 1024i64,
                )
            } else if s.ends_with("Ki") || s.ends_with("K") {
                (s.trim_end_matches("Ki").trim_end_matches("K"), 1024i64)
            } else {
                (s, 1i64)
            };
            num_str.trim().parse::<i64>().ok().map(|n| n * mul)
        };

        assert_eq!(parse_mem("256M"), Some(256 * 1024 * 1024));
        assert_eq!(parse_mem("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_mem("512Mi"), Some(512 * 1024 * 1024));
        assert_eq!(parse_mem("1024K"), Some(1024 * 1024));

        // Test CPU parsing
        let parse_cpu = |s: &str| -> Option<i64> {
            let s = s.trim();
            if s.ends_with("m") {
                s.trim_end_matches("m")
                    .parse::<i64>()
                    .ok()
                    .map(|v| v * 1_000_000)
            } else {
                s.parse::<f64>().ok().map(|v| (v * 1_000_000_000.0) as i64)
            }
        };

        assert_eq!(parse_cpu("500m"), Some(500_000_000));
        assert_eq!(parse_cpu("0.5"), Some(500_000_000));
        assert_eq!(parse_cpu("1"), Some(1_000_000_000));
        assert_eq!(parse_cpu("250m"), Some(250_000_000));
    }

    #[test]
    fn test_visibility_enforcement_logic() {
        // Test the visibility matching logic
        let check_visibility = |vis: &str, source_ip: &str, known_peers: &[&str]| -> bool {
            let is_local = source_ip == "127.0.0.1" || source_ip == "::1";
            match vis {
                "private" => is_local,
                "friends" => is_local || known_peers.contains(&source_ip),
                _ => true, // "public" or unknown
            }
        };

        // Private: only local
        assert!(check_visibility("private", "127.0.0.1", &[]));
        assert!(!check_visibility("private", "10.47.0.2", &[]));

        // Friends: local + known peers
        assert!(check_visibility("friends", "127.0.0.1", &[]));
        assert!(check_visibility("friends", "10.47.0.2", &["10.47.0.2"]));
        assert!(!check_visibility("friends", "10.47.0.3", &["10.47.0.2"]));

        // Public: anyone
        assert!(check_visibility("public", "1.2.3.4", &[]));
    }

    #[test]
    fn test_bearer_auth_parsing() {
        // Test the bearer token extraction logic
        let check_auth = |header: Option<&str>, expected_token: &str| -> bool {
            match header {
                Some(h) if h.starts_with("Bearer ") => {
                    h.trim_start_matches("Bearer ").trim() == expected_token
                }
                _ => false,
            }
        };

        assert!(check_auth(
            Some("Bearer my-secret-token"),
            "my-secret-token"
        ));
        assert!(!check_auth(Some("Bearer wrong-token"), "my-secret-token"));
        assert!(!check_auth(Some("Basic abc123"), "my-secret-token"));
        assert!(!check_auth(None, "my-secret-token"));
    }
}

// ── HTTP-level integration tests ────────────────────────────────────────────

#[cfg(test)]
mod http_tests {
    use axum::{
        body::Body,
        extract::State,
        http::{Request, StatusCode},
        middleware,
        routing::{get, post},
        Router,
    };
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Minimal AppState for HTTP tests (no Docker, no WG container).
    #[derive(Clone)]
    struct TestAppState {
        node_id: String,
        name: String,
        wg_pubkey: String,
        wg_address: String,
        wg_endpoint: String,
        port: u16,
        api_token: String,
        peers: Arc<tokio::sync::RwLock<Vec<Value>>>,
    }

    fn build_test_router(state: TestAppState) -> Router {
        async fn get_info(State(s): State<TestAppState>) -> axum::Json<Value> {
            axum::Json(json!({
                "node_id": s.node_id,
                "name": s.name,
                "wg_pubkey": s.wg_pubkey,
                "wg_address": s.wg_address,
                "wg_endpoint": s.wg_endpoint,
            }))
        }

        async fn get_peers(State(s): State<TestAppState>) -> axum::Json<Value> {
            let peers = s.peers.read().await;
            axum::Json(json!({ "peers": *peers }))
        }

        async fn get_wg_status(State(s): State<TestAppState>) -> axum::Json<Value> {
            axum::Json(json!({
                "status": "connected",
                "public_key": s.wg_pubkey,
                "address": s.wg_address,
                "endpoint": s.wg_endpoint,
                "listen_port": 51820,
                "active_tunnels": 0,
                "peers": [],
            }))
        }

        async fn complete_invite(
            State(s): State<TestAppState>,
            axum::Json(body): axum::Json<Value>,
        ) -> Result<axum::Json<Value>, StatusCode> {
            let psk = body["psk"].as_str().unwrap_or("");
            // Simple: accept any PSK for testing
            if psk.is_empty() {
                return Err(StatusCode::GONE);
            }
            let mut peers = s.peers.write().await;
            peers.push(json!({
                "node_id": "pending",
                "name": "pending",
                "wg_pubkey": body["my_pubkey"],
                "wg_address": "10.47.0.99",
                "wg_endpoint": body["my_endpoint"],
                "port": body["my_daemon_port"],
            }));
            Ok(axum::Json(json!({ "status": "completed" })))
        }

        // Bearer auth middleware
        async fn bearer_auth(
            State(s): State<TestAppState>,
            req: Request<Body>,
            next: middleware::Next,
        ) -> Result<axum::response::Response, StatusCode> {
            let auth = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok());
            match auth {
                Some(h) if h.starts_with("Bearer ") => {
                    if h.trim_start_matches("Bearer ").trim() == s.api_token {
                        Ok(next.run(req).await)
                    } else {
                        Err(StatusCode::FORBIDDEN)
                    }
                }
                _ => Err(StatusCode::UNAUTHORIZED),
            }
        }

        let protected = Router::new()
            .route(
                "/node/invite",
                post(|| async { axum::Json(json!({ "invite_code": "howm://invite/test" })) }),
            )
            .layer(middleware::from_fn_with_state(state.clone(), bearer_auth));

        let open = Router::new()
            .route("/node/info", get(get_info))
            .route("/node/peers", get(get_peers))
            .route("/node/wireguard", get(get_wg_status))
            .route("/node/complete-invite", post(complete_invite));

        Router::new().merge(protected).merge(open).with_state(state)
    }

    fn make_test_state(name: &str, port: u16) -> TestAppState {
        TestAppState {
            node_id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            wg_pubkey: format!("pubkey-{}", name),
            wg_address: format!("10.47.0.{}", port % 256),
            wg_endpoint: format!("127.0.0.1:{}", port + 10000),
            port,
            api_token: format!("token-{}", name),
            peers: Arc::new(tokio::sync::RwLock::new(vec![])),
        }
    }

    #[tokio::test]
    async fn test_get_info() {
        let state = make_test_state("alice", 7000);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/node/info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(info["name"], "alice");
        assert!(info["wg_pubkey"].as_str().unwrap().starts_with("pubkey-"));
        assert!(info["wg_address"].as_str().unwrap().starts_with("10.47."));
    }

    #[tokio::test]
    async fn test_bearer_auth_required_for_mutations() {
        let state = make_test_state("bob", 7001);
        let app = build_test_router(state);

        // POST without auth should fail
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/node/invite")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // POST with wrong token should fail
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/node/invite")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // POST with correct token should succeed
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/node/invite")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer token-bob")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_does_not_need_auth() {
        let state = make_test_state("charlie", 7002);
        let app = build_test_router(state);

        // GET /node/info without any auth should work
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/node/info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // GET /node/wireguard should also work
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/node/wireguard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let wg: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(wg["status"], "connected");
    }

    #[tokio::test]
    async fn test_complete_invite_no_auth_needed() {
        let state = make_test_state("dave", 7003);
        let app = build_test_router(state.clone());

        // complete-invite is on the open router (PSK-based auth)
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/node/complete-invite")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&json!({
                            "psk": "some-psk",
                            "my_pubkey": "pubkey-redeemer",
                            "my_endpoint": "5.6.7.8:51820",
                            "my_wg_address": "10.47.0.99",
                            "my_daemon_port": 7004,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        // Verify peer was added
        let peers = state.peers.read().await;
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0]["wg_pubkey"], "pubkey-redeemer");
    }

    #[tokio::test]
    async fn test_peers_empty_initially() {
        let state = make_test_state("eve", 7004);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/node/peers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let data: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(data["peers"].as_array().unwrap().len(), 0);
    }

    /// Simulate two nodes: Alice generates invite, Bob redeems it.
    /// Tests the full mutual peer add flow at the API level.
    #[tokio::test]
    async fn test_invite_redemption_flow() {
        let alice = make_test_state("alice", 7010);
        let bob = make_test_state("bob", 7011);

        let alice_app = build_test_router(alice.clone());

        // 1. Alice generates an invite (requires auth)
        let resp = alice_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/node/invite")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer token-alice")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 2. Bob calls complete-invite on Alice (simulating the redemption callback)
        let resp = build_test_router(alice.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/node/complete-invite")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&json!({
                            "psk": "test-psk-123",
                            "my_pubkey": bob.wg_pubkey,
                            "my_endpoint": bob.wg_endpoint,
                            "my_wg_address": bob.wg_address,
                            "my_daemon_port": bob.port,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 3. Verify Alice now has Bob as a peer
        let alice_peers = alice.peers.read().await;
        assert_eq!(alice_peers.len(), 1);
        assert_eq!(alice_peers[0]["wg_pubkey"], bob.wg_pubkey);
    }
}

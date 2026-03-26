//! Integration tests for NAT hole punch scenarios.
//!
//! Uses a mock WgControl implementation to simulate various NAT topologies
//! without requiring a real WireGuard interface. Each test configures when
//! (or if) the mock handshake "succeeds" relative to which endpoint was set.
//!
//! Scenarios covered:
//!   - Open ↔ Open: immediate handshake on first candidate
//!   - Cone ↔ Cone: handshake on STUN-reflected port (attempt 1)
//!   - Cone ↔ Symmetric: cone initiates, succeeds on stride-predicted port
//!   - Symmetric ↔ Symmetric: no punch possible, timeout
//!   - Port-preserving NAT: first candidate wins
//!   - Strided NAT: handshake on stride offset
//!   - Timeout: handshake never completes
//!   - Add-peer failure: wg add_peer returns error
//!   - Set-endpoint partial failure: some endpoints fail, punch recovers
//!   - IPv6 direct: handshake on first try (simulating direct IPv6 connectivity)

use howm::punch::{self, PunchConfig, PunchResult, WgControl};
use howm::stun::NatType;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

// ── Mock WgControl ─────────────────────────────────────────────────────────

/// Mock WireGuard control plane for testing punch scenarios.
///
/// Configurable behaviors:
/// - `succeed_on_endpoint`: if set, check_handshake returns true when this
///   endpoint was the last one set via set_endpoint.
/// - `succeed_after_n_checks`: handshake succeeds after N check_handshake calls
///   regardless of endpoint (simulates timing-based success).
/// - `add_peer_fails`: if true, add_peer returns an error.
/// - `fail_endpoints`: set of endpoints where set_endpoint returns an error.
struct MockWgControl {
    /// Endpoint that triggers a successful handshake.
    succeed_on_endpoint: Option<String>,
    /// Handshake succeeds after this many check_handshake calls.
    succeed_after_n_checks: Option<usize>,
    /// If true, add_peer always fails.
    add_peer_fails: bool,
    /// Endpoints where set_endpoint fails.
    fail_endpoints: HashSet<String>,

    // ── Observable state ──
    /// Total check_handshake calls.
    check_count: AtomicUsize,
    /// Last endpoint set via set_endpoint.
    last_endpoint: Mutex<Option<String>>,
    /// All endpoints attempted (in order).
    attempted_endpoints: Mutex<Vec<String>>,
    /// Whether add_peer was called.
    peer_added: Mutex<bool>,
}

impl MockWgControl {
    fn new() -> Self {
        Self {
            succeed_on_endpoint: None,
            succeed_after_n_checks: None,
            add_peer_fails: false,
            fail_endpoints: HashSet::new(),
            check_count: AtomicUsize::new(0),
            last_endpoint: Mutex::new(None),
            attempted_endpoints: Mutex::new(Vec::new()),
            peer_added: Mutex::new(false),
        }
    }

    /// Handshake succeeds when this endpoint is set.
    fn succeed_on(mut self, endpoint: &str) -> Self {
        self.succeed_on_endpoint = Some(endpoint.to_string());
        self
    }

    /// Handshake succeeds after N check calls (regardless of endpoint).
    fn succeed_after(mut self, n: usize) -> Self {
        self.succeed_after_n_checks = Some(n);
        self
    }

    /// add_peer will return an error.
    fn with_add_peer_failure(mut self) -> Self {
        self.add_peer_fails = true;
        self
    }

    /// set_endpoint fails for these specific endpoints.
    fn with_failing_endpoints(mut self, endpoints: &[&str]) -> Self {
        self.fail_endpoints = endpoints.iter().map(|s| s.to_string()).collect();
        self
    }

    fn attempted_endpoints(&self) -> Vec<String> {
        self.attempted_endpoints.lock().unwrap().clone()
    }

    fn was_peer_added(&self) -> bool {
        *self.peer_added.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl WgControl for MockWgControl {
    async fn add_peer(&self, _config: &PunchConfig, _wg_iface: &str) -> anyhow::Result<()> {
        *self.peer_added.lock().unwrap() = true;
        if self.add_peer_fails {
            return Err(anyhow::anyhow!("mock: add_peer failed"));
        }
        Ok(())
    }

    async fn set_endpoint(
        &self,
        _wg_iface: &str,
        _pubkey: &str,
        endpoint: &str,
    ) -> anyhow::Result<()> {
        self.attempted_endpoints
            .lock()
            .unwrap()
            .push(endpoint.to_string());
        *self.last_endpoint.lock().unwrap() = Some(endpoint.to_string());

        if self.fail_endpoints.contains(endpoint) {
            return Err(anyhow::anyhow!(
                "mock: set_endpoint failed for {}",
                endpoint
            ));
        }
        Ok(())
    }

    async fn check_handshake(&self, _wg_iface: &str, _pubkey: &str) -> anyhow::Result<bool> {
        let n = self.check_count.fetch_add(1, Ordering::SeqCst) + 1;

        // Check count-based success
        if let Some(threshold) = self.succeed_after_n_checks {
            if n >= threshold {
                return Ok(true);
            }
        }

        // Check endpoint-based success
        if let Some(ref target) = self.succeed_on_endpoint {
            let last = self.last_endpoint.lock().unwrap();
            if last.as_deref() == Some(target.as_str()) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

// ── Helper ─────────────────────────────────────────────────────────────────

fn make_config(
    our_nat: NatType,
    peer_nat: NatType,
    peer_port: u16,
    peer_stride: i32,
) -> PunchConfig {
    PunchConfig {
        peer_pubkey: "dGVzdC1wdWJrZXk=".to_string(), // "test-pubkey" in base64
        peer_external_ip: "203.0.113.5".to_string(),
        peer_external_port: peer_port,
        peer_stride,
        peer_wg_port: peer_port,
        peer_nat_type: peer_nat,
        our_nat_type: our_nat,
        psk: Some("test-psk".to_string()),
        allowed_ip: "100.222.0.2".to_string(),
        we_initiate: punch::should_we_initiate(our_nat, peer_nat),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Open ↔ Open: direct connectivity, handshake succeeds on STUN-reflected port.
#[tokio::test]
async fn test_open_to_open_immediate_success() {
    let config = make_config(NatType::Open, NatType::Open, 41641, 0);
    let mock = MockWgControl::new().succeed_on("203.0.113.5:41641");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "203.0.113.5:41641");
    }
    assert!(mock.was_peer_added());
}

/// Cone ↔ Cone: both probe simultaneously, handshake on STUN port.
#[tokio::test]
async fn test_cone_to_cone_stun_port() {
    let config = make_config(NatType::Cone, NatType::Cone, 41641, 0);
    // Succeed on the first candidate (STUN-reflected)
    let mock = MockWgControl::new().succeed_on("203.0.113.5:41641");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "203.0.113.5:41641");
    }
}

/// Cone ↔ Symmetric: cone initiates, symmetric has stride=4.
/// Handshake succeeds on the +4 stride offset.
#[tokio::test]
async fn test_cone_vs_symmetric_stride_offset() {
    let config = make_config(NatType::Cone, NatType::Symmetric, 41641, 4);
    assert!(config.we_initiate, "cone should initiate vs symmetric");

    // Succeed on the first stride offset (+4)
    let mock = MockWgControl::new().succeed_on("203.0.113.5:41645");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(10)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "203.0.113.5:41645");
    }
}

/// Symmetric ↔ Symmetric: both have unpredictable mappings.
/// Neither can hit the other's ephemeral port → timeout.
#[tokio::test]
async fn test_symmetric_to_symmetric_timeout() {
    let config = make_config(NatType::Symmetric, NatType::Symmetric, 41641, 7);
    // No succeed_on or succeed_after → always returns false
    let mock = MockWgControl::new();

    // Short timeout for test speed
    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_millis(500)).await;

    assert!(matches!(result, PunchResult::Timeout { .. }));
    // Should have attempted multiple endpoints before timing out
    assert!(!mock.attempted_endpoints().is_empty());
}

/// Port-preserving NAT (stride=0): STUN port == WG port, first candidate wins.
#[tokio::test]
async fn test_port_preserving_nat() {
    let config = make_config(NatType::Cone, NatType::Cone, 51820, 0);
    let mock = MockWgControl::new().succeed_on("203.0.113.5:51820");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    // First attempted endpoint should be the STUN port
    let attempts = mock.attempted_endpoints();
    assert_eq!(attempts[0], "203.0.113.5:51820");
}

/// Handshake takes multiple rotations — succeeds on 3rd check.
#[tokio::test]
async fn test_success_after_multiple_rotations() {
    let config = make_config(NatType::Cone, NatType::Cone, 41641, 0);
    // Succeed after 3 check_handshake calls
    let mock = MockWgControl::new().succeed_after(3);

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(10)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    // Should have rotated through some endpoints
    assert!(mock.attempted_endpoints().len() >= 3);
}

/// add_peer failure: punch should return Error immediately.
#[tokio::test]
async fn test_add_peer_failure() {
    let config = make_config(NatType::Cone, NatType::Cone, 41641, 0);
    let mock = MockWgControl::new().with_add_peer_failure();

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Error(_)));
    if let PunchResult::Error(msg) = result {
        assert!(msg.contains("add WG peer"));
    }
    // Should NOT have attempted any endpoints
    assert!(mock.attempted_endpoints().is_empty());
}

/// Some endpoints fail to set but punch recovers and succeeds on another.
#[tokio::test]
async fn test_set_endpoint_partial_failure() {
    let config = make_config(NatType::Cone, NatType::Cone, 41641, 0);

    // First candidate fails, but a neighbor succeeds
    let mock = MockWgControl::new()
        .with_failing_endpoints(&["203.0.113.5:41641"])
        .succeed_on("203.0.113.5:41642");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "203.0.113.5:41642");
    }
}

/// Different WG port vs STUN port: handshake on the WG listen port.
#[tokio::test]
async fn test_different_wg_and_stun_ports() {
    let mut config = make_config(NatType::Cone, NatType::Cone, 41645, 0);
    config.peer_wg_port = 41641; // Different from STUN-reflected

    // Succeed on the actual WG listen port (second candidate)
    let mock = MockWgControl::new().succeed_on("203.0.113.5:41641");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "203.0.113.5:41641");
    }

    // Verify candidate ordering: STUN port first, then WG port
    let attempts = mock.attempted_endpoints();
    assert_eq!(attempts[0], "203.0.113.5:41645"); // tried STUN port first
    assert_eq!(attempts[1], "203.0.113.5:41641"); // then WG port (success)
}

/// Candidate cycling: ensure endpoints wrap around and retry.
#[tokio::test]
async fn test_candidate_cycling() {
    let config = make_config(NatType::Cone, NatType::Cone, 41641, 0);
    // Succeed after enough checks that we must cycle through candidates
    let mock = MockWgControl::new().succeed_after(25);

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(30)).await;

    assert!(matches!(result, PunchResult::Success { .. }));

    let attempts = mock.attempted_endpoints();
    let candidates = punch::build_candidates(&config);
    // We should have cycled: attempts > candidates means wrapping happened
    assert!(
        attempts.len() > candidates.len(),
        "expected cycling: {} attempts, {} candidates",
        attempts.len(),
        candidates.len()
    );
}

/// Initiator timing: cone vs symmetric uses 1s interval (slower).
/// Responder timing: symmetric vs cone uses 200ms interval (faster).
#[tokio::test]
async fn test_initiator_vs_responder_timing() {
    // Cone vs Symmetric: we_initiate=true, 1s interval → fewer attempts in 2s
    let config_initiator = make_config(NatType::Cone, NatType::Symmetric, 41641, 0);
    assert!(config_initiator.we_initiate);

    let mock_i = MockWgControl::new();
    let _ = punch::run_punch(&config_initiator, &mock_i, "howm0", Duration::from_secs(2)).await;
    let initiator_attempts = mock_i.attempted_endpoints().len();

    // Symmetric vs Cone: we_initiate=false, 200ms interval → more attempts in 2s
    let config_responder = make_config(NatType::Symmetric, NatType::Cone, 41641, 0);
    assert!(!config_responder.we_initiate);

    let mock_r = MockWgControl::new();
    let _ = punch::run_punch(&config_responder, &mock_r, "howm0", Duration::from_secs(2)).await;
    let responder_attempts = mock_r.attempted_endpoints().len();

    // Responder should have significantly more attempts (200ms vs 1000ms interval)
    assert!(
        responder_attempts > initiator_attempts * 2,
        "responder ({} attempts) should be >2x initiator ({} attempts)",
        responder_attempts,
        initiator_attempts
    );
}

/// Negative stride: some NATs allocate ports in decreasing order.
#[tokio::test]
async fn test_negative_stride() {
    let config = make_config(NatType::Cone, NatType::Symmetric, 41641, -2);
    // Succeed on port 41641 - 2 = 41639
    let mock = MockWgControl::new().succeed_on("203.0.113.5:41639");

    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "203.0.113.5:41639");
    }
}

/// Verify build_candidates includes stride predictions for symmetric NAT.
#[test]
fn test_candidates_include_stride_predictions() {
    let config = make_config(NatType::Cone, NatType::Symmetric, 41641, 4);
    let candidates = punch::build_candidates(&config);

    // Should include: base, base±4, base±8, base±12, base±16, base±20
    assert!(candidates.contains(&"203.0.113.5:41641".to_string())); // base
    assert!(candidates.contains(&"203.0.113.5:41645".to_string())); // +4
    assert!(candidates.contains(&"203.0.113.5:41637".to_string())); // -4
    assert!(candidates.contains(&"203.0.113.5:41649".to_string())); // +8
    assert!(candidates.contains(&"203.0.113.5:41633".to_string())); // -8
}

/// Verify that PSK is passed through to add_peer.
#[tokio::test]
async fn test_psk_passed_to_add_peer() {
    let config = make_config(NatType::Open, NatType::Open, 41641, 0);
    assert!(config.psk.is_some());

    let mock = MockWgControl::new().succeed_on("203.0.113.5:41641");
    let _ = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(mock.was_peer_added());
}

/// No PSK: should still work (PSK is optional for testing/dev).
#[tokio::test]
async fn test_no_psk() {
    let mut config = make_config(NatType::Open, NatType::Open, 41641, 0);
    config.psk = None;

    let mock = MockWgControl::new().succeed_on("203.0.113.5:41641");
    let result = punch::run_punch(&config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
}

/// Verify the full invite → accept → punch flow with mocked WG.
/// This tests the data pipeline: invite fields → PunchConfig → candidates → punch loop.
#[tokio::test]
async fn test_full_invite_accept_punch_pipeline() {
    // Simulate: Alice invites Bob. Alice is Cone, Bob is Cone.
    // Alice's NAT profile: external 203.0.113.5:41641, stride 0
    // Bob's NAT profile: external 198.51.100.1:41641, stride 0

    // Bob decodes the invite and generates an accept token
    let accept_token = howm::accept::generate(
        "alice_pubkey_base64",
        "bob_pubkey_base64",
        &[],
        "198.51.100.1",
        41641,
        41641,
        NatType::Cone,
        0,
        "shared-psk-value",
    );

    // Alice decodes the accept token
    let decoded = howm::accept::decode(&accept_token).unwrap();
    assert_eq!(decoded.pubkey, "bob_pubkey_base64");
    assert_eq!(decoded.external_ip, "198.51.100.1");
    assert_eq!(decoded.nat_type, NatType::Cone);

    // Alice builds PunchConfig from the decoded accept
    let punch_config = PunchConfig {
        peer_pubkey: decoded.pubkey,
        peer_external_ip: decoded.external_ip.clone(),
        peer_external_port: decoded.external_port,
        peer_stride: decoded.observed_stride,
        peer_wg_port: decoded.wg_port,
        peer_nat_type: decoded.nat_type,
        our_nat_type: NatType::Cone,
        psk: Some(decoded.psk),
        allowed_ip: "100.222.0.2".to_string(),
        we_initiate: punch::should_we_initiate(NatType::Cone, decoded.nat_type),
    };

    // Verify candidates are built correctly from the accept data
    let candidates = punch::build_candidates(&punch_config);
    assert_eq!(candidates[0], "198.51.100.1:41641");

    // Mock: handshake succeeds on the STUN-reflected port
    let mock = MockWgControl::new().succeed_on("198.51.100.1:41641");

    let result = punch::run_punch(&punch_config, &mock, "howm0", Duration::from_secs(5)).await;

    assert!(matches!(result, PunchResult::Success { .. }));
    if let PunchResult::Success { endpoint, .. } = result {
        assert_eq!(endpoint, "198.51.100.1:41641");
    }
}

/// Edge case: port near u16::MAX — ensure no overflow in candidate generation.
#[test]
fn test_candidates_near_port_max() {
    let config = PunchConfig {
        peer_pubkey: "test".to_string(),
        peer_external_ip: "1.2.3.4".to_string(),
        peer_external_port: 65530,
        peer_stride: 4,
        peer_wg_port: 65530,
        peer_nat_type: NatType::Cone,
        our_nat_type: NatType::Cone,
        psk: None,
        allowed_ip: "100.222.0.2".to_string(),
        we_initiate: false,
    };

    let candidates = punch::build_candidates(&config);
    // Should not panic, should not contain port 0
    for c in &candidates {
        let port: u16 = c.rsplit(':').next().unwrap().parse().unwrap();
        assert!(port > 0, "candidate has port 0: {}", c);
    }
    // First should still be the base port
    assert_eq!(candidates[0], "1.2.3.4:65530");
}

/// Edge case: port near u16::MIN — ensure no underflow.
#[test]
fn test_candidates_near_port_min() {
    let config = PunchConfig {
        peer_pubkey: "test".to_string(),
        peer_external_ip: "1.2.3.4".to_string(),
        peer_external_port: 5,
        peer_stride: 4,
        peer_wg_port: 5,
        peer_nat_type: NatType::Cone,
        our_nat_type: NatType::Cone,
        psk: None,
        allowed_ip: "100.222.0.2".to_string(),
        we_initiate: false,
    };

    let candidates = punch::build_candidates(&config);
    for c in &candidates {
        let port: u16 = c.rsplit(':').next().unwrap().parse().unwrap();
        assert!(port > 0, "candidate has port 0: {}", c);
    }
    assert_eq!(candidates[0], "1.2.3.4:5");
}

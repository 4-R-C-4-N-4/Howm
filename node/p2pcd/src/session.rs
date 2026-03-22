// P2P-CD session state machine — Tasks 2.2 + 2.3
//
// State transitions per spec §6:
//
//   PEER_VISIBLE → HANDSHAKE → CAPABILITY_EXCHANGE → ACTIVE
//                                                   → NONE (no matching caps)
//                                                   → DENIED (trust gate)
//   ACTIVE / NONE / DENIED → CLOSED
//   CLOSED (persists until next PEER_VISIBLE)

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use p2pcd_types::{
    compute_intersection, CloseReason, DiscoveryManifest, PeerId, ProtocolMessage, ScopeParams,
    TrustPolicy,
};

use crate::transport::P2pcdTransport;

// ── State enum ───────────────────────────────────────────────────────────────

/// All states a P2P-CD session can be in per spec §6.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SessionState {
    /// WireGuard handshake detected; no TCP connection yet.
    PeerVisible,
    /// TCP connection established; identity being confirmed.
    Handshake,
    /// OFFER messages being exchanged.
    CapabilityExchange,
    /// CONFIRM reconciliation complete; capabilities agreed.
    Active,
    /// CONFIRM exchange completed; no matching capabilities.
    None,
    /// Trust gate blocked negotiation for all capabilities.
    Denied,
    /// Session terminated (normal, timeout, or error).
    Closed { reason: CloseReason },
}

impl SessionState {
    /// Returns true if this is a terminal state.
    #[allow(dead_code)]
    pub fn is_terminal(&self) -> bool {
        matches!(self, SessionState::Closed { .. })
    }

    /// Returns true if this is a state where an active TCP transport exists.
    #[allow(dead_code)]
    pub fn has_transport(&self) -> bool {
        matches!(
            self,
            SessionState::Handshake
                | SessionState::CapabilityExchange
                | SessionState::Active
                | SessionState::None
                | SessionState::Denied
        )
    }

    /// Validate that a transition from `self` to `next` is legal per spec §6.
    pub fn can_transition_to(&self, next: &SessionState) -> bool {
        use SessionState::*;
        matches!(
            (self, next),
            (PeerVisible,         Handshake)
            | (PeerVisible,       Closed { .. })   // peer vanished before TCP
            | (Handshake,         CapabilityExchange)
            | (Handshake,         Closed { .. })   // TCP failed
            | (CapabilityExchange, Active)
            | (CapabilityExchange, None)
            | (CapabilityExchange, Denied)
            | (CapabilityExchange, Closed { .. })
            | (Active,            Closed { .. })
            | (None,              Closed { .. })
            | (Denied,            Closed { .. })
            // Re-open after close (next WG handshake)
            | (Closed { .. },     PeerVisible)
        )
    }
}

// ── Session struct ───────────────────────────────────────────────────────────

/// A P2P-CD session with one remote peer.
pub struct Session {
    pub remote_peer_id: PeerId,
    pub state: SessionState,
    pub transport: Option<P2pcdTransport>,
    pub local_manifest: DiscoveryManifest,
    pub remote_manifest: Option<DiscoveryManifest>,
    /// Agreed capabilities after CONFIRM reconciliation.
    pub active_set: Vec<String>,
    /// Reconciled scope params per capability.
    pub accepted_params: BTreeMap<String, ScopeParams>,
    pub created_at: u64,
    pub last_activity: u64,
}

impl Session {
    pub fn new(remote_peer_id: PeerId, local_manifest: DiscoveryManifest) -> Self {
        let now = unix_now();
        Self {
            remote_peer_id,
            state: SessionState::PeerVisible,
            transport: None,
            local_manifest,
            remote_manifest: Option::None,
            active_set: vec![],
            accepted_params: BTreeMap::new(),
            created_at: now,
            last_activity: now,
        }
    }

    /// Attempt a state transition, returning Err if the transition is illegal.
    pub fn transition(&mut self, next: SessionState) -> Result<()> {
        if !self.state.can_transition_to(&next) {
            bail!(
                "illegal session transition {:?} → {:?} for peer {}",
                self.state,
                next,
                peer_short(&self.remote_peer_id)
            );
        }
        tracing::info!(
            "session {}: {:?} → {:?}",
            peer_short(&self.remote_peer_id),
            self.state,
            next
        );
        self.state = next;
        self.touch();
        Ok(())
    }

    fn touch(&mut self) {
        self.last_activity = unix_now();
    }
}

// ── OFFER/CONFIRM exchange (Task 2.3) ────────────────────────────────────────

/// Run the OFFER/CONFIRM exchange as the **initiator** (we connected outbound).
///
/// Protocol:
///   1. Send our OFFER (local manifest)
///   2. Receive remote OFFER
///   3. Compute intersection + trust gates → our CONFIRM
///   4. Send our CONFIRM
///   5. Receive remote CONFIRM
///   6. Reconcile final active_set
pub async fn run_initiator_exchange(
    session: &mut Session,
    trust_policies: &std::collections::HashMap<String, TrustPolicy>,
) -> Result<()> {
    // Transition state (no transport borrow active yet)
    session.transition(SessionState::Handshake)?;
    session.transition(SessionState::CapabilityExchange)?;

    let local_manifest = session.local_manifest.clone();
    let peer_id = session.remote_peer_id;

    // Borrow transport for the I/O phase
    let (remote_manifest, final_outcome) = {
        let transport = session
            .transport
            .as_mut()
            .context("no transport in initiator exchange")?;

        // Step 1: send our OFFER
        transport
            .send(&ProtocolMessage::Offer {
                manifest: local_manifest.clone(),
            })
            .await
            .context("send OFFER")?;

        // Step 2: receive remote OFFER
        let remote_manifest = recv_offer(transport).await?;

        // Step 3: compute intersection
        let our_active_set =
            compute_intersection(&local_manifest, &remote_manifest, trust_policies);
        let our_params = reconcile_params(&our_active_set, &local_manifest, &remote_manifest);

        // Step 4: send our CONFIRM
        transport
            .send(&ProtocolMessage::Confirm {
                personal_hash: local_manifest.personal_hash.clone(),
                active_set: our_active_set.clone(),
                accepted_params: if our_params.is_empty() {
                    Option::None
                } else {
                    Some(our_params.clone())
                },
            })
            .await
            .context("send CONFIRM")?;

        // Step 5: receive remote CONFIRM (or CLOSE)
        let outcome = match transport.recv().await.context("recv CONFIRM/CLOSE")? {
            ProtocolMessage::Confirm {
                active_set: remote_set,
                accepted_params: remote_p,
                ..
            } => {
                let final_set = intersect_sets(&our_active_set, &remote_set);
                let final_params = reconcile_confirm_params(
                    &final_set,
                    &our_params,
                    &remote_p.unwrap_or_default(),
                );
                Ok((final_set, final_params))
            }
            ProtocolMessage::Close { reason, .. } => {
                tracing::info!(
                    "session {}: peer CLOSE({:?}) during exchange",
                    peer_short(&peer_id),
                    reason
                );
                Err(reason)
            }
            other => bail!(
                "unexpected during CONFIRM wait: {:?}",
                std::mem::discriminant(&other)
            ),
        };
        (remote_manifest, outcome)
    };

    // Now we can mutate session freely again
    session.remote_manifest = Some(remote_manifest);
    match final_outcome {
        Ok((final_set, final_params)) => finalize_session(session, final_set, final_params),
        Err(reason) => session.transition(SessionState::Closed { reason }),
    }
}

/// Run the OFFER/CONFIRM exchange as the **responder** (we accepted inbound).
///
/// Protocol:
///   1. Receive remote OFFER
///   2. Send our OFFER
///   3. Receive remote CONFIRM (or CLOSE)
///   4. Compute intersection + trust gates → our CONFIRM
///   5. Send our CONFIRM
///   6. Reconcile final active_set
pub async fn run_responder_exchange(
    session: &mut Session,
    trust_policies: &std::collections::HashMap<String, TrustPolicy>,
) -> Result<()> {
    session.transition(SessionState::Handshake)?;
    session.transition(SessionState::CapabilityExchange)?;

    let local_manifest = session.local_manifest.clone();
    let peer_id = session.remote_peer_id;

    // Borrow transport for the I/O phase
    let (remote_manifest, final_outcome) = {
        let transport = session
            .transport
            .as_mut()
            .context("no transport in responder exchange")?;

        // Step 1: receive remote OFFER
        let remote_manifest = recv_offer(transport).await?;

        // Step 2: send our OFFER
        transport
            .send(&ProtocolMessage::Offer {
                manifest: local_manifest.clone(),
            })
            .await
            .context("send OFFER")?;

        // Step 3: receive remote CONFIRM (or CLOSE)
        let (remote_active_set, remote_params) =
            match transport.recv().await.context("recv CONFIRM/CLOSE")? {
                ProtocolMessage::Confirm {
                    active_set,
                    accepted_params,
                    ..
                } => (active_set, accepted_params.unwrap_or_default()),
                ProtocolMessage::Close { reason, .. } => {
                    tracing::info!(
                        "session {}: peer CLOSE({:?}) before our CONFIRM",
                        peer_short(&peer_id),
                        reason
                    );
                    return {
                        let _ = transport; // end borrow before mutating session
                        session.remote_manifest = Some(remote_manifest);
                        session.transition(SessionState::Closed { reason })
                    };
                }
                other => bail!(
                    "unexpected during responder CONFIRM wait: {:?}",
                    std::mem::discriminant(&other)
                ),
            };

        // Step 4: compute our intersection
        let our_active_set =
            compute_intersection(&local_manifest, &remote_manifest, trust_policies);
        let our_params = reconcile_params(&our_active_set, &local_manifest, &remote_manifest);

        // Step 5: send our CONFIRM
        transport
            .send(&ProtocolMessage::Confirm {
                personal_hash: local_manifest.personal_hash.clone(),
                active_set: our_active_set.clone(),
                accepted_params: if our_params.is_empty() {
                    Option::None
                } else {
                    Some(our_params.clone())
                },
            })
            .await
            .context("send CONFIRM")?;

        // Step 6: reconcile
        let final_set = intersect_sets(&our_active_set, &remote_active_set);
        let final_params = reconcile_confirm_params(&final_set, &our_params, &remote_params);
        (remote_manifest, Ok((final_set, final_params)))
    };

    session.remote_manifest = Some(remote_manifest);
    match final_outcome {
        Ok((final_set, final_params)) => finalize_session(session, final_set, final_params),
        Err(reason) => session.transition(SessionState::Closed { reason }),
    }
}

/// Send a CLOSE message and transition to Closed.
pub async fn send_close(session: &mut Session, reason: CloseReason) -> Result<()> {
    if let Some(transport) = session.transport.as_mut() {
        let msg = ProtocolMessage::Close {
            personal_hash: session.local_manifest.personal_hash.clone(),
            reason,
        };
        let _ = transport.send(&msg).await; // best-effort
    }
    session.transition(SessionState::Closed { reason })?;
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn recv_offer(transport: &mut P2pcdTransport) -> Result<DiscoveryManifest> {
    match transport.recv().await.context("recv OFFER")? {
        ProtocolMessage::Offer { manifest } => Ok(manifest),
        ProtocolMessage::Close { reason, .. } => {
            bail!("peer sent CLOSE({:?}) instead of OFFER", reason)
        }
        other => bail!("expected OFFER, got {:?}", std::mem::discriminant(&other)),
    }
}

/// Compute reconciled scope params from two manifests for the given active_set.
fn reconcile_params(
    active_set: &[String],
    local: &DiscoveryManifest,
    remote: &DiscoveryManifest,
) -> BTreeMap<String, ScopeParams> {
    let mut result = BTreeMap::new();
    for cap_name in active_set {
        let local_scope = local
            .capabilities
            .iter()
            .find(|c| &c.name == cap_name)
            .and_then(|c| c.scope.clone())
            .unwrap_or_default();
        let remote_scope = remote
            .capabilities
            .iter()
            .find(|c| &c.name == cap_name)
            .and_then(|c| c.scope.clone())
            .unwrap_or_default();
        result.insert(cap_name.clone(), local_scope.reconcile(&remote_scope));
    }
    result
}

/// Reconcile params from two CONFIRM messages for the final active_set.
fn reconcile_confirm_params(
    final_set: &[String],
    our_params: &BTreeMap<String, ScopeParams>,
    their_params: &BTreeMap<String, ScopeParams>,
) -> BTreeMap<String, ScopeParams> {
    let mut result = BTreeMap::new();
    for cap_name in final_set {
        let ours = our_params.get(cap_name).cloned().unwrap_or_default();
        let theirs = their_params.get(cap_name).cloned().unwrap_or_default();
        result.insert(cap_name.clone(), ours.reconcile(&theirs));
    }
    result
}

/// Intersection of two sorted capability-name lists.
fn intersect_sets(a: &[String], b: &[String]) -> Vec<String> {
    let b_set: std::collections::HashSet<&String> = b.iter().collect();
    let mut result: Vec<String> = a.iter().filter(|n| b_set.contains(n)).cloned().collect();
    result.sort();
    result
}

/// Apply final active_set + params to the session and transition to the
/// appropriate end-state.
fn finalize_session(
    session: &mut Session,
    final_set: Vec<String>,
    final_params: BTreeMap<String, ScopeParams>,
) -> Result<()> {
    session.active_set = final_set.clone();
    session.accepted_params = final_params;

    if final_set.is_empty() {
        session.transition(SessionState::None)
    } else {
        session.transition(SessionState::Active)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn peer_short(id: &PeerId) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(&id[..4])
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{connect, P2pcdListener};
    use p2pcd_types::{
        CapabilityDeclaration, CloseReason, DiscoveryManifest, Role, ScopeParams, PROTOCOL_VERSION,
    };
    use std::collections::HashMap;

    fn make_manifest(id: u8, caps: Vec<CapabilityDeclaration>) -> DiscoveryManifest {
        DiscoveryManifest {
            protocol_version: PROTOCOL_VERSION,
            peer_id: [id; 32],
            sequence_num: 1,
            capabilities: caps,
            personal_hash: vec![id; 32],
            hash_algorithm: "sha-256".to_string(),
        }
    }

    fn social_cap(role: Role) -> CapabilityDeclaration {
        CapabilityDeclaration {
            name: "howm.social.feed.1".to_string(),
            role,
            mutual: false,
            scope: Some(ScopeParams {
                rate_limit: 100,
                ttl: 3600,
                ..Default::default()
            }),
            applicable_scope_keys: None,
        }
    }

    fn heartbeat_cap() -> CapabilityDeclaration {
        CapabilityDeclaration {
            name: "core.session.heartbeat.1".to_string(),
            role: Role::Both,
            mutual: true,
            scope: Option::None,
            applicable_scope_keys: None,
        }
    }

    // ── State machine unit tests ─────────────────────────────────────────────

    #[test]
    fn legal_transitions() {
        let mut s = Session::new([1u8; 32], make_manifest(1, vec![]));
        assert_eq!(s.state, SessionState::PeerVisible);

        s.transition(SessionState::Handshake).unwrap();
        s.transition(SessionState::CapabilityExchange).unwrap();
        s.transition(SessionState::Active).unwrap();
        s.transition(SessionState::Closed {
            reason: CloseReason::Normal,
        })
        .unwrap();
        // Re-open after close
        s.transition(SessionState::PeerVisible).unwrap();
    }

    #[test]
    fn illegal_transition_rejected() {
        let mut s = Session::new([2u8; 32], make_manifest(2, vec![]));
        // Can't jump from PeerVisible straight to Active
        let result = s.transition(SessionState::Active);
        assert!(result.is_err());
    }

    #[test]
    fn none_state_on_empty_active_set() {
        let manifest = make_manifest(3, vec![]);
        let mut session = Session::new([3u8; 32], manifest.clone());
        session.transition(SessionState::Handshake).unwrap();
        session
            .transition(SessionState::CapabilityExchange)
            .unwrap();
        // Empty active_set → None
        finalize_session(&mut session, vec![], BTreeMap::new()).unwrap();
        assert_eq!(session.state, SessionState::None);
    }

    #[test]
    fn active_state_with_caps() {
        let manifest = make_manifest(4, vec![social_cap(Role::Provide)]);
        let mut session = Session::new([4u8; 32], manifest.clone());
        session.transition(SessionState::Handshake).unwrap();
        session
            .transition(SessionState::CapabilityExchange)
            .unwrap();
        finalize_session(
            &mut session,
            vec!["p2pcd.social.post.1".to_string()],
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(session.state, SessionState::Active);
        assert_eq!(session.active_set, vec!["p2pcd.social.post.1"]);
    }

    #[test]
    fn scope_reconcile_most_restrictive() {
        let a = ScopeParams {
            rate_limit: 100,
            ttl: 3600,
            ..Default::default()
        };
        let b = ScopeParams {
            rate_limit: 50,
            ttl: 7200,
            ..Default::default()
        };
        let r = a.reconcile(&b);
        assert_eq!(r.rate_limit, 50);
        assert_eq!(r.ttl, 3600);
    }

    // ── OFFER/CONFIRM integration tests ──────────────────────────────────────

    /// Two nodes, both with social.post (Provide + Consume) — should reach ACTIVE.
    #[tokio::test]
    async fn normal_normal_full_exchange() {
        let local_manifest = make_manifest(1, vec![social_cap(Role::Provide), heartbeat_cap()]);
        let remote_manifest = make_manifest(2, vec![social_cap(Role::Consume), heartbeat_cap()]);

        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        // Responder side — has remote_manifest as local, local_manifest as remote
        let rm = remote_manifest.clone();
        let lm = local_manifest.clone();
        let responder_task = tokio::spawn(async move {
            let (transport, _) = listener.accept().await.unwrap();
            let mut session = Session::new([1u8; 32], rm);
            session.transport = Some(transport);
            run_responder_exchange(&mut session, &HashMap::new())
                .await
                .unwrap();
            session.state.clone()
        });

        // Initiator side
        let transport = connect(addr).await.unwrap();
        let mut session = Session::new([2u8; 32], lm);
        session.transport = Some(transport);
        run_initiator_exchange(&mut session, &HashMap::new())
            .await
            .unwrap();

        let initiator_state = session.state.clone();
        let responder_state = responder_task.await.unwrap();

        assert_eq!(
            initiator_state,
            SessionState::Active,
            "initiator should be ACTIVE"
        );
        assert_eq!(
            responder_state,
            SessionState::Active,
            "responder should be ACTIVE"
        );

        assert!(
            session
                .active_set
                .contains(&"howm.social.feed.1".to_string()),
            "howm.social.feed.1 should be in active_set, got {:?}",
            session.active_set
        );
    }

    /// Two lurkers (both Role::Both but mutual=false) — no social match, only heartbeat.
    #[tokio::test]
    async fn lurker_lurker_no_social() {
        // Both have Role::Both but mutual=false for social → no match
let lurker_cap = CapabilityDeclaration {
            name: "howm.social.feed.1".to_string(),
            role: Role::Consume,
            mutual: false,
            scope: None,
            applicable_scope_keys: None,
        };

        let local_manifest = make_manifest(3, vec![lurker_cap.clone(), heartbeat_cap()]);
        let remote_manifest = make_manifest(4, vec![lurker_cap.clone(), heartbeat_cap()]);

        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let rm = remote_manifest.clone();
        let responder_task = tokio::spawn(async move {
            let (transport, _) = listener.accept().await.unwrap();
            let mut session = Session::new([3u8; 32], rm);
            session.transport = Some(transport);
            run_responder_exchange(&mut session, &HashMap::new())
                .await
                .unwrap();
            (session.state.clone(), session.active_set.clone())
        });

        let transport = connect(addr).await.unwrap();
        let mut session = Session::new([4u8; 32], local_manifest);
        session.transport = Some(transport);
        run_initiator_exchange(&mut session, &HashMap::new())
            .await
            .unwrap();

        let (resp_state, resp_set) = responder_task.await.unwrap();

        // social: Both+Both mutual=false → no match
        // heartbeat: Both+Both mutual=true → match
        assert!(
            !session
                .active_set
                .contains(&"p2pcd.social.post.1".to_string()),
            "lurker+lurker social should NOT match"
        );
        assert!(
            session
                .active_set
                .contains(&"core.session.heartbeat.1".to_string()),
            "heartbeat should match, got {:?}",
            session.active_set
        );
        assert_eq!(session.state, SessionState::Active);

        assert!(!resp_set.contains(&"p2pcd.social.post.1".to_string()));
        assert_eq!(resp_state, SessionState::Active);
    }

    /// Nodes with no overlapping capabilities → NONE state.
    #[tokio::test]
    async fn no_match_yields_none() {
        let local_manifest = make_manifest(5, vec![social_cap(Role::Provide)]);
        // remote only has a different cap (no consumer for social, no heartbeat mutual)
let other_cap = CapabilityDeclaration {
            name: "howm.social.feed.1".to_string(),
            role: Role::Provide,
            mutual: false,
            scope: None,
            applicable_scope_keys: None,
        };
        let remote_manifest = make_manifest(6, vec![other_cap]);

        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let rm = remote_manifest.clone();
        let responder_task = tokio::spawn(async move {
            let (transport, _) = listener.accept().await.unwrap();
            let mut session = Session::new([5u8; 32], rm);
            session.transport = Some(transport);
            run_responder_exchange(&mut session, &HashMap::new())
                .await
                .unwrap();
            session.state.clone()
        });

        let transport = connect(addr).await.unwrap();
        let mut session = Session::new([6u8; 32], local_manifest);
        session.transport = Some(transport);
        run_initiator_exchange(&mut session, &HashMap::new())
            .await
            .unwrap();

        let responder_state = responder_task.await.unwrap();
        assert_eq!(
            session.state,
            SessionState::None,
            "initiator should be NONE"
        );
        assert_eq!(
            responder_state,
            SessionState::None,
            "responder should be NONE"
        );
        assert!(session.active_set.is_empty());
    }
}

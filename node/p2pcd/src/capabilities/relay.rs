// core.network.relay.1 — Relay circuits (msg types 13-15)
//
// Enables peers to relay traffic through an intermediary node when direct
// connectivity isn't available. A relay node maintains circuit state mapping
// an initiator peer to a target peer, forwarding CIRCUIT_DATA in both
// directions.
//
// Message flow:
//   Initiator → CIRCUIT_OPEN (circuit_id, target_peer)
//   Relay     → CIRCUIT_OPEN forwarded to target (with initiator info)
//   Target    → CIRCUIT_OPEN response (circuit_id, status) → relay forwards to initiator
//   Either    → CIRCUIT_DATA (circuit_id, data) — forwarded to the other end
//   Either    → CIRCUIT_CLOSE (circuit_id, reason) — forwarded + circuit torn down
//
// Three roles:
//   Relay:     forwards messages between initiator and target (Circuit state)
//   Initiator: opened the circuit, receives data via callback (EndpointCircuit)
//   Target:    accepted the circuit, receives data via callback (EndpointCircuit)
//
// Scope params:
//   RELAY_MAX_CIRCUITS (key 9)      — max concurrent circuits (default 16)
//   RELAY_MAX_BANDWIDTH_KBPS (10)   — max throughput per circuit (default 1024)
//   RELAY_TTL (11)                  — circuit lifetime in seconds (default 300)

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::RwLock;

use p2pcd_types::{
    message_types, scope_keys, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage,
};

use crate::cbor_helpers::{
    cbor_encode_map, cbor_get_bytes, cbor_get_int, decode_payload, make_capability_msg,
};

// ── CBOR payload keys ────────────────────────────────────────────────────────

mod keys {
    pub const CIRCUIT_ID: u64 = 1;
    pub const TARGET_PEER: u64 = 2;
    pub const INITIATOR_PEER: u64 = 3;
    pub const DATA: u64 = 4;
    pub const REASON: u64 = 5;
    pub const STATUS: u64 = 6;
}

/// CIRCUIT_CLOSE reason codes
mod reasons {
    pub const NORMAL: u64 = 0;
    pub const TARGET_UNREACHABLE: u64 = 1;
    pub const CAPACITY_EXCEEDED: u64 = 2;
    pub const TTL_EXPIRED: u64 = 3;
    #[allow(dead_code)]
    pub const ERROR: u64 = 4;
}

/// CIRCUIT_OPEN response status
mod open_status {
    pub const ACCEPTED: u64 = 0;
    #[allow(dead_code)]
    pub const REJECTED: u64 = 1;
}

/// Default max concurrent circuits per peer session.
const DEFAULT_MAX_CIRCUITS: u64 = 16;
/// Default circuit TTL: 5 minutes.
const DEFAULT_TTL_SECS: u64 = 300;

// ── Circuit state (relay-side) ──────────────────────────────────────────────

struct Circuit {
    #[allow(dead_code)]
    circuit_id: u64,
    /// Peer who requested the circuit.
    initiator: PeerId,
    /// Destination peer.
    target: PeerId,
    /// Unix timestamp when circuit was established.
    created_at: u64,
    /// Total bytes forwarded through this circuit.
    bytes_relayed: u64,
    /// TTL in seconds (from scope params at creation time).
    ttl_secs: u64,
}

impl Circuit {
    fn is_expired(&self) -> bool {
        unix_now().saturating_sub(self.created_at) > self.ttl_secs
    }

    /// Given a peer_id, return the peer on the other end of the circuit.
    fn other_end(&self, peer_id: &PeerId) -> Option<PeerId> {
        if *peer_id == self.initiator {
            Some(self.target)
        } else if *peer_id == self.target {
            Some(self.initiator)
        } else {
            None
        }
    }
}

// ── Endpoint circuit state ──────────────────────────────────────────────────

/// Circuit state for when we are an endpoint (initiator or target), not the relay.
struct EndpointCircuit {
    #[allow(dead_code)]
    circuit_id: u64,
    /// The relay peer we talk through.
    relay_peer: PeerId,
    /// The peer on the other end of the circuit.
    remote_peer: PeerId,
    /// Our role in this circuit.
    #[allow(dead_code)]
    role: EndpointRole,
}

/// Our role as a circuit endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRole {
    /// We initiated the circuit.
    Initiator,
    /// We are the target of the circuit.
    Target,
}

/// Events delivered to the host application when we are a circuit endpoint.
#[derive(Debug)]
pub enum CircuitEvent {
    /// Circuit established — we can send/receive data.
    Opened {
        circuit_id: u64,
        remote_peer: PeerId,
        role: EndpointRole,
    },
    /// Data arrived on a circuit we're an endpoint of.
    Data {
        circuit_id: u64,
        from: PeerId,
        data: Vec<u8>,
    },
    /// Circuit closed (by remote, relay, or TTL expiry).
    Closed { circuit_id: u64, reason: u64 },
}

// ── RelayHandler ─────────────────────────────────────────────────────────────

pub struct RelayHandler {
    /// Active circuits we are RELAYING.
    circuits: Arc<RwLock<HashMap<u64, Circuit>>>,
    /// Active circuits we are an ENDPOINT of (initiator or target).
    endpoint_circuits: Arc<RwLock<HashMap<u64, EndpointCircuit>>>,
    /// Circuits we initiated that haven't been accepted yet (circuit_id → target).
    pending_initiations: Arc<RwLock<HashMap<u64, PeerId>>>,
    /// Per-peer send channels. The relay needs to route messages to peers other
    /// than the one that sent the incoming message.
    peer_senders: Arc<RwLock<HashMap<PeerId, tokio::sync::mpsc::Sender<ProtocolMessage>>>>,
    /// Callback channel for circuit events (set by daemon at startup).
    event_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<CircuitEvent>>>>,
    /// Counter for generating unique circuit IDs when initiating.
    next_circuit_id: AtomicU64,
}

impl Default for RelayHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayHandler {
    pub fn new() -> Self {
        Self {
            circuits: Arc::new(RwLock::new(HashMap::new())),
            endpoint_circuits: Arc::new(RwLock::new(HashMap::new())),
            pending_initiations: Arc::new(RwLock::new(HashMap::new())),
            peer_senders: Arc::new(RwLock::new(HashMap::new())),
            event_tx: Arc::new(RwLock::new(None)),
            next_circuit_id: AtomicU64::new(1),
        }
    }

    /// Register a send channel for a connected peer.
    pub async fn add_peer_sender(
        &self,
        peer_id: PeerId,
        tx: tokio::sync::mpsc::Sender<ProtocolMessage>,
    ) {
        self.peer_senders.write().await.insert(peer_id, tx);
    }

    /// Remove a peer's send channel (e.g. on disconnect).
    pub async fn remove_peer_sender(&self, peer_id: &PeerId) {
        self.peer_senders.write().await.remove(peer_id);
    }

    /// Number of active relay circuits.
    pub async fn circuit_count(&self) -> usize {
        self.circuits.read().await.len()
    }

    /// Number of active endpoint circuits.
    pub async fn endpoint_circuit_count(&self) -> usize {
        self.endpoint_circuits.read().await.len()
    }

    /// Register a callback channel for circuit events.
    /// The daemon calls this once at startup.
    pub async fn set_event_callback(&self, tx: tokio::sync::mpsc::Sender<CircuitEvent>) {
        *self.event_tx.write().await = Some(tx);
    }

    /// Initiate a circuit through a relay peer to a target peer.
    /// Returns the circuit_id. The caller should wait for `CircuitEvent::Opened`
    /// on the event channel.
    pub async fn initiate_circuit(&self, relay_peer: &PeerId, target_peer: &PeerId) -> u64 {
        let circuit_id = self.next_circuit_id.fetch_add(1, Ordering::Relaxed);
        self.pending_initiations
            .write()
            .await
            .insert(circuit_id, *target_peer);

        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(circuit_id.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(target_peer.to_vec()),
            ),
        ]);
        self.send_to_peer(relay_peer, message_types::CIRCUIT_OPEN, open)
            .await;

        tracing::info!(
            "relay: initiated circuit {} through relay {} to target {}",
            circuit_id,
            hex::encode(&relay_peer[..4]),
            hex::encode(&target_peer[..4])
        );
        circuit_id
    }

    /// Send data on a circuit we are an endpoint of.
    pub async fn send_circuit_data(&self, circuit_id: u64, data: Vec<u8>) -> Result<()> {
        let ep = self.endpoint_circuits.read().await;
        let circuit = ep
            .get(&circuit_id)
            .ok_or_else(|| anyhow::anyhow!("no endpoint circuit {}", circuit_id))?;
        let relay = circuit.relay_peer;
        drop(ep);

        let msg = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(circuit_id.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(data)),
        ]);
        self.send_to_peer(&relay, message_types::CIRCUIT_DATA, msg)
            .await;
        Ok(())
    }

    /// Close a circuit we are an endpoint of.
    pub async fn close_endpoint_circuit(&self, circuit_id: u64) -> Result<()> {
        let ep = self.endpoint_circuits.write().await.remove(&circuit_id);
        let circuit = ep.ok_or_else(|| anyhow::anyhow!("no endpoint circuit {}", circuit_id))?;
        self.pending_initiations.write().await.remove(&circuit_id);

        let close = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(circuit_id.into()),
            ),
            (
                keys::REASON,
                ciborium::value::Value::Integer(reasons::NORMAL.into()),
            ),
        ]);
        self.send_to_peer(&circuit.relay_peer, message_types::CIRCUIT_CLOSE, close)
            .await;

        tracing::info!("relay: closed endpoint circuit {}", circuit_id);
        Ok(())
    }

    // ── Scope param helpers ───────────────────────────────────────────────────

    fn max_circuits(params: &p2pcd_types::ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::RELAY_MAX_CIRCUITS)
            .unwrap_or(DEFAULT_MAX_CIRCUITS)
    }

    fn ttl_secs(params: &p2pcd_types::ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::RELAY_TTL)
            .unwrap_or(DEFAULT_TTL_SECS)
    }

    // ── Event helper ──────────────────────────────────────────────────────────

    async fn fire_event(&self, event: CircuitEvent) {
        if let Some(tx) = self.event_tx.read().await.as_ref() {
            let _ = tx.send(event).await;
        }
    }

    // ── Message handlers ─────────────────────────────────────────────────────

    async fn handle_open(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let circuit_id = cbor_get_int(&map, keys::CIRCUIT_ID).unwrap_or(0);
        let target_bytes = cbor_get_bytes(&map, keys::TARGET_PEER).unwrap_or_default();
        let initiator_bytes = cbor_get_bytes(&map, keys::INITIATOR_PEER);
        let status = cbor_get_int(&map, keys::STATUS);

        // ── Case A: Forwarded OPEN — we are the TARGET ────────────────────
        // Carol forwarded Alice's CIRCUIT_OPEN to us. Contains INITIATOR_PEER.
        if let Some(init_bytes) = initiator_bytes {
            if init_bytes.len() != 32 {
                tracing::warn!("relay: CIRCUIT_OPEN forwarded with invalid initiator length");
                return Ok(());
            }
            let mut initiator = [0u8; 32];
            initiator.copy_from_slice(&init_bytes);

            tracing::info!(
                "relay: accepting forwarded circuit {} from initiator {} via relay {}",
                circuit_id,
                hex::encode(&initiator[..4]),
                hex::encode(&ctx.peer_id[..4])
            );

            // Store endpoint state — we are the target
            self.endpoint_circuits.write().await.insert(
                circuit_id,
                EndpointCircuit {
                    circuit_id,
                    relay_peer: ctx.peer_id,
                    remote_peer: initiator,
                    role: EndpointRole::Target,
                },
            );

            self.fire_event(CircuitEvent::Opened {
                circuit_id,
                remote_peer: initiator,
                role: EndpointRole::Target,
            })
            .await;

            // Respond with acceptance back through the relay
            let accept = cbor_encode_map(vec![
                (
                    keys::CIRCUIT_ID,
                    ciborium::value::Value::Integer(circuit_id.into()),
                ),
                (
                    keys::STATUS,
                    ciborium::value::Value::Integer(open_status::ACCEPTED.into()),
                ),
            ]);
            self.send_to_peer(&ctx.peer_id, message_types::CIRCUIT_OPEN, accept)
                .await;
            return Ok(());
        }

        // ── Case B: Acceptance with STATUS ────────────────────────────────
        // Could be arriving at the relay (from target) or at the initiator
        // (forwarded by relay).
        if let Some(st) = status {
            // Check if we're the relay — circuit_id in our relay circuits map
            {
                let circuits = self.circuits.read().await;
                if let Some(circuit) = circuits.get(&circuit_id) {
                    // We are the relay. Forward acceptance to the initiator.
                    let accept = cbor_encode_map(vec![
                        (
                            keys::CIRCUIT_ID,
                            ciborium::value::Value::Integer(circuit_id.into()),
                        ),
                        (keys::STATUS, ciborium::value::Value::Integer(st.into())),
                    ]);
                    let initiator = circuit.initiator;
                    drop(circuits);
                    self.send_to_peer(&initiator, message_types::CIRCUIT_OPEN, accept)
                        .await;

                    tracing::debug!(
                        "relay: forwarded acceptance for circuit {} to initiator {}",
                        circuit_id,
                        hex::encode(&initiator[..4])
                    );
                    return Ok(());
                }
            }

            // Check if we're the initiator — circuit_id in pending_initiations
            if let Some(target_peer) = self.pending_initiations.write().await.remove(&circuit_id) {
                // We are the initiator receiving the forwarded acceptance.
                self.endpoint_circuits.write().await.insert(
                    circuit_id,
                    EndpointCircuit {
                        circuit_id,
                        relay_peer: ctx.peer_id,
                        remote_peer: target_peer,
                        role: EndpointRole::Initiator,
                    },
                );

                self.fire_event(CircuitEvent::Opened {
                    circuit_id,
                    remote_peer: target_peer,
                    role: EndpointRole::Initiator,
                })
                .await;

                tracing::info!(
                    "relay: circuit {} accepted, endpoint to {} via relay {}",
                    circuit_id,
                    hex::encode(&target_peer[..4]),
                    hex::encode(&ctx.peer_id[..4])
                );
                return Ok(());
            }

            tracing::debug!(
                "relay: CIRCUIT_OPEN acceptance for unknown circuit {}",
                circuit_id
            );
            return Ok(());
        }

        // ── Case C: New circuit request — we are the RELAY ────────────────
        if target_bytes.len() != 32 {
            tracing::warn!(
                "relay: CIRCUIT_OPEN with invalid target peer length from {}",
                hex::encode(&ctx.peer_id[..4])
            );
            return Ok(());
        }
        let mut target = [0u8; 32];
        target.copy_from_slice(&target_bytes);

        tracing::debug!(
            "relay: CIRCUIT_OPEN id={} from {} → target {}",
            circuit_id,
            hex::encode(&ctx.peer_id[..4]),
            hex::encode(&target[..4])
        );

        // Check circuit capacity
        let max = Self::max_circuits(&ctx.params);
        let current = self.circuits.read().await.len() as u64;
        if current >= max {
            tracing::warn!(
                "relay: capacity exceeded ({}/{}), rejecting circuit {}",
                current,
                max,
                circuit_id
            );
            let close = cbor_encode_map(vec![
                (
                    keys::CIRCUIT_ID,
                    ciborium::value::Value::Integer(circuit_id.into()),
                ),
                (
                    keys::REASON,
                    ciborium::value::Value::Integer(reasons::CAPACITY_EXCEEDED.into()),
                ),
            ]);
            self.send_to_peer(&ctx.peer_id, message_types::CIRCUIT_CLOSE, close)
                .await;
            return Ok(());
        }

        // Check if we can reach the target
        let senders = self.peer_senders.read().await;
        if !senders.contains_key(&target) {
            tracing::info!(
                "relay: target {} unreachable for circuit {}",
                hex::encode(&target[..4]),
                circuit_id
            );
            let close = cbor_encode_map(vec![
                (
                    keys::CIRCUIT_ID,
                    ciborium::value::Value::Integer(circuit_id.into()),
                ),
                (
                    keys::REASON,
                    ciborium::value::Value::Integer(reasons::TARGET_UNREACHABLE.into()),
                ),
            ]);
            self.send_to_peer(&ctx.peer_id, message_types::CIRCUIT_CLOSE, close)
                .await;
            return Ok(());
        }
        drop(senders);

        let ttl = Self::ttl_secs(&ctx.params);

        // Create circuit state
        self.circuits.write().await.insert(
            circuit_id,
            Circuit {
                circuit_id,
                initiator: ctx.peer_id,
                target,
                created_at: unix_now(),
                bytes_relayed: 0,
                ttl_secs: ttl,
            },
        );

        // Forward CIRCUIT_OPEN to target with initiator info
        let forward = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(circuit_id.into()),
            ),
            (
                keys::INITIATOR_PEER,
                ciborium::value::Value::Bytes(ctx.peer_id.to_vec()),
            ),
        ]);
        self.send_to_peer(&target, message_types::CIRCUIT_OPEN, forward)
            .await;

        tracing::info!(
            "relay: circuit {} established: {} ↔ {} (ttl={}s)",
            circuit_id,
            hex::encode(&ctx.peer_id[..4]),
            hex::encode(&target[..4]),
            ttl
        );

        Ok(())
    }

    async fn handle_data(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let circuit_id = cbor_get_int(&map, keys::CIRCUIT_ID).unwrap_or(0);
        let data = cbor_get_bytes(&map, keys::DATA).unwrap_or_default();

        let data_len = data.len() as u64;

        // ── Relay path (forwards to the other end) ────────────────────────
        {
            let mut circuits = self.circuits.write().await;
            if let Some(circuit) = circuits.get_mut(&circuit_id) {
                // Check TTL
                if circuit.is_expired() {
                    let initiator = circuit.initiator;
                    let target = circuit.target;
                    circuits.remove(&circuit_id);
                    drop(circuits);

                    // Notify both ends
                    let close = cbor_encode_map(vec![
                        (
                            keys::CIRCUIT_ID,
                            ciborium::value::Value::Integer(circuit_id.into()),
                        ),
                        (
                            keys::REASON,
                            ciborium::value::Value::Integer(reasons::TTL_EXPIRED.into()),
                        ),
                    ]);
                    self.send_to_peer(&initiator, message_types::CIRCUIT_CLOSE, close.clone())
                        .await;
                    self.send_to_peer(&target, message_types::CIRCUIT_CLOSE, close)
                        .await;
                    tracing::info!("relay: circuit {} expired, closed", circuit_id);
                    return Ok(());
                }

                let other = circuit.other_end(&ctx.peer_id);
                circuit.bytes_relayed += data_len;
                drop(circuits);

                match other {
                    Some(peer) => {
                        // Forward the data payload as-is to the other end
                        let fwd = cbor_encode_map(vec![
                            (
                                keys::CIRCUIT_ID,
                                ciborium::value::Value::Integer(circuit_id.into()),
                            ),
                            (keys::DATA, ciborium::value::Value::Bytes(data)),
                        ]);
                        self.send_to_peer(&peer, message_types::CIRCUIT_DATA, fwd)
                            .await;
                    }
                    None => {
                        tracing::warn!(
                            "relay: CIRCUIT_DATA from {} not part of circuit {}",
                            hex::encode(&ctx.peer_id[..4]),
                            circuit_id
                        );
                    }
                }
                return Ok(());
            }
        }

        // ── Endpoint path (deliver to daemon via callback) ────────────────
        if let Some(ep) = self.endpoint_circuits.read().await.get(&circuit_id) {
            self.fire_event(CircuitEvent::Data {
                circuit_id,
                from: ep.remote_peer,
                data,
            })
            .await;
            return Ok(());
        }

        tracing::debug!("relay: CIRCUIT_DATA for unknown circuit {}", circuit_id);
        Ok(())
    }

    async fn handle_close(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let circuit_id = cbor_get_int(&map, keys::CIRCUIT_ID).unwrap_or(0);
        let reason = cbor_get_int(&map, keys::REASON).unwrap_or(reasons::NORMAL);

        // ── Relay path ────────────────────────────────────────────────────
        {
            let mut circuits = self.circuits.write().await;
            if let Some(circuit) = circuits.remove(&circuit_id) {
                drop(circuits);

                // Forward CLOSE to the other end
                if let Some(other) = circuit.other_end(&ctx.peer_id) {
                    let close = cbor_encode_map(vec![
                        (
                            keys::CIRCUIT_ID,
                            ciborium::value::Value::Integer(circuit_id.into()),
                        ),
                        (keys::REASON, ciborium::value::Value::Integer(reason.into())),
                    ]);
                    self.send_to_peer(&other, message_types::CIRCUIT_CLOSE, close)
                        .await;
                }

                tracing::info!(
                    "relay: circuit {} closed (reason={}, relayed {} bytes)",
                    circuit_id,
                    reason,
                    circuit.bytes_relayed
                );
                return Ok(());
            }
        }

        // ── Endpoint path ─────────────────────────────────────────────────
        if self
            .endpoint_circuits
            .write()
            .await
            .remove(&circuit_id)
            .is_some()
        {
            self.fire_event(CircuitEvent::Closed { circuit_id, reason })
                .await;

            tracing::info!(
                "relay: endpoint circuit {} closed (reason={})",
                circuit_id,
                reason
            );
            return Ok(());
        }

        tracing::debug!("relay: CIRCUIT_CLOSE for unknown circuit {}", circuit_id);
        Ok(())
    }

    // ── Sending helpers ──────────────────────────────────────────────────────

    async fn send_to_peer(&self, peer_id: &PeerId, msg_type: u64, payload: Vec<u8>) {
        let senders = self.peer_senders.read().await;
        if let Some(tx) = senders.get(peer_id) {
            let _ = tx.send(make_capability_msg(msg_type, payload)).await;
        } else {
            tracing::debug!(
                "relay: no sender for peer {}, dropping msg_type={}",
                hex::encode(&peer_id[..4]),
                msg_type
            );
        }
    }

    /// Reap circuits that have exceeded their TTL.
    pub async fn reap_expired_circuits(&self) -> Vec<(PeerId, PeerId, u64)> {
        let mut circuits = self.circuits.write().await;
        let mut reaped = Vec::new();
        circuits.retain(|id, c| {
            if c.is_expired() {
                tracing::info!("relay: reaping expired circuit {}", id);
                reaped.push((c.initiator, c.target, *id));
                false
            } else {
                true
            }
        });
        reaped
    }

    /// Reap expired circuits and notify both ends.
    pub async fn reap_and_notify(&self) {
        let reaped = self.reap_expired_circuits().await;
        for (initiator, target, circuit_id) in reaped {
            let close = cbor_encode_map(vec![
                (
                    keys::CIRCUIT_ID,
                    ciborium::value::Value::Integer(circuit_id.into()),
                ),
                (
                    keys::REASON,
                    ciborium::value::Value::Integer(reasons::TTL_EXPIRED.into()),
                ),
            ]);
            self.send_to_peer(&initiator, message_types::CIRCUIT_CLOSE, close.clone())
                .await;
            self.send_to_peer(&target, message_types::CIRCUIT_CLOSE, close)
                .await;
        }
    }
}

impl CapabilityHandler for RelayHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn capability_name(&self) -> &str {
        "core.network.relay.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[
            message_types::CIRCUIT_OPEN,
            message_types::CIRCUIT_DATA,
            message_types::CIRCUIT_CLOSE,
        ]
    }

    fn on_message(
        &self,
        msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let payload = payload.to_vec();
        let ctx = ctx.clone();
        Box::pin(async move {
            match msg_type {
                message_types::CIRCUIT_OPEN => self.handle_open(&payload, &ctx).await,
                message_types::CIRCUIT_DATA => self.handle_data(&payload, &ctx).await,
                message_types::CIRCUIT_CLOSE => self.handle_close(&payload, &ctx).await,
                _ => Ok(()),
            }
        })
    }

    fn on_deactivated(
        &self,
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            // Close all relay circuits involving this peer
            let mut circuits = self.circuits.write().await;
            let to_close: Vec<u64> = circuits
                .iter()
                .filter(|(_, c)| c.initiator == peer_id || c.target == peer_id)
                .map(|(id, _)| *id)
                .collect();

            for circuit_id in &to_close {
                if let Some(circuit) = circuits.remove(circuit_id) {
                    // Notify the other end
                    if let Some(other) = circuit.other_end(&peer_id) {
                        let close = cbor_encode_map(vec![
                            (
                                keys::CIRCUIT_ID,
                                ciborium::value::Value::Integer((*circuit_id).into()),
                            ),
                            (
                                keys::REASON,
                                ciborium::value::Value::Integer(reasons::NORMAL.into()),
                            ),
                        ]);
                        drop(circuits);
                        self.send_to_peer(&other, message_types::CIRCUIT_CLOSE, close)
                            .await;
                        circuits = self.circuits.write().await;
                    }
                }
            }
            drop(circuits);

            // Close endpoint circuits through this relay peer
            let mut ep_circuits = self.endpoint_circuits.write().await;
            let ep_to_close: Vec<u64> = ep_circuits
                .iter()
                .filter(|(_, ep)| ep.relay_peer == peer_id)
                .map(|(id, _)| *id)
                .collect();
            for circuit_id in &ep_to_close {
                if ep_circuits.remove(circuit_id).is_some() {
                    self.fire_event(CircuitEvent::Closed {
                        circuit_id: *circuit_id,
                        reason: reasons::NORMAL,
                    })
                    .await;
                }
            }
            drop(ep_circuits);

            // Clean up pending initiations through this peer
            // (We don't know which relay peer they were through, so leave them
            // to time out. The initiator side will get a timeout error.)

            // Remove the peer's sender
            self.peer_senders.write().await.remove(&peer_id);

            tracing::debug!(
                "relay: cleaned up {} relay + {} endpoint circuits for peer {}",
                to_close.len(),
                ep_to_close.len(),
                hex::encode(&peer_id[..4])
            );
            Ok(())
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use p2pcd_types::{CapabilityHandler, ScopeParams};

    fn make_ctx(peer_id: PeerId) -> CapabilityContext {
        CapabilityContext {
            peer_id,
            params: ScopeParams::default(),
            capability_name: "core.network.relay.1".to_string(),
        }
    }

    #[test]
    fn handler_metadata() {
        let h = RelayHandler::new();
        assert_eq!(h.capability_name(), "core.network.relay.1");
        assert_eq!(h.handled_message_types(), &[13, 14, 15]);
    }

    #[tokio::test]
    async fn open_target_unreachable() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let target_b = [2u8; 32];

        // Register sender for peer A (the initiator) but NOT for target B
        let (tx_a, mut rx_a) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;

        let payload = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(target_b.to_vec()),
            ),
        ]);

        let ctx = make_ctx(peer_a);
        handler.handle_open(&payload, &ctx).await.unwrap();

        // Should get CIRCUIT_CLOSE with TARGET_UNREACHABLE
        let msg = rx_a.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::CIRCUIT_CLOSE);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::REASON),
                    Some(reasons::TARGET_UNREACHABLE)
                );
                assert_eq!(cbor_get_int(&map, keys::CIRCUIT_ID), Some(1));
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.circuit_count().await, 0);
    }

    #[tokio::test]
    async fn open_capacity_exceeded() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let target_b = [2u8; 32];

        let (tx_a, _rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, _rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(target_b, tx_b).await;

        // Set max_circuits to 0 via scope params
        let mut params = ScopeParams::default();
        params.set_ext(
            scope_keys::RELAY_MAX_CIRCUITS,
            p2pcd_types::ScopeValue::Uint(0),
        );
        let ctx = CapabilityContext {
            peer_id: peer_a,
            params,
            capability_name: "core.network.relay.1".to_string(),
        };

        let payload = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(target_b.to_vec()),
            ),
        ]);
        handler.handle_open(&payload, &ctx).await.unwrap();

        assert_eq!(handler.circuit_count().await, 0);
    }

    #[tokio::test]
    async fn full_circuit_lifecycle() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32]; // initiator
        let peer_b = [2u8; 32]; // target

        let (tx_a, mut rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(peer_b, tx_b).await;

        // 1. Peer A opens circuit to B
        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(peer_b.to_vec()),
            ),
        ]);
        let ctx_a = make_ctx(peer_a);
        handler.handle_open(&open, &ctx_a).await.unwrap();

        // B should receive forwarded CIRCUIT_OPEN with initiator info
        let msg = rx_b.recv().await.unwrap();
        match &msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(*message_type, message_types::CIRCUIT_OPEN);
                let map = decode_payload(payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::CIRCUIT_ID), Some(42));
                let init = cbor_get_bytes(&map, keys::INITIATOR_PEER).unwrap();
                assert_eq!(init, peer_a.to_vec());
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.circuit_count().await, 1);

        // 2. A sends data through the circuit
        let data_msg = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"hello from A".to_vec()),
            ),
        ]);
        handler.handle_data(&data_msg, &ctx_a).await.unwrap();

        // B should receive the forwarded data
        let msg = rx_b.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::CIRCUIT_DATA);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_bytes(&map, keys::DATA).unwrap(), b"hello from A");
            }
            _ => panic!("expected CapabilityMsg"),
        }

        // 3. B sends data back through the circuit
        let ctx_b = make_ctx(peer_b);
        let reply = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"hello from B".to_vec()),
            ),
        ]);
        handler.handle_data(&reply, &ctx_b).await.unwrap();

        // A should receive the forwarded reply
        let msg = rx_a.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::CIRCUIT_DATA);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_bytes(&map, keys::DATA).unwrap(), b"hello from B");
            }
            _ => panic!("expected CapabilityMsg"),
        }

        // 4. A closes the circuit
        let close = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::REASON,
                ciborium::value::Value::Integer(reasons::NORMAL.into()),
            ),
        ]);
        handler.handle_close(&close, &ctx_a).await.unwrap();

        // B should receive CIRCUIT_CLOSE
        let msg = rx_b.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::CIRCUIT_CLOSE);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::REASON), Some(reasons::NORMAL));
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.circuit_count().await, 0);
    }

    #[tokio::test]
    async fn data_for_unknown_circuit_ignored() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];

        let data = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(999u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"ghost data".to_vec()),
            ),
        ]);
        let ctx = make_ctx(peer_a);
        // Should not panic or error
        handler.handle_data(&data, &ctx).await.unwrap();
    }

    #[tokio::test]
    async fn close_for_unknown_circuit_ignored() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];

        let close = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(999u64.into()),
            ),
            (
                keys::REASON,
                ciborium::value::Value::Integer(reasons::NORMAL.into()),
            ),
        ]);
        let ctx = make_ctx(peer_a);
        handler.handle_close(&close, &ctx).await.unwrap();
    }

    #[tokio::test]
    async fn data_from_non_participant_ignored() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];
        let peer_c = [3u8; 32]; // not part of circuit

        let (tx_a, _rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, _rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(peer_b, tx_b).await;

        // Open circuit between A and B
        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(peer_b.to_vec()),
            ),
        ]);
        let ctx_a = make_ctx(peer_a);
        handler.handle_open(&open, &ctx_a).await.unwrap();

        // C tries to send data on that circuit
        let data = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"intruder".to_vec()),
            ),
        ]);
        let ctx_c = make_ctx(peer_c);
        handler.handle_data(&data, &ctx_c).await.unwrap();
        // Should be silently ignored — no forwarding
    }

    #[tokio::test]
    async fn on_deactivated_closes_peer_circuits() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];

        let (tx_a, _rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(peer_b, tx_b).await;

        // Open circuit
        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(peer_b.to_vec()),
            ),
        ]);
        let ctx_a = make_ctx(peer_a);
        handler.handle_open(&open, &ctx_a).await.unwrap();

        // Drain the forwarded OPEN from rx_b
        let _ = rx_b.recv().await;

        assert_eq!(handler.circuit_count().await, 1);

        // Peer A disconnects
        handler.on_deactivated(&ctx_a).await.unwrap();

        assert_eq!(handler.circuit_count().await, 0);

        // B should have received CIRCUIT_CLOSE
        let msg = rx_b.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::CIRCUIT_CLOSE);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::REASON), Some(reasons::NORMAL));
            }
            _ => panic!("expected CapabilityMsg"),
        }
    }

    #[tokio::test]
    async fn reap_expired_circuits() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];

        // Insert a circuit that's already expired
        handler.circuits.write().await.insert(
            1,
            Circuit {
                circuit_id: 1,
                initiator: peer_a,
                target: peer_b,
                created_at: 0, // epoch = very expired
                bytes_relayed: 0,
                ttl_secs: 1,
            },
        );

        assert_eq!(handler.circuit_count().await, 1);

        let reaped = handler.reap_expired_circuits().await;
        assert_eq!(reaped.len(), 1);
        assert_eq!(reaped[0].2, 1); // circuit_id
        assert_eq!(handler.circuit_count().await, 0);
    }

    #[tokio::test]
    async fn bytes_relayed_tracking() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];

        let (tx_a, _rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, _rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(peer_b, tx_b).await;

        // Open circuit
        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(peer_b.to_vec()),
            ),
        ]);
        let ctx_a = make_ctx(peer_a);
        handler.handle_open(&open, &ctx_a).await.unwrap();

        // Send data
        let data = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(vec![0u8; 100])),
        ]);
        handler.handle_data(&data, &ctx_a).await.unwrap();
        handler.handle_data(&data, &ctx_a).await.unwrap();

        let circuits = handler.circuits.read().await;
        let circuit = circuits.get(&1).unwrap();
        assert_eq!(circuit.bytes_relayed, 200);
    }

    #[tokio::test]
    async fn bidirectional_data_forwarding() {
        let handler = RelayHandler::new();
        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];

        let (tx_a, mut rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(peer_b, tx_b).await;

        // Open circuit
        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(peer_b.to_vec()),
            ),
        ]);
        let ctx_a = make_ctx(peer_a);
        let ctx_b = make_ctx(peer_b);
        handler.handle_open(&open, &ctx_a).await.unwrap();
        let _ = rx_b.recv().await; // drain forwarded OPEN

        // A → B
        let msg_ab = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"A to B".to_vec()),
            ),
        ]);
        handler.handle_data(&msg_ab, &ctx_a).await.unwrap();
        let fwd = rx_b.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = fwd {
            let map = decode_payload(&payload).unwrap();
            assert_eq!(cbor_get_bytes(&map, keys::DATA).unwrap(), b"A to B");
        }

        // B → A
        let msg_ba = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"B to A".to_vec()),
            ),
        ]);
        handler.handle_data(&msg_ba, &ctx_b).await.unwrap();
        let fwd = rx_a.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = fwd {
            let map = decode_payload(&payload).unwrap();
            assert_eq!(cbor_get_bytes(&map, keys::DATA).unwrap(), b"B to A");
        }
    }

    // ── Endpoint circuit tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn acceptance_forwarding_through_relay() {
        // Carol correctly forwards Bob's acceptance back to Alice
        let handler = RelayHandler::new(); // Carol
        let peer_a = [1u8; 32]; // Alice (initiator)
        let peer_b = [2u8; 32]; // Bob (target)

        let (tx_a, mut rx_a) = tokio::sync::mpsc::channel(16);
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(16);
        handler.add_peer_sender(peer_a, tx_a).await;
        handler.add_peer_sender(peer_b, tx_b).await;

        // Alice opens circuit through Carol
        let open = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::TARGET_PEER,
                ciborium::value::Value::Bytes(peer_b.to_vec()),
            ),
        ]);
        let ctx_a = make_ctx(peer_a);
        handler.handle_open(&open, &ctx_a).await.unwrap();
        let _ = rx_b.recv().await; // drain forwarded OPEN to Bob

        // Bob sends acceptance back through Carol
        let accept = cbor_encode_map(vec![
            (
                keys::CIRCUIT_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::STATUS,
                ciborium::value::Value::Integer(open_status::ACCEPTED.into()),
            ),
        ]);
        let ctx_b = make_ctx(peer_b);
        handler.handle_open(&accept, &ctx_b).await.unwrap();

        // Alice should receive the forwarded acceptance
        let msg = rx_a.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::CIRCUIT_OPEN);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::CIRCUIT_ID), Some(42));
                assert_eq!(
                    cbor_get_int(&map, keys::STATUS),
                    Some(open_status::ACCEPTED)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }
    }

    #[tokio::test]
    async fn endpoint_circuit_lifecycle() {
        // Full 3-node test: Alice initiates through Carol to Bob
        let carol = RelayHandler::new();
        let alice = RelayHandler::new();
        let bob = RelayHandler::new();

        let peer_alice = [1u8; 32];
        let peer_carol = [3u8; 32];
        let peer_bob = [2u8; 32];

        // Carol's peer senders (she knows both Alice and Bob)
        let (tx_to_alice, mut rx_alice) = tokio::sync::mpsc::channel(16);
        let (tx_to_bob, mut rx_bob) = tokio::sync::mpsc::channel(16);
        carol.add_peer_sender(peer_alice, tx_to_alice).await;
        carol.add_peer_sender(peer_bob, tx_to_bob).await;

        // Alice's sender to Carol
        let (tx_alice_to_carol, mut rx_carol_from_alice) = tokio::sync::mpsc::channel(16);
        alice.add_peer_sender(peer_carol, tx_alice_to_carol).await;

        // Bob's sender to Carol
        let (tx_bob_to_carol, mut rx_carol_from_bob) = tokio::sync::mpsc::channel(16);
        bob.add_peer_sender(peer_carol, tx_bob_to_carol).await;

        // Event callbacks
        let (alice_evt_tx, mut alice_evt_rx) = tokio::sync::mpsc::channel(16);
        let (bob_evt_tx, mut bob_evt_rx) = tokio::sync::mpsc::channel(16);
        alice.set_event_callback(alice_evt_tx).await;
        bob.set_event_callback(bob_evt_tx).await;

        // 1. Alice initiates circuit through Carol to Bob
        let circuit_id = alice.initiate_circuit(&peer_carol, &peer_bob).await;

        // Carol receives CIRCUIT_OPEN from Alice
        let msg = rx_carol_from_alice.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            carol
                .handle_open(&payload, &make_ctx(peer_alice))
                .await
                .unwrap();
        }

        // Bob receives forwarded CIRCUIT_OPEN from Carol
        let msg = rx_bob.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            bob.handle_open(&payload, &make_ctx(peer_carol))
                .await
                .unwrap();
        }

        // Bob should have fired an Opened event
        let evt = bob_evt_rx.recv().await.unwrap();
        assert!(matches!(
            evt,
            CircuitEvent::Opened {
                role: EndpointRole::Target,
                ..
            }
        ));

        // Carol receives Bob's acceptance
        let msg = rx_carol_from_bob.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            carol
                .handle_open(&payload, &make_ctx(peer_bob))
                .await
                .unwrap();
        }

        // Alice receives forwarded acceptance from Carol
        let msg = rx_alice.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            alice
                .handle_open(&payload, &make_ctx(peer_carol))
                .await
                .unwrap();
        }

        // Alice should have fired an Opened event
        let evt = alice_evt_rx.recv().await.unwrap();
        assert!(matches!(
            evt,
            CircuitEvent::Opened {
                role: EndpointRole::Initiator,
                ..
            }
        ));

        // 2. Alice sends data to Bob through Carol
        alice
            .send_circuit_data(circuit_id, b"hello bob".to_vec())
            .await
            .unwrap();

        // Carol receives and forwards
        let msg = rx_carol_from_alice.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            carol
                .handle_data(&payload, &make_ctx(peer_alice))
                .await
                .unwrap();
        }

        // Bob receives forwarded data
        let msg = rx_bob.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            bob.handle_data(&payload, &make_ctx(peer_carol))
                .await
                .unwrap();
        }

        // Bob should have received a Data event
        let evt = bob_evt_rx.recv().await.unwrap();
        match evt {
            CircuitEvent::Data { data, .. } => assert_eq!(data, b"hello bob"),
            _ => panic!("expected Data event"),
        }

        // 3. Alice closes the circuit
        alice.close_endpoint_circuit(circuit_id).await.unwrap();

        // Carol receives and forwards the close
        let msg = rx_carol_from_alice.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            carol
                .handle_close(&payload, &make_ctx(peer_alice))
                .await
                .unwrap();
        }

        // Bob receives the close
        let msg = rx_bob.recv().await.unwrap();
        if let ProtocolMessage::CapabilityMsg { payload, .. } = msg {
            bob.handle_close(&payload, &make_ctx(peer_carol))
                .await
                .unwrap();
        }

        // Bob should have received a Closed event
        let evt = bob_evt_rx.recv().await.unwrap();
        assert!(matches!(evt, CircuitEvent::Closed { .. }));

        assert_eq!(carol.circuit_count().await, 0);
        assert_eq!(alice.endpoint_circuit_count().await, 0);
        assert_eq!(bob.endpoint_circuit_count().await, 0);
    }
}

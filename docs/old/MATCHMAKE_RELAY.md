# Matchmake Relay — Phase 4 Implementation

## Peer Signaling for NAT Traversal

---

## 1. What This Is

When hole punching fails (Tier 2 timeout) or both peers are behind symmetric
NAT, a mutual friend already on the mesh can relay endpoint information so both
peers can attempt a direct WireGuard connection. This is **STUN-over-mesh** —
signaling only, no traffic forwarding.

Carol (the relay) passes a few small messages between Alice and Bob. Total
relay traffic: < 1KB. The actual WG tunnel is direct between Alice and Bob.
Carol never sees their traffic.

**This is a howm daemon process.** The orchestration — when to matchmake, what
endpoint info to exchange, triggering the WG punch — lives entirely in
`node/daemon`. It leverages p2pcd's existing relay circuit capability
(`core.network.relay.1`) as transport, but the matchmaking protocol is
daemon-level.

---

## 2. Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                        node/daemon                           │
│                                                              │
│  matchmake.rs                                                │
│  ┌──────────────────────────────────────────────────────┐    │
│  │ MatchmakeRequest  → build + send via p2pcd circuit   │    │
│  │ MatchmakeOffer    → forward endpoint info to target  │    │
│  │ MatchmakeExchange → respond with own endpoint info   │    │
│  │ Orchestrator      → trigger punch after exchange     │    │
│  └──────────────────────────────────────────────────────┘    │
│                          │                                   │
│                          │ consumes CircuitEvents via channel │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐    │
│  │ p2pcd relay circuits (CIRCUIT_OPEN/DATA/CLOSE)       │    │
│  │ node/p2pcd/src/capabilities/relay.rs                 │    │
│  │ Relay path: unchanged (Carol forwards blindly)       │    │
│  │ Endpoint path: NEW — fires CircuitEvent callback     │    │
│  │ when data arrives for us as a circuit endpoint       │    │
│  └──────────────────────────────────────────────────────┘    │
│                                                              │
│  invite.rs   → adds relay_candidates to invite tokens        │
│  punch.rs    → called after matchmake exchange completes     │
│  config.rs   → allow_relay already exists                    │
│  state.rs    → allow_relay runtime toggle already exists     │
└──────────────────────────────────────────────────────────────┘
```

### Why p2pcd circuits as transport

The existing relay capability handles the hard parts: circuit state, peer
routing, forwarding between two peers through an intermediary, TTL, cleanup.
Matchmaking is just 3 small messages riding a short-lived circuit. We open a
circuit through Carol, exchange endpoint info as CIRCUIT_DATA payloads, close
the circuit, then punch directly. The circuit lives for seconds, not minutes.

### Relay capability gap: endpoint-side completion

The current relay.rs implements Carol's relay role correctly but has two bugs
that prevent Alice and Bob from actually using circuits end-to-end:

1. **Acceptance never reaches Alice.** When Bob gets a forwarded CIRCUIT_OPEN
   and sends back an acceptance, Carol's `handle_open` receives it as a new
   CIRCUIT_OPEN (no INITIATOR_PEER field). It falls into the relay-node
   path, fails to find a TARGET_PEER, and drops silently. The acceptance
   never gets forwarded back to Alice.

2. **Endpoint peers can't receive CIRCUIT_DATA.** `handle_data` looks up the
   circuit_id in the circuits map and calls `other_end(ctx.peer_id)`. But the
   circuits map is relay-centric — only Carol stores circuit state. When Bob
   gets forwarded CIRCUIT_DATA from Carol, Bob's relay handler has no matching
   circuit, hits "unknown circuit", and drops it.

These are fixed in Section 2.1 below. The fixes are additive — Carol's relay
forwarding path is untouched.

---

### 2.1 Relay Capability Fixes (~75 lines in relay.rs)

All changes are in `node/p2pcd/src/capabilities/relay.rs`. No new message
types, no wire format changes, no new capability.

#### 2.1.1 Endpoint-Side Circuit State

New struct for tracking circuits from the perspective of an endpoint (Alice or
Bob), as opposed to the relay (Carol):

```rust
struct EndpointCircuit {
    circuit_id: u64,
    relay_peer: PeerId,     // Carol — who we talk through
    remote_peer: PeerId,    // the other endpoint
    role: EndpointRole,
}

#[derive(Debug, Clone, Copy)]
enum EndpointRole { Initiator, Target }
```

New field on `RelayHandler`:
```rust
pub struct RelayHandler {
    circuits: Arc<RwLock<HashMap<u64, Circuit>>>,                     // existing — relay state
    endpoint_circuits: Arc<RwLock<HashMap<u64, EndpointCircuit>>>,    // NEW — endpoint state
    peer_senders: Arc<RwLock<HashMap<PeerId, ...>>>,                 // existing
    data_callback: Arc<RwLock<Option<mpsc::Sender<CircuitEvent>>>>,  // NEW
}
```

#### 2.1.2 CircuitEvent Callback

```rust
/// Events delivered to the daemon when we are a circuit endpoint.
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
    Closed {
        circuit_id: u64,
        reason: u64,
    },
}

impl RelayHandler {
    /// Register a callback channel for circuit events.
    /// The daemon calls this once at startup.
    pub async fn set_data_callback(&self, tx: mpsc::Sender<CircuitEvent>) {
        *self.data_callback.write().await = Some(tx);
    }
}
```

#### 2.1.3 Fix handle_open — Acceptance Forwarding

Current `handle_open` has two branches:
- `is_forwarded` (has INITIATOR_PEER) → we're the target
- else → we're the relay

Missing third case: **acceptance arriving at the relay**. Bob sends back
`{circuit_id, status: ACCEPTED}` with no INITIATOR_PEER and no TARGET_PEER.
Carol needs to recognize this as an acceptance for an existing circuit and
forward it to the initiator.

```rust
// In handle_open, after the is_forwarded branch and before the relay branch:

// Check if this is an acceptance for a circuit we're relaying
if let Some(status) = cbor_get_int(&map, keys::STATUS) {
    // This is a response (ACCEPTED/REJECTED) from the target,
    // arriving at the relay. Forward to the initiator.
    let circuits = self.circuits.read().await;
    if let Some(circuit) = circuits.get(&circuit_id) {
        let accept = cbor_encode_map(vec![
            (keys::CIRCUIT_ID, ciborium::value::Value::Integer(circuit_id.into())),
            (keys::STATUS, ciborium::value::Value::Integer(status.into())),
        ]);
        self.send_to_peer(&circuit.initiator, message_types::CIRCUIT_OPEN, accept).await;
    }
    return Ok(());
}
```

On Alice's side, when the forwarded acceptance arrives (has STATUS, no
INITIATOR_PEER, no TARGET_PEER): store an EndpointCircuit and fire callback.

```rust
// In handle_open, inside the is_forwarded check — actually, acceptance
// arrives with STATUS but without INITIATOR_PEER, so add after the
// acceptance-forwarding check above:

// Check if this is an acceptance arriving at the initiator
if status is Some && we're not the relay (no circuit in our circuits map):
    // We're Alice receiving the acceptance. Store endpoint state.
    self.endpoint_circuits.write().await.insert(circuit_id, EndpointCircuit {
        circuit_id,
        relay_peer: ctx.peer_id,  // Carol sent this to us
        remote_peer: ???,         // We know this from our pending request
        role: EndpointRole::Initiator,
    });
    // Fire callback
    fire_event(CircuitEvent::Opened { circuit_id, remote_peer, role: Initiator });
```

There's a subtlety: when Alice receives the acceptance, she doesn't know Bob's
PeerId from the acceptance message alone. She knows it from her own
CIRCUIT_OPEN request. So we need one more piece of state:

```rust
/// Circuits we've initiated but haven't been accepted yet.
pending_initiations: Arc<RwLock<HashMap<u64, PeerId>>>,  // circuit_id → target
```

Alice stores the target PeerId when she sends CIRCUIT_OPEN. When acceptance
arrives, she looks it up.

On Bob's side, the existing `is_forwarded` branch already handles the target
receiving a forwarded OPEN. Add endpoint state storage + callback there:

```rust
// In the is_forwarded branch, after sending acceptance:
self.endpoint_circuits.write().await.insert(circuit_id, EndpointCircuit {
    circuit_id,
    relay_peer: ctx.peer_id,  // Carol
    remote_peer: initiator,   // already extracted from INITIATOR_PEER
    role: EndpointRole::Target,
});
fire_event(CircuitEvent::Opened { circuit_id, remote_peer: initiator, role: Target });
```

#### 2.1.4 Fix handle_data — Endpoint Path

Add an endpoint check before the "unknown circuit" drop:

```rust
async fn handle_data(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
    let map = decode_payload(payload)?;
    let circuit_id = cbor_get_int(&map, keys::CIRCUIT_ID).unwrap_or(0);
    let data = cbor_get_bytes(&map, keys::DATA).unwrap_or_default();

    // ── Relay path (existing, unchanged) ──────────────────────────────
    // Check if we're relaying this circuit
    {
        let mut circuits = self.circuits.write().await;
        if let Some(circuit) = circuits.get_mut(&circuit_id) {
            // ... existing relay forwarding logic, unchanged ...
            return Ok(());
        }
    }

    // ── Endpoint path (NEW) ───────────────────────────────────────────
    // We're an endpoint of this circuit — deliver to daemon via callback
    if let Some(ep) = self.endpoint_circuits.read().await.get(&circuit_id) {
        if let Some(cb) = self.data_callback.read().await.as_ref() {
            let _ = cb.send(CircuitEvent::Data {
                circuit_id,
                from: ep.remote_peer,
                data,
            }).await;
        }
        return Ok(());
    }

    // Unknown circuit
    tracing::debug!("relay: CIRCUIT_DATA for unknown circuit {}", circuit_id);
    Ok(())
}
```

#### 2.1.5 Fix handle_close — Endpoint Cleanup

Same pattern: check endpoint_circuits after the relay circuits map, fire
Closed event, remove entry.

#### 2.1.6 Fix on_deactivated — Endpoint Cleanup

When a peer disconnects, clean up any endpoint_circuits where `relay_peer`
matches the disconnecting peer. Fire Closed events.

#### 2.1.7 New Public Methods for the Daemon

```rust
impl RelayHandler {
    /// Register a callback for circuit events (daemon calls once at startup).
    pub async fn set_data_callback(&self, tx: mpsc::Sender<CircuitEvent>);

    /// Initiate a circuit through a relay to a target peer.
    /// Stores pending initiation state. The daemon calls this, then waits
    /// for CircuitEvent::Opened on the callback channel.
    pub async fn initiate_circuit(
        &self,
        relay_peer: &PeerId,
        target_peer: &PeerId,
    ) -> u64;  // returns circuit_id

    /// Send data on a circuit we're an endpoint of.
    pub async fn send_circuit_data(
        &self,
        circuit_id: u64,
        data: Vec<u8>,
    ) -> Result<()>;

    /// Close a circuit we're an endpoint of.
    pub async fn close_circuit(&self, circuit_id: u64) -> Result<()>;
}
```

`initiate_circuit` generates a circuit_id, stores it in pending_initiations,
and sends CIRCUIT_OPEN to the relay peer with the target. The daemon then
listens on the callback channel for the Opened event.

`send_circuit_data` looks up the endpoint_circuit, wraps data in a
CIRCUIT_DATA message, and sends it to the relay_peer (who forwards it).

`close_circuit` sends CIRCUIT_CLOSE to the relay_peer and removes endpoint
state.

---

## 3. Message Protocol

Three CBOR-encoded messages sent as CIRCUIT_DATA payloads on a relay circuit.
The circuit is opened by the initiator (Alice) through the relay peer (Carol)
to the target (Bob).

### 3.1 MatchmakeRequest (Alice → Carol → Bob)

Alice opens a circuit to Bob through Carol and sends her endpoint info.

```
CBOR map {
    1: "matchmake-request"       // msg_type (text)
    2: <alice_wg_pubkey>         // bytes(32) — Alice's WG public key
    3: <external_ip>             // text — Alice's STUN-reflected IP
    4: <external_port>           // uint — Alice's STUN-reflected port
    5: <wg_port>                 // uint — Alice's actual WG listen port
    6: <nat_type>                // text — "cone" | "symmetric" | "unknown"
    7: <observed_stride>         // int — port allocation stride
    8: [<ipv6_gua>, ...]         // array(text) — IPv6 GUA candidates
    9: <psk>                     // bytes — pre-shared key for the WG peer
   10: <assigned_ip>             // text — IP Alice assigned for Bob on her wg0
   11: <alice_wg_address>        // text — Alice's own WG address
}
```

### 3.2 MatchmakeOffer (Carol forwards to Bob)

Carol doesn't construct this — it's just Alice's MatchmakeRequest arriving at
Bob via CIRCUIT_DATA forwarding. From Bob's perspective, he receives a
CIRCUIT_DATA on a circuit that was opened to him. The `msg_type` field tells
him it's a matchmake request.

No separate message type needed. The relay capability already forwards
CIRCUIT_DATA transparently.

### 3.3 MatchmakeExchange (Bob → Carol → Alice)

Bob responds with his own endpoint info on the same circuit.

```
CBOR map {
    1: "matchmake-exchange"      // msg_type (text)
    2: <bob_wg_pubkey>           // bytes(32) — Bob's WG public key
    3: <external_ip>             // text — Bob's STUN-reflected IP
    4: <external_port>           // uint — Bob's STUN-reflected port
    5: <wg_port>                 // uint — Bob's actual WG listen port
    6: <nat_type>                // text
    7: <observed_stride>         // int
    8: [<ipv6_gua>, ...]         // array(text)
    9: <bob_wg_address>          // text — Bob's own WG address
}
```

PSK and assigned_ip are not in the exchange — Alice already provided those in
her request. Bob uses them directly.

### 3.4 Circuit Lifecycle

```
Alice                       Carol (relay)                    Bob
  │                              │                             │
  │ CIRCUIT_OPEN ──────────────► │                             │
  │                              │ CIRCUIT_OPEN (forwarded) ──►│
  │                              │◄── CIRCUIT_OPEN (accepted) ─│
  │◄── CIRCUIT_OPEN (accepted) ──│                             │
  │                              │                             │
  │ CIRCUIT_DATA ──────────────► │                             │
  │  (MatchmakeRequest)          │ CIRCUIT_DATA (forwarded) ──►│
  │                              │                             │
  │                              │◄── CIRCUIT_DATA ────────────│
  │◄── CIRCUIT_DATA (forwarded) ─│    (MatchmakeExchange)      │
  │                              │                             │
  │ CIRCUIT_CLOSE ─────────────► │                             │
  │                              │ CIRCUIT_CLOSE (forwarded) ─►│
  │                              │                             │
  │ ◄═══════ Both configure WG and attempt direct punch ═════►│
```

Total relay involvement: 6 messages (open, open-fwd, accept, accept-fwd,
2× data fwd, close, close-fwd). Carol's work is done in under a second.

---

## 4. Invite Token: relay_candidates Field

### 4.1 When to Include

At invite creation, if the node's reachability is `Punchable` or `RelayOnly`:

```rust
if reachability != Reachability::Direct {
    relay_candidates = collect_relay_capable_peers();
}
```

### 4.2 What It Contains

A list of WG public keys of peers that:
1. Are currently connected (active WG tunnel)
2. Have `core.network.relay.1` in their active p2pcd capability set
3. Have relay enabled (we can't know their config, but if they negotiated the
   relay capability, they're willing)

Encoded as comma-separated base64 pubkeys appended to the invite token.

### 4.3 Invite Token Format Change

Current v2 format (9 fields):
```
pubkey|endpoint|wg_addr|psk|assigned_ip|daemon_port|expiry|ipv6_csv|wg_port
```

New v3 format (12 fields — trailing, backward compatible):
```
pubkey|endpoint|wg_addr|psk|assigned_ip|daemon_port|expiry|ipv6_csv|wg_port|nat_type|stride|relay_csv
```

| Field | Index | When included | Example |
|---|---|---|---|
| `nat_type` | 9 | When behind NAT | `cone`, `symmetric`, `unknown` |
| `stride` | 10 | When stride != 0 | `2`, `-1`, `0` |
| `relay_csv` | 11 | When relay candidates exist | `<pubkey1>,<pubkey2>` |

Older parsers that splitn on `|` and take the first 9 will still work — new
fields are trailing. Empty string for fields that don't apply.

### 4.4 Changes to invite.rs

```rust
pub fn generate(
    // ... existing params ...
    nat_profile: Option<&NatProfile>,        // NEW
    relay_candidates: &[String],             // NEW — base64 WG pubkeys
) -> anyhow::Result<String> {
    // ... existing logic ...

    let nat_type_str = nat_profile.map(|p| p.nat_type.to_string()).unwrap_or_default();
    let stride_str = nat_profile.map(|p| p.observed_stride.to_string()).unwrap_or_default();
    let relay_csv = relay_candidates.join(",");

    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        // ... existing 9 fields ...
        nat_type_str,
        stride_str,
        relay_csv,
    );
    // ...
}
```

```rust
pub struct DecodedInvite {
    // ... existing fields ...
    pub their_nat_type: Option<NatType>,      // NEW
    pub their_stride: i32,                    // NEW
    pub their_relay_candidates: Vec<String>,  // NEW — base64 WG pubkeys
}
```

---

## 5. Matchmake Orchestration (matchmake.rs)

New file: `node/daemon/src/matchmake.rs`

This is the daemon-level coordinator. It doesn't touch p2pcd internals — it
uses the p2pcd engine's public API to open circuits and send data.

### 5.1 Initiator Side (Alice)

Triggered when:
- Tier 2 hole punch times out, OR
- Invite includes `nat_type: symmetric` and we detect our own NAT is also
  symmetric (skip straight to Tier 3)

```rust
pub async fn initiate_matchmake(
    state: &AppState,
    target_pubkey: &str,           // Bob's WG pubkey (from invite)
    relay_candidates: &[String],   // From invite token
    our_endpoint_info: EndpointInfo,
    peer_config: PeerConfig,       // PSK, assigned IP, etc.
) -> Result<MatchmakeResult, MatchmakeError> {
    // 1. Find a mutual relay peer
    let relay_pubkey = find_mutual_relay(state, relay_candidates).await?;

    // 2. Open a circuit through the relay to the target
    let circuit_id = open_matchmake_circuit(state, &relay_pubkey, target_pubkey).await?;

    // 3. Send MatchmakeRequest with our endpoint info
    send_matchmake_request(state, circuit_id, &our_endpoint_info, &peer_config).await?;

    // 4. Wait for MatchmakeExchange response (timeout: 30s)
    let their_info = await_matchmake_exchange(state, circuit_id, Duration::from_secs(30)).await?;

    // 5. Close the circuit — Carol's job is done
    close_circuit(state, circuit_id).await;

    // 6. Configure WG peer and attempt direct punch
    let punch_config = build_punch_config(&their_info, &our_endpoint_info, &peer_config);
    let punch_result = punch::run_punch(punch_config).await;

    match punch_result {
        PunchResult::Success { .. } => Ok(MatchmakeResult::Connected),
        PunchResult::Timeout { .. } => Ok(MatchmakeResult::PunchFailed),
        PunchResult::Error(e) => Err(MatchmakeError::PunchError(e)),
    }
}
```

### 5.2 Responder Side (Bob)

Bob receives a MatchmakeRequest via a CIRCUIT_DATA on a circuit that was
opened to him. The daemon registers a handler for incoming matchmake messages.

```rust
pub async fn handle_incoming_matchmake(
    state: &AppState,
    circuit_id: u64,
    request: MatchmakeRequest,
) -> Result<(), MatchmakeError> {
    // 1. Check if we want to accept (basic validation)
    //    - Is the request well-formed?
    //    - Do we have NAT info to share?

    // 2. Get our own endpoint info (fresh STUN if needed)
    let our_info = gather_endpoint_info(state).await?;

    // 3. Send MatchmakeExchange back on the same circuit
    send_matchmake_exchange(state, circuit_id, &our_info).await?;

    // 4. Configure WG peer from the request info
    let punch_config = build_punch_config_from_request(&request, &our_info);

    // 5. Begin hole punch attempt
    //    Bob starts punching immediately after sending his exchange.
    //    Alice will start when she receives it.
    tokio::spawn(async move {
        let result = punch::run_punch(punch_config).await;
        match result {
            PunchResult::Success { endpoint, .. } => {
                tracing::info!("matchmake: connected to {} via {}", endpoint, request.wg_pubkey_short());
            }
            PunchResult::Timeout { .. } => {
                tracing::warn!("matchmake: punch failed after exchange with {}", request.wg_pubkey_short());
            }
            _ => {}
        }
    });

    Ok(())
}
```

### 5.3 Relay Peer (Carol)

Carol does **nothing special** for matchmaking. The existing relay capability
handles everything:
- CIRCUIT_OPEN arrives, Carol checks her circuit capacity, forwards to target
- CIRCUIT_DATA arrives, Carol forwards to the other end of the circuit
- CIRCUIT_CLOSE arrives, Carol tears down the circuit

Carol doesn't parse, inspect, or understand the matchmake messages. She's a
dumb pipe for the duration of the circuit (< 1 second of actual relay work).

The only Carol-side concern is the `allow_relay` config gate. If relay is
disabled, the p2pcd relay capability shouldn't be in her active set, so the
circuit open will fail at the p2pcd negotiation level. No daemon-level check
needed — the capability negotiation handles it.

### 5.4 Relay Discovery

```rust
/// Find a relay peer that both we and the target share.
///
/// `their_candidates` comes from the invite token — pubkeys of peers the
/// inviter is connected to that have relay capability.
///
/// We check which of those we're also connected to. Any overlap is a
/// potential relay.
async fn find_mutual_relay(
    state: &AppState,
    their_candidates: &[String],
) -> Result<String, MatchmakeError> {
    let our_peers: HashSet<String> = state.peers.read().await
        .keys()
        .cloned()
        .collect();

    // Prefer relay peers that have the capability active
    for candidate in their_candidates {
        if our_peers.contains(candidate) {
            // Verify the peer actually has relay capability active
            if is_relay_capable(state, candidate).await {
                return Ok(candidate.clone());
            }
        }
    }

    Err(MatchmakeError::NoMutualRelay)
}
```

---

## 6. Integration Points

### 6.1 Connection Flow (Tier Ladder)

The full connection attempt, integrating matchmaking as Tier 3:

```rust
// In the invite redemption / connection flow:

// Tier 1: Direct connection (IPv6 or public endpoint)
match try_direct_connect(&invite).await {
    Ok(()) => return Ok(ConnectionResult::Direct),
    Err(_) => {} // fall through
}

// Tier 2: Hole punch (two-way exchange)
if invite.their_nat_type.is_some() || needs_two_way(&our_nat) {
    match try_hole_punch(&invite, &our_nat).await {
        Ok(()) => return Ok(ConnectionResult::Punched),
        Err(PunchError::Timeout) => {} // fall through to Tier 3
        Err(e) => return Err(e),
    }
}

// Tier 3: Matchmake relay
if !invite.their_relay_candidates.is_empty() {
    match initiate_matchmake(state, &invite, &our_info).await {
        Ok(MatchmakeResult::Connected) => return Ok(ConnectionResult::Matchmade),
        Ok(MatchmakeResult::PunchFailed) => {} // fall through to UNREACHABLE
        Err(MatchmakeError::NoMutualRelay) => {} // fall through
        Err(e) => {
            tracing::warn!("matchmake failed: {}", e);
        }
    }
}

// UNREACHABLE
Err(ConnectionError::Unreachable {
    tiers_attempted: attempted_tiers,
    our_nat: our_nat.nat_type,
    their_nat: invite.their_nat_type,
    suggestion: unreachable_suggestion(&our_nat, &invite),
})
```

### 6.2 Incoming Matchmake Handler Registration

At daemon startup, register a callback for incoming circuit data that
contains matchmake messages:

```rust
// In daemon init, after p2pcd engine setup:
if let Some(engine) = &state.p2pcd_engine {
    let state_clone = state.clone();
    engine.on_circuit_data(move |circuit_id, data, ctx| {
        let state = state_clone.clone();
        async move {
            if let Ok(msg) = decode_matchmake_message(&data) {
                match msg {
                    MatchmakeMessage::Request(req) => {
                        handle_incoming_matchmake(&state, circuit_id, req).await
                    }
                    MatchmakeMessage::Exchange(exch) => {
                        // This is handled by the initiator's await loop
                        // Route to the pending matchmake future
                        route_exchange_to_pending(&state, circuit_id, exch).await
                    }
                }
            }
        }
    });
}
```

### 6.3 API Endpoints

New endpoints for the UI to monitor matchmaking state:

```
GET  /network/matchmake/status    → current matchmake attempts (if any)
POST /network/matchmake/cancel    → abort an in-progress matchmake
```

These are informational — the UI doesn't drive the matchmake flow. It happens
automatically as part of the tier ladder during invite redemption.

The existing `GET /network/status` response already includes relay info. Add
one field:

```json
{
    "relay": {
        "allow_relay": true,
        "relay_capable_peers": 3,
        "active_matchmakes": 0       // NEW — in-progress matchmake circuits
    }
}
```

### 6.4 Config

Already exists — no changes needed:

```rust
// config.rs
pub allow_relay: bool,   // default false

// state.rs
pub allow_relay: Arc<RwLock<bool>>,

// API: PUT /network/relay
```

The only config consideration: `allow_relay` gates whether this node
**acts as a relay** for others. It does NOT gate whether this node
**uses** relays. Any node can initiate a matchmake through a willing relay.

---

## 7. Types

```rust
// node/daemon/src/matchmake.rs

use crate::stun::NatType;

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

/// A decoded matchmake request (Alice's info arriving at Bob).
#[derive(Debug, Clone)]
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

/// A decoded matchmake exchange (Bob's response to Alice).
#[derive(Debug, Clone)]
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

#[derive(Debug, thiserror::Error)]
pub enum MatchmakeError {
    #[error("no mutual relay peer found")]
    NoMutualRelay,
    #[error("relay circuit failed: {0}")]
    CircuitFailed(String),
    #[error("matchmake exchange timed out")]
    ExchangeTimeout,
    #[error("punch failed after exchange: {0}")]
    PunchError(String),
    #[error("invalid matchmake message: {0}")]
    InvalidMessage(String),
}

/// Wrapper enum for dispatching incoming circuit data.
pub enum MatchmakeMessage {
    Request(MatchmakeRequest),
    Exchange(MatchmakeExchangeMsg),
}
```

---

## 8. CBOR Encoding

Use the same `cbor_helpers` pattern from p2pcd. Small, flat CBOR maps.

```rust
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
```

Request message is ~200 bytes. Exchange is ~150 bytes. Both fit comfortably
in a single CIRCUIT_DATA payload.

---

## 9. Timing & Timeouts

| Event | Timeout | Notes |
|---|---|---|
| Circuit open through relay | 5s | If relay is unreachable, fail fast |
| MatchmakeExchange response | 30s | Bob needs time to STUN + respond |
| WG punch after exchange | 15s | Same as Tier 2 timeout |
| Total Tier 3 attempt | 50s | Sum of above with margin |

### Timing Sequence

```
t=0     Alice opens circuit through Carol to Bob
t<1     Circuit established (all 3 peers already connected)
t<1     Alice sends MatchmakeRequest
t<2     Bob receives, does fresh STUN binding
t<3     Bob sends MatchmakeExchange
t<3     Both configure WG peers
t<3     Both begin endpoint rotation (punch)
t<18    Either: WG handshake succeeds → Connected
        Or: 15s punch timeout → PunchFailed → UNREACHABLE
```

Best case: connected in ~5 seconds. Worst case (punch fails): ~20 seconds
before UNREACHABLE. The circuit through Carol is active for < 5 seconds.

---

## 10. UX Notifications

### 10.1 Relay Requested (Carol's side)

When a circuit opens through Carol for matchmaking AND `allow_relay` is true:

```
Silently handle. Log at info level:
"relay: matchmake circuit opened — [alice_short] ↔ [bob_short]"
```

No UI notification for accepted relays — it's expected behavior.

### 10.2 Relay Denied (Carol's side)

When `allow_relay` is false, the relay capability isn't in Carol's active set,
so the circuit open fails at the p2pcd level. No matchmake-specific handling.

But if Carol has relay capability active but later toggled `allow_relay` off
mid-session (race condition): the circuit still goes through because capability
negotiation happened at session start. This is acceptable — the relay is
signaling only and ephemeral.

### 10.3 Matchmake In Progress (Alice/Bob's side)

The connection progress UI (Phase 5) shows:

```
Tier 1: Direct       ✗ (no public inbound)
Tier 2: Hole Punch   ✗ (timed out after 15s)
Tier 3: Relay Signal  ◌ (relaying through Carol...)
```

Then either:
```
Tier 3: Relay Signal  ✓ (connected via relay-assisted punch)
```
Or:
```
Tier 3: Relay Signal  ✗ (punch failed after relay exchange)
→ UNREACHABLE: Both peers behind symmetric NAT. No direct path possible.
```

---

## 11. Edge Cases

### 11.1 Multiple Relay Candidates

If the invite contains multiple relay candidates, try them in order. If the
first circuit fails, try the next. Don't try them in parallel — the relay
load should be minimal and sequential is simpler.

### 11.2 Relay Peer Goes Offline Mid-Exchange

If Carol disconnects after the circuit opens but before the exchange completes,
the circuit fails and both peers get an error. The initiator can retry with the
next relay candidate.

### 11.3 Bob Doesn't Have p2pcd Running

If Bob's node doesn't have the p2pcd engine active, the circuit open will fail
because there's no peer session to route to. Fall through to UNREACHABLE.

### 11.4 Stale Relay Candidates

The invite was created hours ago. The relay candidates listed might no longer
be connected to the inviter. The circuit open will fail — fall through to
UNREACHABLE with a message: "Relay peers from the invite are no longer
reachable. Ask your peer to create a fresh invite."

### 11.5 Symmetric + Symmetric

Even after relay-assisted exchange, symmetric + symmetric peers have
unpredictable port mappings. The punch will almost certainly fail. The relay
exchange is still attempted (it's fast and cheap), but UNREACHABLE is the
expected outcome. The error message should be specific:

"Both you and your peer are behind symmetric NAT. Even with relay assistance,
direct connection isn't possible. Enable IPv6, set up port forwarding, or
connect from a different network."

### 11.6 Self-Relay

If Alice and Bob both list Carol as a relay candidate, and Carol is trying to
connect to Dave who lists Alice as a relay candidate — no circular dependency.
Each matchmake is an independent circuit. The relay capability's circuit
limits (`RELAY_MAX_CIRCUITS`) prevent resource exhaustion.

---

## 12. Implementation Checklist

### 12.1 relay.rs Fixes (~75 lines, p2pcd crate)

- [ ] Add `EndpointCircuit` struct and `EndpointRole` enum
- [ ] Add `endpoint_circuits`, `pending_initiations`, `data_callback` fields to `RelayHandler`
- [ ] Add `CircuitEvent` enum (Opened, Data, Closed)
- [ ] Fix `handle_open`: detect acceptance (STATUS field, no TARGET_PEER),
      forward to initiator when we're the relay
- [ ] Fix `handle_open`: on acceptance at initiator, look up pending_initiations,
      store EndpointCircuit, fire Opened callback
- [ ] Fix `handle_open`: in is_forwarded branch (target), store EndpointCircuit,
      fire Opened callback
- [ ] Fix `handle_data`: add endpoint path — check endpoint_circuits before
      "unknown circuit" drop, fire Data callback
- [ ] Fix `handle_close`: add endpoint cleanup, fire Closed callback
- [ ] Fix `on_deactivated`: clean up endpoint_circuits for disconnected peer
- [ ] Add `set_data_callback()`, `initiate_circuit()`, `send_circuit_data()`,
      `close_circuit()` public methods
- [ ] Update existing relay tests to not break
- [ ] Add endpoint-side circuit tests (open → accept → data → close)

### 12.2 invite.rs Changes (~50 lines)

- [ ] Add `nat_type`, `stride`, `relay_csv` fields to `generate()` signature
- [ ] Encode new fields as trailing `|`-separated values (v3 format)
- [ ] Add `their_nat_type`, `their_stride`, `their_relay_candidates` to `DecodedInvite`
- [ ] Parse new fields in `decode()` with backward-compatible defaults
- [ ] Update tests for v3 format + v2 backward compat

### 12.3 matchmake.rs — New File (~300 lines)

- [ ] `EndpointInfo`, `MatchmakeRequest`, `MatchmakeExchangeMsg` types
- [ ] `MatchmakeResult`, `MatchmakeError` enums
- [ ] `encode_request()` / `decode_request()` — CBOR encode/decode
- [ ] `encode_exchange()` / `decode_exchange()` — CBOR encode/decode
- [ ] `find_mutual_relay()` — intersect invite candidates with our peers
- [ ] `gather_endpoint_info()` — collect our STUN + IPv6 + WG info
- [ ] `initiate_matchmake()` — full initiator flow (open circuit → send
      request → await exchange → close → punch)
- [ ] `handle_incoming_matchmake()` — responder flow (receive request →
      gather info → send exchange → punch)
- [ ] Unit tests for encode/decode roundtrip
- [ ] Unit tests for find_mutual_relay logic

### 12.4 Connection Flow Integration (~50 lines)

- [ ] Wire `initiate_matchmake()` into the tier ladder after Tier 2 timeout
- [ ] Register incoming matchmake handler on daemon startup (set_data_callback)
- [ ] Pass `relay_candidates` through from invite decode to connection flow
- [ ] Collect `nat_profile` and `relay_candidates` at invite creation time

### 12.5 API Updates (~30 lines)

- [ ] Add `active_matchmakes` to `GET /network/status` relay section
- [ ] Add `GET /network/matchmake/status` endpoint (optional, for UI polish)
- [ ] Update `count_relay_capable_peers()` — already exists, may need no change

### 12.6 Tests (~150 lines)

- [ ] Relay endpoint circuit lifecycle (open → accept → data roundtrip → close)
- [ ] Relay acceptance forwarding (Carol correctly routes acceptance to Alice)
- [ ] Invite v3 roundtrip (encode → decode with all new fields)
- [ ] Invite v2 backward compat (decode old format → new fields default)
- [ ] MatchmakeRequest CBOR roundtrip
- [ ] MatchmakeExchange CBOR roundtrip
- [ ] find_mutual_relay with overlap
- [ ] find_mutual_relay with no overlap → NoMutualRelay error
- [ ] Build punch config from matchmake exchange

### Total Estimate: ~655 lines

- relay.rs fixes: ~75 lines (endpoint state, callback, acceptance forwarding)
- matchmake.rs: ~300 lines (types + encode/decode + orchestration)
- invite.rs changes: ~50 lines
- connection flow: ~50 lines
- API: ~30 lines
- tests: ~150 lines

---

## 13. What This Does NOT Include

| Feature | Why not |
|---|---|
| Traffic relay / bandwidth forwarding | Signaling only. Direct WG or nothing. |
| Automatic relay discovery via gossip | Explicit list in invite token is simpler. |
| Relay reputation / scoring | Premature. Any willing peer works. |
| Persistent relay connections | Circuit lives for seconds. No keep-alive. |
| Relay-through-relay (multi-hop) | One hop max. If no direct mutual peer, UNREACHABLE. |
| Relay consent prompt (interactive) | Notification only (Phase 5). Config toggle is sufficient. |
| QR code for matchmake tokens | The whole point is it's automatic once the invite is redeemed. |

---

## 14. Dependency on Existing Code

| Component | Status | What we use |
|---|---|---|
| `stun.rs` (600 lines) | ✅ Done | `NatType`, `NatProfile`, `refresh_mapping()` |
| `punch.rs` (411 lines) | ✅ Done | `PunchConfig`, `run_punch()`, `build_candidates()` |
| `accept.rs` (238 lines) | ✅ Done | Reference for CBOR encoding pattern |
| `net_detect.rs` (268 lines) | ✅ Done | `detect_ipv6_guas()` |
| `relay.rs` (1062 lines, p2pcd) | 🔧 Needs fixes | Relay path works; endpoint path needs ~75 lines (Section 2.1) |
| `connection_routes.rs` | ✅ Done | `count_relay_capable_peers()`, relay toggle API |
| `config.rs` / `state.rs` | ✅ Done | `allow_relay` config + runtime toggle |

The relay.rs fixes (Section 2.1) complete the circuit capability so endpoint
peers can actually send/receive data through a relay. This is a prerequisite
for matchmaking but also makes relay circuits usable for any future feature
that needs peer-to-peer data through an intermediary.

---

## 15. P2PCD Conformance Verdict

**No conformance impact.** Here's why:

- **No new message types.** We use the existing CIRCUIT_OPEN (13),
  CIRCUIT_DATA (14), CIRCUIT_CLOSE (15). No new type codes allocated.

- **No wire format changes.** The CBOR payloads for CIRCUIT_OPEN/DATA/CLOSE
  use the same keys and structure. The acceptance response (`{circuit_id,
  status}`) was already defined in the original design — we're just making
  sure it gets forwarded correctly.

- **No new capability.** Still `core.network.relay.1`. The capability name,
  scope keys (RELAY_MAX_CIRCUITS, RELAY_MAX_BANDWIDTH_KBPS, RELAY_TTL), and
  negotiation are unchanged.

- **No scope param changes.** Same defaults, same keys.

- **Backward compatible.** A node running the old relay.rs can still relay
  for nodes running the new one. Carol's forwarding path is untouched. The
  only difference is that endpoint nodes now actually process what they
  receive instead of silently dropping it. An old-code endpoint paired with
  a new-code endpoint just means the old side drops data — same as today.

The changes are purely **implementation fixes** — the relay capability was
designed to support endpoint-to-endpoint circuits, the code just didn't
implement the endpoint side. We're completing the implementation, not
changing the protocol.

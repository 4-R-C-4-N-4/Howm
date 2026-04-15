// p2pcd-types: CBOR integer map key constants and wire message type constants.
// Extracted from lib.rs — all items remain accessible at p2pcd_types::<item>.

// ─── CBOR integer map key constants ──────────────────────────────────────────

/// CBOR integer map keys for discovery_manifest (§5.3)
pub mod manifest_keys {
    pub const PROTOCOL_VERSION: u64 = 1;
    pub const PEER_ID: u64 = 2;
    pub const SEQUENCE_NUM: u64 = 3;
    pub const CAPABILITIES: u64 = 4;
    pub const PERSONAL_HASH: u64 = 5;
    pub const HASH_ALGORITHM: u64 = 6;
}

/// CBOR integer map keys for capability_declaration (§5.3)
pub mod capability_keys {
    pub const NAME: u64 = 1;
    pub const ROLE: u64 = 2;
    pub const MUTUAL: u64 = 3;
    pub const CLASSIFICATION: u64 = 4; // omitted from wire per spec
    pub const SCOPE: u64 = 5;
    pub const APPLICABLE_SCOPE_KEYS: u64 = 6;
}

/// CBOR integer map keys for scope_params (§5.3)
pub mod scope_keys {
    pub const RATE_LIMIT: u64 = 1;
    pub const TTL: u64 = 2;
    // Core capability-specific params (keys 3-23, reserved per v0.4 spec)
    pub const HEARTBEAT_INTERVAL_MS: u64 = 3;
    pub const HEARTBEAT_TIMEOUT_MS: u64 = 4;
    pub const TIMESYNC_PRECISION_MS: u64 = 5;
    pub const LATENCY_SAMPLE_INTERVAL_MS: u64 = 6;
    pub const LATENCY_WINDOW_SIZE: u64 = 7;
    pub const ENDPOINT_INCLUDE_GEO: u64 = 8;
    pub const RELAY_MAX_CIRCUITS: u64 = 9;
    pub const RELAY_MAX_BANDWIDTH_KBPS: u64 = 10;
    pub const RELAY_TTL: u64 = 11;
    pub const PEX_MAX_PEERS: u64 = 12;
    pub const PEX_INCLUDE_CAPABILITIES: u64 = 13;
    pub const STREAM_BITRATE_KBPS: u64 = 14;
    pub const STREAM_CODEC: u64 = 15;
    pub const BLOB_MAX_BYTES: u64 = 16;
    pub const BLOB_CHUNK_SIZE: u64 = 17;
    pub const BLOB_HASH_ALGORITHM: u64 = 18;
    pub const RPC_MAX_REQUEST_BYTES: u64 = 19;
    pub const RPC_MAX_RESPONSE_BYTES: u64 = 20;
    pub const RPC_METHODS: u64 = 21;
    pub const EVENT_TOPICS: u64 = 22;
    pub const EVENT_MAX_PAYLOAD_BYTES: u64 = 23;
    // core.data.stream.1 (keys 24-26)
    pub const STREAM_MAX_CONCURRENT: u64 = 24;
    pub const STREAM_MAX_FRAME_BYTES: u64 = 25;
    pub const STREAM_TIMEOUT_SECS: u64 = 26;
}

/// CBOR integer map keys for protocol messages (outer envelope)
pub mod message_keys {
    pub const MESSAGE_TYPE: u64 = 1;
    pub const MANIFEST: u64 = 2; // for OFFER
    pub const PERSONAL_HASH: u64 = 3; // for CONFIRM and CLOSE
    pub const ACTIVE_SET: u64 = 4; // for CONFIRM
    pub const ACCEPTED_PARAMS: u64 = 5; // for CONFIRM
    pub const REASON: u64 = 6; // for CLOSE
    pub const TIMESTAMP: u64 = 7; // for PING/PONG
    /// Opaque payload bytes for CapabilityMsg (msg_type 4+ application messages).
    /// Stored as `Value::Bytes` so the inner CBOR is never merged into the outer
    /// envelope — that merge previously dropped any inner key that collided
    /// with MESSAGE_TYPE (= 1), e.g. RpcHandler's `method` field.
    pub const PAYLOAD: u64 = 8;
}

// ─── Wire message types (§5.3.6 + Appendix B.12) ────────────────────────────
// These are the top-level constants for the message_types module.
// Accessible as p2pcd_types::message_types::OFFER etc.

// Protocol core (1-3)
pub const OFFER: u64 = 1;
pub const CONFIRM: u64 = 2;
pub const CLOSE: u64 = 3;
// core.session.heartbeat.1 (4-5)
pub const PING: u64 = 4;
pub const PONG: u64 = 5;
// core.session.attest.1 (6)
pub const BUILD_ATTEST: u64 = 6;
// core.session.timesync.1 (7-8)
pub const TIME_REQ: u64 = 7;
pub const TIME_RESP: u64 = 8;
// core.session.latency.1 (9-10)
pub const LAT_PING: u64 = 9;
pub const LAT_PONG: u64 = 10;
// core.network.endpoint.1 (11-12)
pub const WHOAMI_REQ: u64 = 11;
pub const WHOAMI_RESP: u64 = 12;
// core.network.relay.1 (13-15)
pub const CIRCUIT_OPEN: u64 = 13;
pub const CIRCUIT_DATA: u64 = 14;
pub const CIRCUIT_CLOSE: u64 = 15;
// core.network.peerexchange.1 (16-17)
pub const PEX_REQ: u64 = 16;
pub const PEX_RESP: u64 = 17;
// core.data.blob.1 (18-21)
pub const BLOB_REQ: u64 = 18;
pub const BLOB_OFFER: u64 = 19;
pub const BLOB_CHUNK: u64 = 20;
pub const BLOB_ACK: u64 = 21;
// core.data.rpc.1 (22-23)
pub const RPC_REQ: u64 = 22;
pub const RPC_RESP: u64 = 23;
// core.data.event.1 (24-26)
pub const EVENT_SUB: u64 = 24;
pub const EVENT_UNSUB: u64 = 25;
pub const EVENT_MSG: u64 = 26;
// core.data.stream.1 (27-30)
pub const STREAM_OPEN: u64 = 27;
pub const STREAM_DATA: u64 = 28;
pub const STREAM_CLOSE: u64 = 29;
pub const STREAM_CONTROL: u64 = 30;
// 31-35: reserved for v2 core extensions
// 36+: application-defined

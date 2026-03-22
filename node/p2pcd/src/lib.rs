// P2P-CD protocol library — Phase 3 extraction
//
// This crate contains the reusable, transport-agnostic P2P Capability Discovery
// protocol implementation:
//
//   transport    — TCP length-prefixed CBOR framing
//   session      — Session state machine + OFFER/CONFIRM exchange
//   heartbeat    — PING/PONG liveness monitoring
//   capabilities — Core capability handler router + 10 built-in handlers
//
// The daemon-specific engine (ProtocolEngine), WireGuard monitor integration,
// and HTTP capability notifier remain in the daemon crate.

pub mod blob_store;
#[cfg(feature = "bridge-client")]
pub mod bridge_client;
pub mod capabilities;
pub mod cbor_helpers;
pub mod heartbeat;
pub mod mux;
pub mod session;
pub mod transport;

// Re-export key types for convenience
pub use capabilities::CapabilityRouter;
pub use heartbeat::{HeartbeatEvent, HeartbeatManager};
pub use session::{Session, SessionState};
pub use transport::{connect, P2pcdListener, P2pcdTransport};

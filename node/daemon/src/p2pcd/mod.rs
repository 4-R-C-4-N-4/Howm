// P2P-CD protocol engine — Phase 3
//
// Core protocol types (transport, session, heartbeat, capabilities) are now in
// the `p2pcd` crate. This module keeps daemon-specific wiring:
//   engine       — ProtocolEngine (ties sessions to WgPeerMonitor + notifier)
//   cap_notify   — HTTP capability notification interface

pub mod bridge;
pub mod cap_notify;
pub mod engine;

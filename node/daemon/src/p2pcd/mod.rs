// P2P-CD protocol engine — Phase 2+
//
// Module layout:
//   transport  — TCP length-prefixed CBOR framing (Task 2.1)
//   session    — Session state machine + OFFER/CONFIRM exchange (Tasks 2.2, 2.3)
//   engine     — Protocol engine coordinator (Task 3.1)
//   heartbeat  — PING/PONG liveness (Task 4.1)
//   cap_notify — Capability notification interface (Task 6.2)

pub mod transport;
pub mod session;

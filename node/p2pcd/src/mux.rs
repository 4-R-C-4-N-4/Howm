// Session message multiplexer — splits a single transport into:
//   1. Heartbeat channel (PING/PONG only)
//   2. Capability channel (msg types 6+ → cap_router dispatch)
//
// Also provides a shared `send_tx` that any component can use to send
// outbound messages through the transport.
//
// This replaces the old pattern where HeartbeatManager exclusively owned
// the transport channels.

use tokio::sync::mpsc;

use p2pcd_types::ProtocolMessage;

/// Shared outbound sender — cloneable, any component can send messages.
pub type SharedSender = mpsc::Sender<ProtocolMessage>;

/// Handles for the multiplexed session channels.
pub struct SessionMux {
    /// Shared outbound sender — clone this for heartbeat, cap handlers, bridge, etc.
    pub send_tx: SharedSender,
    /// Receives only PING/PONG messages (for HeartbeatManager).
    pub heartbeat_rx: mpsc::Receiver<ProtocolMessage>,
    /// Receives CapabilityMsg (msg_type 6+) for dispatch to cap_router.
    pub capability_rx: mpsc::Receiver<ProtocolMessage>,
    /// Handle for the mux task (abort on session teardown).
    pub mux_handle: tokio::task::JoinHandle<()>,
}

/// Build a session multiplexer from a transport's raw channel pair.
///
/// `transport_send_tx` — the channel that writes to the TCP stream.
/// `transport_recv_rx` — the channel that reads from the TCP stream.
pub fn build_session_mux(
    transport_send_tx: mpsc::Sender<ProtocolMessage>,
    mut transport_recv_rx: mpsc::Receiver<ProtocolMessage>,
) -> SessionMux {
    let (heartbeat_tx, heartbeat_rx) = mpsc::channel::<ProtocolMessage>(64);
    let (capability_tx, capability_rx) = mpsc::channel::<ProtocolMessage>(64);

    // The mux task reads from the transport and routes messages to the right channel.
    let mux_handle = tokio::spawn(async move {
        while let Some(msg) = transport_recv_rx.recv().await {
            match &msg {
                ProtocolMessage::Ping { .. } | ProtocolMessage::Pong { .. } => {
                    if heartbeat_tx.send(msg).await.is_err() {
                        break;
                    }
                }
                ProtocolMessage::CapabilityMsg { .. } => {
                    if capability_tx.send(msg).await.is_err() {
                        // No capability consumer — drop the message
                        tracing::debug!("mux: capability channel closed, dropping message");
                    }
                }
                // Protocol messages (OFFER/CONFIRM/CLOSE) shouldn't arrive post-session
                _ => {
                    tracing::debug!("mux: unexpected protocol message post-session, ignoring");
                }
            }
        }
    });

    SessionMux {
        send_tx: transport_send_tx,
        heartbeat_rx,
        capability_rx,
        mux_handle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn mux_routes_ping_to_heartbeat() {
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let (outbound_tx, _outbound_rx) = mpsc::channel(16);
        let mut mux = build_session_mux(outbound_tx, inbound_rx);

        // Send a PING via the inbound channel
        inbound_tx
            .send(ProtocolMessage::Ping { timestamp: 42 })
            .await
            .unwrap();

        let msg = timeout(Duration::from_millis(100), mux.heartbeat_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(matches!(msg, ProtocolMessage::Ping { timestamp: 42 }));
    }

    #[tokio::test]
    async fn mux_routes_capability_msg() {
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let (outbound_tx, _outbound_rx) = mpsc::channel(16);
        let mut mux = build_session_mux(outbound_tx, inbound_rx);

        // Send a CapabilityMsg via the inbound channel
        inbound_tx
            .send(ProtocolMessage::CapabilityMsg {
                message_type: 22,
                payload: vec![1, 2, 3],
            })
            .await
            .unwrap();

        let msg = timeout(Duration::from_millis(100), mux.capability_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(matches!(
            msg,
            ProtocolMessage::CapabilityMsg {
                message_type: 22,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn mux_send_tx_forwards_to_transport() {
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let (outbound_tx, mut outbound_rx) = mpsc::channel(16);
        let mux = build_session_mux(outbound_tx, inbound_rx);

        // Send outbound through the shared sender
        mux.send_tx
            .send(ProtocolMessage::Pong { timestamp: 99 })
            .await
            .unwrap();

        let msg = timeout(Duration::from_millis(100), outbound_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(matches!(msg, ProtocolMessage::Pong { timestamp: 99 }));

        drop(inbound_tx); // clean shutdown
    }

    #[tokio::test]
    async fn mux_handle_aborts_cleanly() {
        let (_inbound_tx, inbound_rx) = mpsc::channel(16);
        let (outbound_tx, _outbound_rx) = mpsc::channel(16);
        let mux = build_session_mux(outbound_tx, inbound_rx);

        mux.mux_handle.abort();
        // Should not panic
        let _ = mux.mux_handle.await;
    }
}

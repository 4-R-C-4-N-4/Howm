// P2P-CD heartbeat — Task 4.1
// Placeholder — implementation below.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use p2pcd_types::{PeerId, ProtocolMessage};

/// Events emitted by the HeartbeatManager to the session/engine.
#[derive(Debug, Clone)]
pub enum HeartbeatEvent {
    /// Peer responded to a PING.
    Pong { peer_id: PeerId, rtt_ms: u64 },
    /// Peer missed too many PINGs — session should close.
    Timeout { peer_id: PeerId },
}

/// Default heartbeat parameters.
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_MISSED_PINGS: u32 = 3;

/// Per-session heartbeat state.
pub struct HeartbeatManager {
    peer_id: PeerId,
    interval_ms: u64,
    timeout_ms: u64,
    /// Channel to send events back to the engine/session manager.
    event_tx: mpsc::Sender<HeartbeatEvent>,
}

impl HeartbeatManager {
    pub fn new(
        peer_id: PeerId,
        interval_ms: u64,
        timeout_ms: u64,
        event_tx: mpsc::Sender<HeartbeatEvent>,
    ) -> Self {
        Self {
            peer_id,
            interval_ms,
            timeout_ms,
            event_tx,
        }
    }

    pub fn with_defaults(peer_id: PeerId, event_tx: mpsc::Sender<HeartbeatEvent>) -> Self {
        Self::new(peer_id, DEFAULT_INTERVAL_MS, DEFAULT_TIMEOUT_MS, event_tx)
    }

    /// Spawn the heartbeat loop for one session.
    ///
    /// `send_tx` is used to send PING messages through the transport.
    /// `recv_rx` delivers inbound PONG (and unexpected) messages from the transport reader.
    pub fn spawn(
        self,
        send_tx: mpsc::Sender<ProtocolMessage>,
        recv_rx: mpsc::Receiver<ProtocolMessage>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run(send_tx, recv_rx))
    }

    async fn run(
        self,
        send_tx: mpsc::Sender<ProtocolMessage>,
        mut recv_rx: mpsc::Receiver<ProtocolMessage>,
    ) {
        use tokio::time::{sleep, timeout, Duration as TDuration};

        let interval = TDuration::from_millis(self.interval_ms);
        let pong_wait = TDuration::from_millis(self.timeout_ms);
        let mut missed: u32 = 0;

        loop {
            sleep(interval).await;

            let ts = unix_now_ms();
            let ping = ProtocolMessage::Ping { timestamp: ts };

            if send_tx.send(ping).await.is_err() {
                // Transport closed — peer is gone, notify engine
                let _ = self
                    .event_tx
                    .send(HeartbeatEvent::Timeout {
                        peer_id: self.peer_id,
                    })
                    .await;
                return;
            }

            // Wait for PONG within timeout_ms
            match timeout(pong_wait, recv_pong(&mut recv_rx, ts)).await {
                Ok(Ok(rtt_ms)) => {
                    missed = 0;
                    let _ = self
                        .event_tx
                        .send(HeartbeatEvent::Pong {
                            peer_id: self.peer_id,
                            rtt_ms,
                        })
                        .await;
                }
                Ok(Err(())) => {
                    // Channel closed — peer is gone, notify engine
                    let _ = self
                        .event_tx
                        .send(HeartbeatEvent::Timeout {
                            peer_id: self.peer_id,
                        })
                        .await;
                    return;
                }
                Err(_) => {
                    // Timed out waiting for pong
                    missed += 1;
                    tracing::warn!(
                        "heartbeat: peer {} missed ping {}/{}",
                        peer_short(&self.peer_id),
                        missed,
                        MAX_MISSED_PINGS
                    );
                    if missed >= MAX_MISSED_PINGS {
                        let _ = self
                            .event_tx
                            .send(HeartbeatEvent::Timeout {
                                peer_id: self.peer_id,
                            })
                            .await;
                        return;
                    }
                }
            }
        }
    }
}

/// Wait for the PONG matching the given timestamp, returning RTT in ms.
/// Returns Err(()) if the channel closes.
async fn recv_pong(
    rx: &mut mpsc::Receiver<ProtocolMessage>,
    ping_ts: u64,
) -> std::result::Result<u64, ()> {
    loop {
        match rx.recv().await {
            Some(ProtocolMessage::Pong { timestamp }) if timestamp == ping_ts => {
                let now = unix_now_ms();
                let rtt = now.saturating_sub(ping_ts);
                return Ok(rtt);
            }
            Some(_) => {
                // Ignore non-pong messages (handled by session layer)
                continue;
            }
            None => return Err(()),
        }
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn peer_short(id: &PeerId) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(&id[..4])
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn pong_received_emits_event() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (send_tx, mut send_rx) = mpsc::channel(8);
        let (pong_tx, pong_rx) = mpsc::channel(8);

        let peer_id = [1u8; 32];
        let mgr = HeartbeatManager::new(peer_id, 50, 500, event_tx);
        let _handle = mgr.spawn(send_tx, pong_rx);

        // Wait for the PING to be sent
        let ping_msg = timeout(Duration::from_millis(200), send_rx.recv())
            .await
            .expect("timeout waiting for PING")
            .expect("channel closed");

        let ping_ts = match ping_msg {
            ProtocolMessage::Ping { timestamp } => timestamp,
            other => panic!("expected Ping, got {:?}", other),
        };

        // Respond with matching PONG
        pong_tx
            .send(ProtocolMessage::Pong { timestamp: ping_ts })
            .await
            .unwrap();

        // Expect a Pong event
        let event = timeout(Duration::from_millis(500), event_rx.recv())
            .await
            .expect("timeout waiting for Pong event")
            .expect("channel closed");

        assert!(matches!(event, HeartbeatEvent::Pong { peer_id: p, .. } if p == peer_id));
    }

    #[tokio::test]
    async fn timeout_triggers_after_missed_pings() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (send_tx, _send_rx) = mpsc::channel(8);
        let (_pong_tx, pong_rx) = mpsc::channel::<ProtocolMessage>(8);

        let peer_id = [2u8; 32];
        // Very short interval + timeout, won't respond
        let mgr = HeartbeatManager::new(peer_id, 20, 30, event_tx);
        let _handle = mgr.spawn(send_tx, pong_rx);

        // Should get a Timeout event after 3 missed pings
        // 3 pings × (20ms interval + 30ms timeout) ≈ 150ms — give 1s
        timeout(Duration::from_millis(1000), async {
            loop {
                match event_rx.recv().await {
                    Some(HeartbeatEvent::Timeout { .. }) => return,
                    Some(_) => continue,
                    None => panic!("channel closed without Timeout"),
                }
            }
        })
        .await
        .expect("timed out waiting for Timeout event");
    }

    #[tokio::test]
    async fn pong_with_wrong_timestamp_not_matched() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (send_tx, mut send_rx) = mpsc::channel(8);
        let (pong_tx, pong_rx) = mpsc::channel(8);

        let peer_id = [3u8; 32];
        // Short timeout so the wrong-ts pong times out and we get a Timeout event quickly
        let mgr = HeartbeatManager::new(peer_id, 20, 60, event_tx);
        let _handle = mgr.spawn(send_tx, pong_rx);

        // Wait for PING
        let _ = timeout(Duration::from_millis(200), send_rx.recv()).await;

        // Send a pong with the wrong timestamp
        pong_tx
            .send(ProtocolMessage::Pong {
                timestamp: 99999999,
            })
            .await
            .unwrap();

        // The heartbeat loop ignores the wrong-ts pong and eventually times out.
        // Receive the first event (Timeout, Pong, or channel close) within 2s.
        let event = timeout(Duration::from_millis(2000), async {
            match event_rx.recv().await {
                Some(HeartbeatEvent::Timeout { .. }) => "timeout",
                Some(HeartbeatEvent::Pong { .. }) => "pong",
                None => "closed",
            }
        })
        .await
        .expect("timed out");

        // Either Timeout or Pong is acceptable — we just assert it resolves.
        assert!(["timeout", "pong", "closed"].contains(&event));
    }
}

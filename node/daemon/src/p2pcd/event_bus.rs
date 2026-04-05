// P2P-CD EventBus — in-process broadcast channel for peer lifecycle events.
//
// All peer-session lifecycle notifications (active/inactive) and inbound
// capability messages are published here in addition to the existing HTTP
// fire-and-forget POST loops.  Subscribers receive a clone of every event
// published after they subscribe.

use p2pcd_types::ScopeParams;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// All events the daemon emits to capabilities over the SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CapEvent {
    PeerActive {
        peer_id: String, // base64
        wg_address: String,
        capability: String,
        scope: ScopeParams,
        active_since: u64,
    },
    PeerInactive {
        peer_id: String,
        capability: String,
        reason: String,
    },
    Inbound {
        peer_id: String,
        capability: String,
        message_type: u64,
        payload: String, // base64
    },
}

/// Thin broadcast channel wrapper. Clone cheaply — all clones share the sender.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<CapEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    pub fn publish(&self, event: CapEvent) {
        // SendError means no receivers yet — safe to ignore.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CapEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::RecvError;

    #[tokio::test]
    async fn publish_receive() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(CapEvent::PeerActive {
            peer_id: "AAAA".to_string(),
            wg_address: "100.222.0.1".to_string(),
            capability: "test.cap.1".to_string(),
            scope: ScopeParams::default(),
            active_since: 42,
        });

        let event = rx.recv().await.expect("should receive event");
        match event {
            CapEvent::PeerActive {
                peer_id,
                wg_address,
                capability,
                active_since,
                ..
            } => {
                assert_eq!(peer_id, "AAAA");
                assert_eq!(wg_address, "100.222.0.1");
                assert_eq!(capability, "test.cap.1");
                assert_eq!(active_since, 42);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn publish_no_subscribers() {
        // Should not panic when there are no subscribers.
        let bus = EventBus::new();
        bus.publish(CapEvent::PeerInactive {
            peer_id: "BBBB".to_string(),
            capability: "test.cap.1".to_string(),
            reason: "Normal".to_string(),
        });
        // If we get here without panic, the test passes.
    }

    #[tokio::test]
    async fn lagged_subscriber() {
        // The channel capacity is 1024. Publishing capacity+1 events causes the
        // subscriber to lag. The next recv() after overflow returns RecvError::Lagged.
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        // Publish capacity + 1 events to force lag.
        for i in 0..1025u64 {
            bus.publish(CapEvent::Inbound {
                peer_id: "CCCC".to_string(),
                capability: "test.cap.1".to_string(),
                message_type: i,
                payload: "AA==".to_string(),
            });
        }

        // Drain until we hit Lagged or Closed.
        let mut got_lagged = false;
        loop {
            match rx.recv().await {
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => {
                    got_lagged = true;
                    break;
                }
                Err(RecvError::Closed) => break,
            }
        }
        assert!(
            got_lagged,
            "expected RecvError::Lagged after overflowing channel"
        );
    }
}

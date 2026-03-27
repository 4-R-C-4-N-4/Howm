//! WebSocket signaling for SDP/ICE exchange between peers.
//!
//! Each room has a set of connected WebSocket clients. Messages with a `to`
//! field are routed to that specific peer. Messages without `to` are broadcast.

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

// ── Types ────────────────────────────────────────────────────────────────────

/// A signaling message exchanged over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignalMessage {
    /// Event type: sdp-offer, sdp-answer, ice-candidate, mute-changed, etc.
    #[serde(rename = "type")]
    pub msg_type: String,

    /// Source peer ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,

    /// Target peer ID. If None, the message is broadcast to all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,

    /// Peer ID (for peer-joined, peer-left, mute-changed events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,

    /// SDP payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp: Option<String>,

    /// ICE candidate payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,

    /// Muted state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,

    /// Joined timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joined_at: Option<u64>,

    /// Reason (for room-closed, error events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A connected client in a room's signaling channel.
struct ConnectedPeer {
    tx: mpsc::UnboundedSender<String>,
}

/// Manages WebSocket connections for all rooms.
#[derive(Clone, Default)]
pub struct SignalHub {
    /// room_id -> (peer_id -> sender)
    connections: Arc<RwLock<HashMap<String, HashMap<String, ConnectedPeer>>>>,
}

impl SignalHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a peer's WebSocket sender for a room.
    fn register(&self, room_id: &str, peer_id: &str, tx: mpsc::UnboundedSender<String>) {
        let mut conns = self.connections.write();
        conns
            .entry(room_id.to_string())
            .or_default()
            .insert(peer_id.to_string(), ConnectedPeer { tx });
        debug!("signal: {} connected to room {}", peer_id, room_id);
    }

    /// Unregister a peer from a room.
    fn unregister(&self, room_id: &str, peer_id: &str) {
        let mut conns = self.connections.write();
        if let Some(room_conns) = conns.get_mut(room_id) {
            room_conns.remove(peer_id);
            if room_conns.is_empty() {
                conns.remove(room_id);
            }
        }
        debug!("signal: {} disconnected from room {}", peer_id, room_id);
    }

    /// Send a message to a specific peer in a room (public interface for bridge relay).
    pub fn send_to_peer(&self, room_id: &str, peer_id: &str, msg: &str) {
        self.send_to(room_id, peer_id, msg);
    }

    /// Send a message to a specific peer in a room.
    fn send_to(&self, room_id: &str, peer_id: &str, msg: &str) {
        let conns = self.connections.read();
        if let Some(room_conns) = conns.get(room_id) {
            if let Some(peer) = room_conns.get(peer_id) {
                let _ = peer.tx.send(msg.to_string());
            }
        }
    }

    /// Broadcast a message to all peers in a room except the sender.
    fn broadcast(&self, room_id: &str, exclude_peer: &str, msg: &str) {
        let conns = self.connections.read();
        if let Some(room_conns) = conns.get(room_id) {
            for (pid, peer) in room_conns {
                if pid != exclude_peer {
                    let _ = peer.tx.send(msg.to_string());
                }
            }
        }
    }

    /// Broadcast to ALL peers in a room (including sender).
    pub fn broadcast_all(&self, room_id: &str, msg: &str) {
        let conns = self.connections.read();
        if let Some(room_conns) = conns.get(room_id) {
            for peer in room_conns.values() {
                let _ = peer.tx.send(msg.to_string());
            }
        }
    }

    /// Remove all connections for a room (used on room close).
    pub fn close_room(&self, room_id: &str) {
        let msg = serde_json::to_string(&SignalMessage {
            msg_type: "room-closed".to_string(),
            reason: Some("room closed by creator".to_string()),
            ..Default::default()
        })
        .unwrap_or_default();

        self.broadcast_all(room_id, &msg);
        self.connections.write().remove(room_id);
    }
}

// ── WebSocket handler ────────────────────────────────────────────────────────

use crate::AppState;

/// WebSocket upgrade handler for `/rooms/:room_id/signal`.
pub async fn signal_ws(
    Path(room_id): Path<String>,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_signal_ws(socket, room_id, state))
}

async fn handle_signal_ws(socket: WebSocket, room_id: String, state: AppState) {
    use futures_util::{SinkExt, StreamExt};

    let (mut ws_tx, mut ws_rx) = socket.split();

    // First message must identify the peer
    let peer_id = match ws_rx.next().await {
        Some(Ok(Message::Text(text))) => {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) {
                msg["peer_id"].as_str().unwrap_or("unknown").to_string()
            } else {
                warn!("signal: first message was not valid JSON");
                return;
            }
        }
        _ => {
            warn!("signal: connection closed before identification");
            return;
        }
    };

    // Verify peer is a member of the room
    if let Some(room) = state.rooms.get_room(&room_id) {
        if !room.members.iter().any(|m| m.peer_id == peer_id) {
            let err = serde_json::to_string(&SignalMessage {
                msg_type: "error".to_string(),
                message: Some("not a member of this room".to_string()),
                ..Default::default()
            })
            .unwrap_or_default();
            let _ = ws_tx.send(Message::Text(err.into())).await;
            return;
        }
    } else {
        let err = serde_json::to_string(&SignalMessage {
            msg_type: "error".to_string(),
            message: Some("room not found".to_string()),
            ..Default::default()
        })
        .unwrap_or_default();
        let _ = ws_tx.send(Message::Text(err.into())).await;
        return;
    }

    // Create channel for outbound messages
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    state.signal_hub.register(&room_id, &peer_id, tx);

    info!("signal: {} joined room {}", peer_id, room_id);

    // Broadcast peer-joined to others
    let joined_msg = serde_json::to_string(&SignalMessage {
        msg_type: "peer-joined".to_string(),
        peer_id: Some(peer_id.clone()),
        joined_at: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        ..Default::default()
    })
    .unwrap_or_default();
    state.signal_hub.broadcast(&room_id, &peer_id, &joined_msg);

    // Spawn outbound task: channel -> WebSocket
    let outbound = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Inbound loop: WebSocket -> route to target or broadcast
    while let Some(Ok(msg)) = ws_rx.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };

        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Route by "to" field
        if let Some(target) = parsed["to"].as_str() {
            // Inject "from" field
            let mut msg_obj = parsed.clone();
            msg_obj["from"] = serde_json::Value::String(peer_id.clone());
            let forwarded = serde_json::to_string(&msg_obj).unwrap_or_default();
            state.signal_hub.send_to(&room_id, target, &forwarded);
        } else {
            // Broadcast to all except sender
            let mut msg_obj = parsed.clone();
            msg_obj["from"] = serde_json::Value::String(peer_id.clone());
            let forwarded = serde_json::to_string(&msg_obj).unwrap_or_default();
            state.signal_hub.broadcast(&room_id, &peer_id, &forwarded);
        }
    }

    // Cleanup
    state.signal_hub.unregister(&room_id, &peer_id);
    outbound.abort();

    // Broadcast peer-left
    let left_msg = serde_json::to_string(&SignalMessage {
        msg_type: "peer-left".to_string(),
        peer_id: Some(peer_id.clone()),
        ..Default::default()
    })
    .unwrap_or_default();
    state.signal_hub.broadcast(&room_id, &peer_id, &left_msg);

    info!("signal: {} left room {}", peer_id, room_id);
}

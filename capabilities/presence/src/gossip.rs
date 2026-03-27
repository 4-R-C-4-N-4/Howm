use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use crate::peers::PeerPresence;
use crate::state::{now_secs, Activity, AppState, PresenceState};

/// Magic bytes identifying a presence gossip packet.
const MAGIC: [u8; 2] = [0x48, 0x50]; // "HP" — Howm Presence
/// Protocol version.
const VERSION: u8 = 0x01;

// ── Encode / Decode ──────────────────────────────────────────────────────────

/// Encode a presence broadcast packet: magic + version + CBOR payload.
pub fn encode_broadcast(state: &PresenceState) -> Vec<u8> {
    use ciborium::value::Value;

    let activity_str = match state.activity {
        Activity::Active => "active",
        Activity::Away => "away",
    };

    let mut entries = vec![
        (
            Value::Text("activity".into()),
            Value::Text(activity_str.into()),
        ),
        (
            Value::Text("ts".into()),
            Value::Integer(state.updated_at.into()),
        ),
    ];

    match &state.status {
        Some(s) => entries.push((Value::Text("status".into()), Value::Text(s.clone()))),
        None => entries.push((Value::Text("status".into()), Value::Null)),
    }

    match &state.emoji {
        Some(e) => entries.push((Value::Text("emoji".into()), Value::Text(e.clone()))),
        None => entries.push((Value::Text("emoji".into()), Value::Null)),
    }

    let map = Value::Map(entries);
    let mut cbor_buf = Vec::new();
    ciborium::into_writer(&map, &mut cbor_buf).expect("CBOR serialization of presence broadcast");

    let mut packet = Vec::with_capacity(3 + cbor_buf.len());
    packet.extend_from_slice(&MAGIC);
    packet.push(VERSION);
    packet.extend_from_slice(&cbor_buf);
    packet
}

/// Decode a presence broadcast packet. Returns (activity, status, emoji, timestamp).
pub fn decode_broadcast(
    data: &[u8],
) -> Result<(Activity, Option<String>, Option<String>, u64), String> {
    if data.len() < 4 {
        return Err("packet too short".into());
    }
    if data[0] != MAGIC[0] || data[1] != MAGIC[1] {
        return Err("invalid magic bytes".into());
    }
    if data[2] != VERSION {
        return Err(format!("unsupported version: {}", data[2]));
    }

    use ciborium::value::Value;
    let value: Value =
        ciborium::from_reader(&data[3..]).map_err(|e| format!("CBOR decode error: {}", e))?;

    let map = match value {
        Value::Map(m) => m,
        _ => return Err("expected CBOR map".into()),
    };

    let mut activity = None;
    let mut status = None;
    let mut emoji = None;
    let mut ts = 0u64;

    for (k, v) in map {
        let key = match k {
            Value::Text(s) => s,
            _ => continue,
        };
        match key.as_str() {
            "activity" => {
                if let Value::Text(a) = v {
                    activity = Some(match a.as_str() {
                        "active" => Activity::Active,
                        "away" => Activity::Away,
                        _ => Activity::Away,
                    });
                }
            }
            "status" => {
                if let Value::Text(s) = v {
                    status = Some(s);
                }
                // Value::Null → remains None
            }
            "emoji" => {
                if let Value::Text(e) = v {
                    emoji = Some(e);
                }
            }
            "ts" => {
                if let Value::Integer(i) = v {
                    let val: i128 = i.into();
                    ts = val as u64;
                }
            }
            _ => {}
        }
    }

    Ok((
        activity.unwrap_or(Activity::Away),
        status,
        emoji,
        ts,
    ))
}

// ── Sender ───────────────────────────────────────────────────────────────────

/// Background task: broadcasts our presence to all known peers at a fixed interval.
pub fn start_gossip_sender(state: AppState) {
    let interval = state.broadcast_interval_secs;
    let gossip_port = state.gossip_port;

    tokio::spawn(async move {
        // Bind an ephemeral UDP socket for sending
        let sock = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!("Gossip sender: failed to bind UDP socket: {}", e);
                return;
            }
        };

        info!("Gossip sender started (interval={}s)", interval);

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

            let presence = state.presence.read().await.clone();
            let packet = encode_broadcast(&presence);
            let addresses = state.peer_addresses.read().await;

            for (_peer_id, wg_addr) in addresses.iter() {
                if wg_addr.is_empty() {
                    continue;
                }
                let target: SocketAddr = match format!("{}:{}", wg_addr, gossip_port).parse() {
                    Ok(addr) => addr,
                    Err(_) => continue,
                };
                if let Err(e) = sock.send_to(&packet, target).await {
                    debug!("Gossip send to {} failed: {}", target, e);
                }
            }
        }
    });
}

/// Send an immediate gossip broadcast (e.g. on status change).
pub async fn send_immediate_broadcast(state: &AppState) {
    let presence = state.presence.read().await.clone();
    let packet = encode_broadcast(&presence);
    let addresses = state.peer_addresses.read().await;
    let gossip_port = state.gossip_port;

    if addresses.is_empty() {
        return;
    }

    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            debug!("Immediate broadcast: failed to bind: {}", e);
            return;
        }
    };

    for (_peer_id, wg_addr) in addresses.iter() {
        if wg_addr.is_empty() {
            continue;
        }
        let target: SocketAddr = match format!("{}:{}", wg_addr, gossip_port).parse() {
            Ok(addr) => addr,
            Err(_) => continue,
        };
        let _ = sock.send_to(&packet, target).await;
    }
}

// ── Receiver ─────────────────────────────────────────────────────────────────

/// Background task: listens for incoming presence broadcasts from peers.
pub fn start_gossip_receiver(state: AppState) {
    let gossip_port = state.gossip_port;

    tokio::spawn(async move {
        let bind_addr = format!("0.0.0.0:{}", gossip_port);
        let sock = match UdpSocket::bind(&bind_addr).await {
            Ok(s) => s,
            Err(e) => {
                warn!("Gossip receiver: failed to bind {}: {}", bind_addr, e);
                return;
            }
        };

        info!("Gossip receiver listening on {}", bind_addr);

        let mut buf = [0u8; 1024];
        loop {
            let (len, src) = match sock.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(e) => {
                    debug!("Gossip recv error: {}", e);
                    continue;
                }
            };

            let data = &buf[..len];
            let (activity, status, emoji, ts) = match decode_broadcast(data) {
                Ok(decoded) => decoded,
                Err(e) => {
                    debug!("Gossip decode error from {}: {}", src, e);
                    continue;
                }
            };

            // Reverse lookup: find which peer_id has this WG address
            let src_ip = src.ip().to_string();
            let peer_id = {
                let addresses = state.peer_addresses.read().await;
                addresses
                    .iter()
                    .find(|(_, addr)| **addr == src_ip)
                    .map(|(id, _)| id.clone())
            };

            let peer_id = match peer_id {
                Some(id) => id,
                None => {
                    debug!("Gossip from unknown address {}, ignoring", src_ip);
                    continue;
                }
            };

            let now = now_secs();
            let mut peers = state.peers.write().await;
            peers
                .entry(peer_id.clone())
                .and_modify(|p| {
                    p.activity = activity;
                    p.status = status.clone();
                    p.emoji = emoji.clone();
                    p.updated_at = ts;
                    p.last_broadcast_received = now;
                })
                .or_insert_with(|| PeerPresence {
                    peer_id,
                    activity,
                    status,
                    emoji,
                    updated_at: ts,
                    last_broadcast_received: now,
                });
        }
    });
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_broadcast() {
        let state = PresenceState {
            activity: Activity::Active,
            status: Some("working on music".into()),
            emoji: Some("🎵".into()),
            updated_at: 1711440000,
        };

        let packet = encode_broadcast(&state);
        assert_eq!(packet[0], 0x48);
        assert_eq!(packet[1], 0x50);
        assert_eq!(packet[2], 0x01);

        let (activity, status, emoji, ts) = decode_broadcast(&packet).unwrap();
        assert_eq!(activity, Activity::Active);
        assert_eq!(status, Some("working on music".into()));
        assert_eq!(emoji, Some("🎵".into()));
        assert_eq!(ts, 1711440000);
    }

    #[test]
    fn roundtrip_null_status() {
        let state = PresenceState {
            activity: Activity::Away,
            status: None,
            emoji: None,
            updated_at: 1711440000,
        };

        let packet = encode_broadcast(&state);
        let (activity, status, emoji, _ts) = decode_broadcast(&packet).unwrap();
        assert_eq!(activity, Activity::Away);
        assert!(status.is_none());
        assert!(emoji.is_none());
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let data = [0x00, 0x00, 0x01, 0xa0]; // bad magic
        assert!(decode_broadcast(&data).is_err());
    }

    #[test]
    fn decode_rejects_short_packet() {
        let data = [0x48, 0x50];
        assert!(decode_broadcast(&data).is_err());
    }
}

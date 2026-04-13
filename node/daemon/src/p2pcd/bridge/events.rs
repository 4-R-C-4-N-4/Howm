use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::StreamExt as _;

use super::{BridgeState, EventsQuery};

fn event_name(e: &crate::p2pcd::event_bus::CapEvent) -> &'static str {
    use crate::p2pcd::event_bus::CapEvent;
    match e {
        CapEvent::PeerActive { .. } => "peer-active",
        CapEvent::PeerInactive { .. } => "peer-inactive",
        CapEvent::Inbound { .. } => "inbound",
    }
}

/// GET /p2pcd/bridge/events?capability=<name>
///
/// Streams a capability-filtered snapshot of active sessions on connect,
/// then streams live peer-active, peer-inactive, and inbound events from
/// the EventBus indefinitely.
///
/// CRITICAL: subscribe() is called BEFORE building the snapshot so no events
/// are missed between the snapshot and the start of the live stream.
pub async fn handle_events(
    State(BridgeState {
        engine, event_bus, ..
    }): State<BridgeState>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use crate::p2pcd::event_bus::CapEvent;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Subscribe to the event bus BEFORE building the snapshot.
    // Any event that fires during snapshot construction is buffered here
    // and will be delivered to the client after the snapshot, preserving
    // strict ordering. This eliminates the startup race.
    let rx = event_bus.subscribe();

    // Build snapshot of currently active peers for this capability.
    let sessions = engine.active_sessions().await;
    let filtered: Vec<_> = sessions
        .into_iter()
        .filter(|s| {
            s.state == p2pcd::session::SessionState::Active && s.active_set.contains(&q.capability)
        })
        .collect();

    let mut snapshot_peers: Vec<serde_json::Value> = Vec::with_capacity(filtered.len());
    for s in filtered {
        let wg_addr = engine.peer_wg_ip(&s.peer_id).await;
        snapshot_peers.push(serde_json::json!({
            "peer_id":      STANDARD.encode(s.peer_id),
            "wg_address":   wg_addr,
            "active_since": s.created_at,
        }));
    }

    let snapshot_event = Event::default().event("snapshot").data(
        serde_json::to_string(&serde_json::json!({ "peers": snapshot_peers }))
            .inspect_err(|e| tracing::error!("Failed to serialize SSE snapshot: {e}"))
            .unwrap_or_default(),
    );

    // Incremental stream: filter by capability, close stream on lag so client reconnects.
    let cap_clone = q.capability.clone();
    let incremental = {
        let mut rx = rx;
        async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let matches = match &event {
                            CapEvent::PeerActive   { capability, .. } => capability == &cap_clone,
                            CapEvent::PeerInactive { capability, .. } => capability == &cap_clone,
                            CapEvent::Inbound      { capability, .. } => capability == &cap_clone,
                        };
                        if !matches { continue; }
                        let name = event_name(&event);
                        if let Ok(data) = serde_json::to_string(&event) {
                            yield Ok::<Event, std::convert::Infallible>(
                                Event::default().event(name).data(data)
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "SSE consumer for '{}' lagged, dropped {} events; closing stream so client reconnects",
                            cap_clone, n
                        );
                        break; // close stream — client's SSE reconnect will get a fresh snapshot
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    };

    let stream = futures::stream::once(futures::future::ready(
        Ok::<Event, std::convert::Infallible>(snapshot_event),
    ))
    .chain(incremental);

    Sse::new(stream).keep_alive(KeepAlive::default())
}

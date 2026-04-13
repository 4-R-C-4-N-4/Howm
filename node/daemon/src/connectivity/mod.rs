pub mod lan_discovery;
pub mod matchmake;
pub mod net_detect;
pub mod punch;
pub mod stun;

use crate::state::AppState;
use std::sync::Arc;

/// Wire up connectivity event handlers.
/// Extracts the matchmake circuit event handler that was previously inline in main.rs.
/// Call this once after the P2P-CD engine has been started.
pub async fn register_handlers(state: AppState) {
    if let Some(ref engine) = state.p2pcd_engine {
        if let Some(handler) = engine.cap_router().handler_by_name("core.network.relay.1") {
            if let Some(relay_handler) = handler
                .as_any()
                .downcast_ref::<::p2pcd::capabilities::relay::RelayHandler>()
            {
                let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                relay_handler.set_event_callback(tx).await;
                let mm_state = state.clone();
                let mm_counter = Arc::clone(&state.matchmake_counter);
                tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        if let ::p2pcd::capabilities::relay::CircuitEvent::Data {
                            circuit_id,
                            data,
                            ..
                        } = event
                        {
                            match matchmake::decode_message(&data) {
                                Ok(matchmake::MatchmakeMessage::Request(req)) => {
                                    let s = mm_state.clone();
                                    let c = Arc::clone(&mm_counter);
                                    tokio::spawn(async move {
                                        if let Err(e) = matchmake::handle_incoming_matchmake(
                                            &s, circuit_id, req, c,
                                        )
                                        .await
                                        {
                                            tracing::warn!("matchmake handler error: {}", e);
                                        }
                                    });
                                }
                                Ok(_) => {
                                    tracing::debug!(
                                        "matchmake: ignoring non-request on circuit {}",
                                        circuit_id
                                    );
                                }
                                Err(_) => {
                                    // Not a matchmake message — ignore
                                }
                            }
                        }
                    }
                    tracing::debug!("matchmake: circuit event channel closed");
                });
                tracing::info!("Matchmake circuit event handler registered");
            }
        }
    }
}

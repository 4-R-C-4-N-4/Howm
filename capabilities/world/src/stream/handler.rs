//! WebSocket handler — axum route that upgrades to WS and runs the view loop.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};

use crate::gen::cell::Cell;

use super::protocol::{ClientMessage, ServerMessage};
use super::view::{ViewState, ViewEvent};

/// Axum handler for WebSocket upgrade.
pub async fn ws_handler(
    AxumPath(ip): AxumPath<String>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, ip))
}

async fn handle_socket(socket: WebSocket, ip: String) {
    let cell = match Cell::from_ip_str(&ip) {
        Some(c) => c,
        None => return,
    };

    let mut view = ViewState::new(cell, 80.0);

    let (mut sender, mut receiver) = socket.split();

    // Send init message
    let (env, cam, ground) = view.get_init();
    let init = ServerMessage::Init {
        environment: env,
        camera: cam,
        ground,
    };
    if let Ok(json) = serde_json::to_string(&init) {
        let _ = sender.send(Message::Text(json.into())).await;
    }

    // Send initial entity batch at the starting camera position
    let initial_events = view.update_camera(
        0.0, view.player_y, 0.0,  // start at origin (player-relative)
        0.0, -0.3, -1.0,
        60.0,
    );
    for event in initial_events {
        let msg = match event {
            ViewEvent::Enter(entity) => ServerMessage::Enter {
                entity: serde_json::to_value(&entity).unwrap_or_default(),
            },
            ViewEvent::Leave(id) => ServerMessage::Leave { id },
            ViewEvent::Lights(lights) => ServerMessage::Lights {
                lights: lights.iter()
                    .filter_map(|l| serde_json::to_value(l).ok())
                    .collect(),
            },
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = sender.send(Message::Text(json.into())).await;
        }
    }

    // Process client messages
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                    match client_msg {
                        ClientMessage::Camera { position, direction, fov } => {
                            let events = view.update_camera(
                                position[0], position[1], position[2],
                                direction[0], direction[1], direction[2],
                                fov,
                            );

                            for event in events {
                                let msg = match event {
                                    ViewEvent::Enter(entity) => {
                                        ServerMessage::Enter {
                                            entity: serde_json::to_value(&entity).unwrap_or_default(),
                                        }
                                    }
                                    ViewEvent::Leave(id) => {
                                        ServerMessage::Leave { id }
                                    }
                                    ViewEvent::Lights(lights) => {
                                        ServerMessage::Lights {
                                            lights: lights.iter()
                                                .filter_map(|l| serde_json::to_value(l).ok())
                                                .collect(),
                                        }
                                    }
                                };

                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if sender.send(Message::Text(json.into())).await.is_err() {
                                        return;
                                    }
                                }
                            }

                            // Send current district info
                            let district_msg = ServerMessage::District {
                                ip: view.current_district_ip(),
                                loaded_count: view.loaded_count(),
                                visible_count: view.visible_count(),
                            };
                            if let Ok(json) = serde_json::to_string(&district_msg) {
                                let _ = sender.send(Message::Text(json.into())).await;
                            }
                        }
                    }
                }
            }
            Message::Close(_) => return,
            _ => {}
        }
    }
}

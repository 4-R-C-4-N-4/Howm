//! WebSocket protocol messages — client↔server.

use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════
// Client → Server
// ═══════════════════════════════════════════════════════════════════════════

/// Client sends camera state at 2-4 Hz.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "camera")]
    Camera {
        position: [f64; 3],
        direction: [f64; 3],
        fov: f64,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Server → Client
// ═══════════════════════════════════════════════════════════════════════════

/// Server streams incremental scene updates.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Initial scene setup — environment, camera hint, ground.
    #[serde(rename = "init")]
    Init {
        environment: serde_json::Value,
        camera: serde_json::Value,
        ground: serde_json::Value,
    },

    /// Entity enters the visible set.
    #[serde(rename = "enter")]
    Enter {
        entity: serde_json::Value,
    },

    /// Entity leaves the visible set.
    #[serde(rename = "leave")]
    Leave {
        id: String,
    },

    /// Entity state update (position, emissive, etc.)
    #[serde(rename = "update")]
    Update {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        position: Option<[f64; 3]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        emissive: Option<f64>,
    },

    /// Active light set changed.
    #[serde(rename = "lights")]
    Lights {
        lights: Vec<serde_json::Value>,
    },
}

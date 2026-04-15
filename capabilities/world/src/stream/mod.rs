//! WebSocket view-dependent streaming.
//!
//! Maintains per-client view state and streams entity enter/leave/update
//! events as the player moves. Handles frustum culling, LOD, coordinate
//! translation to player-relative space, and light streaming.

pub mod protocol;
pub mod view;
pub mod handler;

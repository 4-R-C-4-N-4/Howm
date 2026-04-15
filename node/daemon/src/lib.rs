//! Howm daemon library crate.
//!
//! Re-exports modules for integration testing. The binary entry point
//! is in main.rs which imports from this library.

pub mod accept;
pub mod api;
pub mod capabilities;
pub mod config;
pub mod connectivity;
pub mod embedded_ui;
pub mod error;
pub mod executor;
pub mod identity;
pub mod invite;
pub use connectivity::{lan_discovery, matchmake, net_detect, punch, stun};
pub mod notifications;
pub mod open_invite;
pub mod p2pcd;
pub mod peers;
pub mod proxy;
pub mod state;
pub mod watchdog;
pub mod wireguard;

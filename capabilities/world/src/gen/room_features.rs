//! Room-feature entities (spaces §2.4) — the contents of capability rooms inside
//! a peer's Inside: feed posts in the gallery, message threads in the
//! correspondence room, files in the archive. Each becomes a `DescribedEntity`.
//!
//! Counts come from the owning capability's live state. Until that's pulled from
//! the daemon (the same plumbing the multiplayer phase adds), callers pass the
//! counts in (parameterised, like the tunnel metrics — decision D3). Positions
//! are deterministic from the room seed.

use serde::{Deserialize, Serialize};

use super::hash::ha;
use super::inside::{Inside, Room};
use crate::types::Point;

const FILE_TYPES: [&str; 4] = ["documents", "images", "archives", "code"];

/// Live-state counts that drive how many features each room holds.
#[derive(Debug, Clone, Default)]
pub struct FeatureCounts {
    pub feed_posts: usize,
    /// The first `feed_unread` posts render as unread (glow).
    pub feed_unread: usize,
    pub message_threads: usize,
    /// Total messages across all threads (distributed evenly for the count trait).
    pub messages: usize,
    pub files: usize,
}

/// A single feature placed inside a capability room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomFeature {
    pub room: String,
    /// feed_post | message_thread | file.
    pub kind: String,
    pub index: usize,
    pub seed: u32,
    /// Position within the Inside (inside-local coords).
    pub position: Point,
    pub unread: bool,
    /// Message count (threads) or size proxy (files); 0 otherwise.
    pub count: usize,
    /// File type for files; empty otherwise.
    pub variant: String,
}

/// Deterministic grid position for feature `i` of `n` within a room footprint.
fn grid_pos(room: &Room, i: usize, n: usize) -> Point {
    let cols = (n as f64).sqrt().ceil().max(1.0);
    let rows = ((n as f64) / cols).ceil().max(1.0);
    let margin = 0.6;
    let uw = (room.width - 2.0 * margin).max(0.5);
    let ud = (room.depth - 2.0 * margin).max(0.5);
    let col = (i as f64) % cols;
    let row = ((i as f64) / cols).floor();
    Point::new(
        room.position.x - uw / 2.0 + (col + 0.5) * (uw / cols),
        room.position.y - ud / 2.0 + (row + 0.5) * (ud / rows),
    )
}

fn room_of<'a>(inside: &'a Inside, room_type: &str) -> Option<&'a Room> {
    inside.rooms.iter().find(|r| r.room_type == room_type)
}

/// Generate the feature entities for an Inside from its capability state counts.
pub fn generate_room_features(inside: &Inside, counts: &FeatureCounts) -> Vec<RoomFeature> {
    let mut out = Vec::new();

    // Feed posts → gallery (§2.4).
    if let Some(room) = room_of(inside, "gallery") {
        for i in 0..counts.feed_posts {
            out.push(RoomFeature {
                room: room.name.clone(),
                kind: "feed_post".into(),
                index: i,
                seed: ha(room.room_seed ^ i as u32 ^ 0xf33d),
                position: grid_pos(room, i, counts.feed_posts),
                unread: i < counts.feed_unread,
                count: 0,
                variant: String::new(),
            });
        }
    }

    // Message threads → correspondence room (§2.4).
    if let Some(room) = room_of(inside, "correspondence") {
        let per_thread = if counts.message_threads > 0 {
            (counts.messages / counts.message_threads).max(1)
        } else {
            0
        };
        for i in 0..counts.message_threads {
            let seed = ha(room.room_seed ^ i as u32 ^ 0xc0de);
            out.push(RoomFeature {
                room: room.name.clone(),
                kind: "message_thread".into(),
                index: i,
                seed,
                position: grid_pos(room, i, counts.message_threads),
                unread: seed & 1 == 0,
                count: per_thread,
                variant: String::new(),
            });
        }
    }

    // Files → archive (§2.4).
    if let Some(room) = room_of(inside, "archive") {
        for i in 0..counts.files {
            let seed = ha(room.room_seed ^ i as u32 ^ 0xf17e);
            out.push(RoomFeature {
                room: room.name.clone(),
                kind: "file".into(),
                index: i,
                seed,
                position: grid_pos(room, i, counts.files),
                unread: false,
                count: (seed % 100) as usize + 1, // size proxy 1–100
                variant: FILE_TYPES[(seed as usize) % FILE_TYPES.len()].into(),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::super::cell::Cell;
    use super::super::inside::generate_inside;
    use super::*;

    fn inside() -> Inside {
        let caps: Vec<String> = ["social.feed", "social.messaging", "social.files"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        generate_inside(&Cell::from_ip_str("93.184.216.0").unwrap(), &[1, 2, 3, 4], &caps, 0)
    }

    #[test]
    fn counts_route_to_correct_rooms() {
        let inside = inside();
        let counts = FeatureCounts {
            feed_posts: 5,
            feed_unread: 2,
            message_threads: 3,
            messages: 30,
            files: 4,
        };
        let f = generate_room_features(&inside, &counts);
        let by = |k: &str| f.iter().filter(|x| x.kind == k).count();
        assert_eq!(by("feed_post"), 5);
        assert_eq!(by("message_thread"), 3);
        assert_eq!(by("file"), 4);
        // Feed posts land in the gallery.
        assert!(f.iter().filter(|x| x.kind == "feed_post").all(|x| x.room == "social.feed"));
    }

    #[test]
    fn unread_and_thread_counts() {
        let inside = inside();
        let counts = FeatureCounts {
            feed_posts: 4,
            feed_unread: 2,
            message_threads: 2,
            messages: 10,
            ..Default::default()
        };
        let f = generate_room_features(&inside, &counts);
        let unread = f.iter().filter(|x| x.kind == "feed_post" && x.unread).count();
        assert_eq!(unread, 2);
        let thread = f.iter().find(|x| x.kind == "message_thread").unwrap();
        assert_eq!(thread.count, 5); // 10 messages / 2 threads
    }

    #[test]
    fn deterministic_positions() {
        let inside = inside();
        let counts = FeatureCounts {
            files: 6,
            ..Default::default()
        };
        let a = generate_room_features(&inside, &counts);
        let b = generate_room_features(&inside, &counts);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.position.x, y.position.x);
            assert_eq!(x.seed, y.seed);
            assert!(FILE_TYPES.contains(&x.variant.as_str()));
        }
    }

    #[test]
    fn features_stay_within_room() {
        let inside = inside();
        let counts = FeatureCounts {
            files: 9,
            ..Default::default()
        };
        let room = inside.rooms.iter().find(|r| r.room_type == "archive").unwrap();
        for f in generate_room_features(&inside, &counts) {
            assert!((f.position.x - room.position.x).abs() <= room.width / 2.0);
            assert!((f.position.y - room.position.y).abs() <= room.depth / 2.0);
        }
    }
}

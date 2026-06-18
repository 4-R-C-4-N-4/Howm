//! Inside generation — a peer's interior space (spaces §2).
//!
//! Every peer has an Inside: an entry hall plus one room per installed
//! capability, laid out around the hall. Fully deterministic from the peer id;
//! the visual aesthetic is carried from the peer's Outside district (same
//! palette). Room *positions* are stable as capabilities are added/removed
//! (rooms are placed in a deterministic, name-sorted order).

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use super::cell::Cell;
use super::hash::{ha, hash_to_f64};
use super::home::peer_id_u32;
use crate::types::Point;

// ── Config (spaces §2.9) ────────────────────────────────────────────────────
const HALL_BASE_AREA: f64 = 40.0;
const HALL_AREA_PER_CAP: f64 = 10.0;
const HALL_AREA_PER_TUNNEL: f64 = 6.0;
const HALL_HEIGHT: f64 = 4.0;
const ROOM_BASE_AREA: f64 = 30.0;
const ROOM_MAX_AREA: f64 = 120.0;
const ROOM_BASE_HEIGHT: f64 = 3.5;
const DOOR_WIDTH: f64 = 1.2;
const DOOR_HEIGHT: f64 = 2.8;
/// Gap between the hall wall and a room wall (not spec-pinned; keeps doors sane).
const ROOM_GAP: f64 = 1.5;
/// Salt for the layout orientation seed (spaces §2.5 writes the typo `0xla70`).
const LAYOUT_SEED_SALT: u32 = 0x1a70;

/// A doorway connecting two rooms (here always hall ↔ capability room).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Door {
    pub from: String,
    pub to: String,
    /// Door centre on the hall perimeter (inside-local coords).
    pub position: Point,
    pub width: f64,
    pub height: f64,
}

/// A room within the Inside.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    /// "entry_hall" or the capability name.
    pub name: String,
    /// hall | gallery | hearth | correspondence | archive | amphitheatre | chamber.
    pub room_type: String,
    pub room_seed: u32,
    /// Room centre (x, z) in inside-local coords; the hall sits at the origin.
    pub position: Point,
    pub width: f64,
    pub depth: f64,
    pub height: f64,
}

/// A peer's complete Inside space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inside {
    pub peer_id_u32: u32,
    pub inside_seed: u32,
    /// Cell key of the peer's Outside district (drives the aesthetic).
    pub cell_key: u32,
    /// cardinal | ring | corridor.
    pub layout: String,
    pub hall_area: f64,
    pub rooms: Vec<Room>,
    pub doors: Vec<Door>,
}

/// Capability category → room type (spaces §2.3). Matched on the name suffix so
/// both `social.feed` and a bare `feed` resolve.
fn room_type_for(cap_name: &str) -> &'static str {
    let n = cap_name.rsplit('.').next().unwrap_or(cap_name);
    match n {
        "feed" => "gallery",
        "presence" => "hearth",
        "messaging" => "correspondence",
        "files" => "archive",
        "voice" => "amphitheatre",
        _ => "chamber",
    }
}

/// Room aspect ratio (width:depth) by type (spaces §2.5).
fn aspect_for(room_type: &str) -> f64 {
    match room_type {
        "gallery" => 1.5,
        "correspondence" => 1.2,
        "archive" => 0.8,
        _ => 1.0, // hearth, amphitheatre, chamber, hall
    }
}

/// FNV-1a 32-bit hash of a capability name — the `ha(capability_name)` term in
/// the room-seed derivation (spaces §2.2).
fn hash_str(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Bounding radius used for spacing (half the larger horizontal extent).
fn bounding_radius(r: &Room) -> f64 {
    r.width.max(r.depth) / 2.0
}

/// Generate a peer's Inside. `capabilities` is the peer's installed-capability
/// set; `tunnel_count` the number of active peer tunnels (both feed the hall
/// area). Pure and deterministic.
pub fn generate_inside(
    cell: &Cell,
    peer_id: &[u8],
    capabilities: &[String],
    tunnel_count: usize,
) -> Inside {
    let pid = peer_id_u32(peer_id);
    let inside_seed = ha(pid);

    // Deterministic, stable room order.
    let mut caps: Vec<String> = capabilities.to_vec();
    caps.sort();
    caps.dedup();
    let n = caps.len();

    let hall_area =
        HALL_BASE_AREA + n as f64 * HALL_AREA_PER_CAP + tunnel_count as f64 * HALL_AREA_PER_TUNNEL;
    let hall_width = hall_area.sqrt();

    let mut rooms = Vec::with_capacity(n + 1);
    rooms.push(Room {
        name: "entry_hall".to_string(),
        room_type: "hall".to_string(),
        room_seed: inside_seed,
        position: Point::new(0.0, 0.0),
        width: hall_width,
        depth: hall_width,
        height: HALL_HEIGHT,
    });

    for cap in &caps {
        let room_seed = ha(inside_seed ^ hash_str(cap));
        let rt = room_type_for(cap);
        let aspect = aspect_for(rt);
        let area = ROOM_BASE_AREA.min(ROOM_MAX_AREA); // activity multiplier = 1.0 (no live state)
        let width = (area * aspect).sqrt();
        let depth = area / width;
        rooms.push(Room {
            name: cap.clone(),
            room_type: rt.to_string(),
            room_seed,
            position: Point::new(0.0, 0.0),
            width,
            depth,
            height: ROOM_BASE_HEIGHT,
        });
    }

    let layout = if n <= 4 {
        "cardinal"
    } else if n <= 8 {
        "ring"
    } else {
        "corridor"
    };

    // Orientation jitter so different peers' interiors face different ways.
    let layout_seed = ha(inside_seed ^ LAYOUT_SEED_SALT);
    let base_angle = hash_to_f64(layout_seed) * TAU;
    let hall_half = hall_width / 2.0;
    place_rooms(&mut rooms, layout, base_angle, hall_half);

    // Doors: hall ↔ each capability room.
    let hall_center = rooms[0].position;
    let doors = rooms[1..]
        .iter()
        .map(|r| door_between("entry_hall", &r.name, hall_center, r.position, hall_half))
        .collect();

    Inside {
        peer_id_u32: pid,
        inside_seed,
        cell_key: cell.key,
        layout: layout.to_string(),
        hall_area,
        rooms,
        doors,
    }
}

/// Position the capability rooms (`rooms[1..]`) around the hall per the layout.
fn place_rooms(rooms: &mut [Room], layout: &str, base_angle: f64, hall_half: f64) {
    let n = rooms.len().saturating_sub(1);
    if n == 0 {
        return;
    }
    // Snapshot bounding radii (avoid borrow conflict while mutating positions).
    let radii: Vec<f64> = rooms[1..].iter().map(bounding_radius).collect();

    for (i, r) in rooms[1..].iter_mut().enumerate() {
        let room_half = radii[i];
        let dist = hall_half + ROOM_GAP + room_half;
        let (x, z) = match layout {
            "cardinal" => {
                // N, E, S, W rotated by base_angle.
                let dir = base_angle + (i as f64) * (TAU / 4.0);
                (dir.cos() * dist, dir.sin() * dist)
            }
            "ring" => {
                let dir = base_angle + (i as f64) * (TAU / n as f64);
                (dir.cos() * dist, dir.sin() * dist)
            }
            _ => {
                // corridor: rooms alternate sides along a (rotated) axis.
                let col = (i / 2) as f64 + 1.0;
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let along = col * (2.0 * room_half + ROOM_GAP).max(hall_half);
                let perp = side * dist;
                // Rotate (along, perp) by base_angle.
                (
                    along * base_angle.cos() - perp * base_angle.sin(),
                    along * base_angle.sin() + perp * base_angle.cos(),
                )
            }
        };
        r.position = Point::new(x, z);
    }
}

fn door_between(
    from: &str,
    to: &str,
    hall_center: Point,
    room_center: Point,
    hall_half: f64,
) -> Door {
    let dx = room_center.x - hall_center.x;
    let dz = room_center.y - hall_center.y;
    let len = (dx * dx + dz * dz).sqrt().max(1e-9);
    Door {
        from: from.to_string(),
        to: to.to_string(),
        position: Point::new(
            hall_center.x + dx / len * hall_half,
            hall_center.y + dz / len * hall_half,
        ),
        width: DOOR_WIDTH,
        height: DOOR_HEIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell() -> Cell {
        Cell::from_ip_str("93.184.216.0").unwrap()
    }

    fn caps(n: usize) -> Vec<String> {
        ["social.feed", "social.messaging", "social.files", "social.presence", "social.voice"]
            .iter()
            .take(n)
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn deterministic() {
        let c = cell();
        let pid = [1, 2, 3, 4];
        let a = generate_inside(&c, &pid, &caps(3), 1);
        let b = generate_inside(&c, &pid, &caps(3), 1);
        assert_eq!(a.inside_seed, b.inside_seed);
        assert_eq!(a.rooms.len(), b.rooms.len());
        assert_eq!(a.rooms[2].position.x, b.rooms[2].position.x);
    }

    #[test]
    fn entry_hall_always_present_even_with_zero_caps() {
        let inside = generate_inside(&cell(), &[9, 9], &[], 0);
        assert_eq!(inside.rooms.len(), 1);
        assert_eq!(inside.rooms[0].name, "entry_hall");
        assert_eq!(inside.layout, "cardinal");
        assert_eq!(inside.hall_area, HALL_BASE_AREA);
    }

    #[test]
    fn room_count_and_doors_match_caps() {
        let inside = generate_inside(&cell(), &[1, 2, 3, 4], &caps(5), 2);
        assert_eq!(inside.rooms.len(), 6); // hall + 5 caps
        assert_eq!(inside.doors.len(), 5);
    }

    #[test]
    fn hall_area_formula() {
        let inside = generate_inside(&cell(), &[1, 2, 3, 4], &caps(3), 2);
        assert_eq!(
            inside.hall_area,
            HALL_BASE_AREA + 3.0 * HALL_AREA_PER_CAP + 2.0 * HALL_AREA_PER_TUNNEL
        );
    }

    #[test]
    fn layout_selection_by_count() {
        let c = cell();
        let pid = [7, 7, 7, 7];
        let many: Vec<String> = (0..10).map(|i| format!("cap.{i}")).collect();
        assert_eq!(generate_inside(&c, &pid, &caps(4), 0).layout, "cardinal");
        let six: Vec<String> = (0..6).map(|i| format!("cap.{i}")).collect();
        assert_eq!(generate_inside(&c, &pid, &six, 0).layout, "ring");
        assert_eq!(generate_inside(&c, &pid, &many, 0).layout, "corridor");
    }

    #[test]
    fn room_types_mapped() {
        let inside = generate_inside(&cell(), &[1, 2, 3, 4], &caps(5), 0);
        let ty = |name: &str| {
            inside
                .rooms
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.room_type.as_str())
                .unwrap_or("?")
                .to_string()
        };
        assert_eq!(ty("social.feed"), "gallery");
        assert_eq!(ty("social.presence"), "hearth");
        assert_eq!(ty("social.messaging"), "correspondence");
        assert_eq!(ty("social.files"), "archive");
        assert_eq!(ty("social.voice"), "amphitheatre");
    }

    #[test]
    fn distinct_peers_differ() {
        let c = cell();
        let a = generate_inside(&c, &[1, 1, 1, 1], &caps(3), 0);
        let b = generate_inside(&c, &[2, 2, 2, 2], &caps(3), 0);
        assert_ne!(a.inside_seed, b.inside_seed);
    }

    #[test]
    fn rooms_do_not_overlap_hall() {
        // Every capability room centre must sit outside the hall footprint.
        let inside = generate_inside(&cell(), &[1, 2, 3, 4], &caps(5), 1);
        let hall_half = inside.rooms[0].width / 2.0;
        for r in &inside.rooms[1..] {
            let d = (r.position.x.powi(2) + r.position.y.powi(2)).sqrt();
            assert!(d > hall_half, "room {} overlaps the hall", r.name);
        }
    }
}

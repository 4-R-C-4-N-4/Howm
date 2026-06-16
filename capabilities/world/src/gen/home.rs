//! Home placement — a peer's unique home structure in the Outside (spaces §1.2).
//!
//! A peer's home sits in *their own* district (the cell derived from their IP),
//! at a position derived from their peer id. Fully deterministic: same peer id +
//! same cell key → same home, always. Multiple peers can share a cell; each has
//! a distinct `home_seed`, so homes don't collide.

use serde::{Deserialize, Serialize};

use super::blocks::{extract_blocks, Block, BlockType};
use super::cell::Cell;
use super::config::config;
use super::district::generate_district;
use super::hash::{ha, hash_to_range};
use super::rivers::generate_rivers;
use super::roads::{generate_roads, RoadNetwork};
use super::zones::point_in_polygon_seeded;
use crate::types::{Point, Polygon};

/// Home archetypes (spaces §1.2). Selected by `home_archetype_seed % 6`.
pub const HOME_ARCHETYPES: [&str; 6] =
    ["pavilion", "tower", "chamber", "portal", "burrow", "shrine"];

const HOME_MIN_RADIUS: f64 = 1.5;
const HOME_MAX_RADIUS: f64 = 4.0;
const HOME_MIN_HEIGHT: f64 = 2.5;
const HOME_MAX_HEIGHT: f64 = 5.0;

/// A placed peer home in the Outside.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeStructure {
    /// First 4 bytes of the peer id, packed big-endian — the placement seed input.
    pub peer_id_u32: u32,
    /// `ha(peer_id_u32 ^ cell_key)` — the root seed for all home properties.
    pub home_seed: u32,
    /// Position within the district polygon (world units).
    pub position: Point,
    /// One of [`HOME_ARCHETYPES`].
    pub archetype: String,
    pub footprint_radius: f64,
    pub height: f64,
    /// Cell key of the district this home sits in.
    pub cell_key: u32,
}

/// Pack the first 4 bytes of a peer id big-endian (spaces §1.2). Short ids are
/// zero-padded.
pub fn peer_id_u32(peer_id: &[u8]) -> u32 {
    let mut b = [0u8; 4];
    for (i, slot) in b.iter_mut().enumerate() {
        *slot = peer_id.get(i).copied().unwrap_or(0);
    }
    u32::from_be_bytes(b)
}

/// Distance from point `p` to segment `a`–`b`.
fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-12 {
        return p.distance_to(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
    p.distance_to(Point::new(a.x + t * dx, a.y + t * dy))
}

/// True if `p` is within `tol` of any road centreline (spaces §1.2 uses
/// `LAMP_OFFSET` = 3.5 wu).
fn point_on_road(p: Point, roads: &RoadNetwork, tol: f64) -> bool {
    roads
        .segments
        .iter()
        .any(|s| point_segment_distance(p, s.a, s.b) <= tol)
}

/// A position is walkable if it's not inside a water block and not on a road.
fn walkable(p: Point, blocks: &[Block], roads: &RoadNetwork, tol: f64) -> bool {
    let in_water = blocks
        .iter()
        .any(|b| b.block_type == BlockType::Water && b.polygon.contains(p));
    !in_water && !point_on_road(p, roads, tol)
}

/// Place a peer's home in the given district. Pure function of (cell, peer_id)
/// plus the district's geometry (so callers that already have it avoid a
/// regenerate — see [`place_home_in_cell`] for the convenience path).
pub fn place_home(
    cell: &Cell,
    district_polygon: &Polygon,
    blocks: &[Block],
    roads: &RoadNetwork,
    peer_id: &[u8],
) -> HomeStructure {
    let tol = config().lamp_offset;
    let pid = peer_id_u32(peer_id);
    let home_seed = ha(pid ^ cell.key);

    // Initial position; if it lands in water or on a road, re-roll up to 16
    // times, then fall back to the district centroid.
    let mut position = point_in_polygon_seeded(district_polygon, home_seed);
    if !walkable(position, blocks, roads, tol) {
        position = (1u32..=16)
            .map(|attempt| point_in_polygon_seeded(district_polygon, ha(home_seed ^ attempt)))
            .find(|&candidate| walkable(candidate, blocks, roads, tol))
            .unwrap_or_else(|| district_polygon.centroid());
    }

    let archetype_seed = ha(home_seed ^ 0xb1d);
    let archetype = HOME_ARCHETYPES[(archetype_seed % HOME_ARCHETYPES.len() as u32) as usize];
    let footprint_radius =
        hash_to_range(ha(home_seed ^ 0xb1d ^ 0x1), HOME_MIN_RADIUS, HOME_MAX_RADIUS);
    let height = hash_to_range(ha(home_seed ^ 0xb1d ^ 0x2), HOME_MIN_HEIGHT, HOME_MAX_HEIGHT);

    HomeStructure {
        peer_id_u32: pid,
        home_seed,
        position,
        archetype: archetype.to_string(),
        footprint_radius,
        height,
        cell_key: cell.key,
    }
}

/// Regenerate the district geometry for `cell` and place the peer's home.
pub fn place_home_in_cell(cell: &Cell, peer_id: &[u8]) -> HomeStructure {
    let dist = generate_district(cell);
    let roads = generate_roads(&dist);
    let rivers = generate_rivers(cell, &dist.polygon.vertices);
    let blocks = extract_blocks(cell, &dist.polygon, &roads, &rivers);
    place_home(cell, &dist.polygon, &blocks, &roads, peer_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell() -> Cell {
        Cell::from_ip_str("93.184.216.0").unwrap()
    }

    #[test]
    fn home_is_deterministic() {
        let c = cell();
        let pid = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02];
        let a = place_home_in_cell(&c, &pid);
        let b = place_home_in_cell(&c, &pid);
        assert_eq!(a.home_seed, b.home_seed);
        assert_eq!(a.position.x, b.position.x);
        assert_eq!(a.position.y, b.position.y);
        assert_eq!(a.archetype, b.archetype);
    }

    #[test]
    fn seed_matches_spec_formula() {
        let c = cell();
        let pid = [0xde, 0xad, 0xbe, 0xef];
        let home = place_home_in_cell(&c, &pid);
        assert_eq!(home.peer_id_u32, 0xdead_beef);
        assert_eq!(home.home_seed, ha(0xdead_beef ^ c.key));
    }

    #[test]
    fn archetype_in_set_and_bounds_respected() {
        let c = cell();
        for n in 0u32..64 {
            let pid = n.to_be_bytes();
            let home = place_home_in_cell(&c, &pid);
            assert!(HOME_ARCHETYPES.contains(&home.archetype.as_str()));
            assert!(home.footprint_radius >= HOME_MIN_RADIUS && home.footprint_radius <= HOME_MAX_RADIUS);
            assert!(home.height >= HOME_MIN_HEIGHT && home.height <= HOME_MAX_HEIGHT);
        }
    }

    #[test]
    fn distinct_peers_get_distinct_homes() {
        let c = cell();
        let h1 = place_home_in_cell(&c, &[1, 2, 3, 4]);
        let h2 = place_home_in_cell(&c, &[5, 6, 7, 8]);
        assert_ne!(h1.home_seed, h2.home_seed);
    }

    #[test]
    fn home_avoids_water_and_roads() {
        // Every placed home must satisfy the walkable predicate against its own
        // district (or be the centroid fallback).
        let c = cell();
        let dist = generate_district(&c);
        let roads = generate_roads(&dist);
        let rivers = generate_rivers(&c, &dist.polygon.vertices);
        let blocks = extract_blocks(&c, &dist.polygon, &roads, &rivers);
        let tol = config().lamp_offset;
        for n in 0u32..32 {
            let home = place_home(&c, &dist.polygon, &blocks, &roads, &n.to_be_bytes());
            let ok = walkable(home.position, &blocks, &roads, tol)
                || home.position.distance_to(dist.polygon.centroid()) < 1e-9;
            assert!(ok, "home {} landed on water/road and is not the centroid", n);
        }
    }
}

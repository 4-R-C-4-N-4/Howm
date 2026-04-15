//! Zone system — sub-regions within blocks that carry spawn mode,
//! density, and object affinity. Zones are the fundamental unit of
//! object placement.

use serde::{Deserialize, Serialize};

use super::blocks::{Block, BlockType};
use super::config::config;
use super::hash::{ha, hb, hash_to_f64};
use super::voronoi::{clip_polygon, voronoi_cells};
use crate::types::{Point, Polygon};

/// A zone within a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    /// Zone index within the block.
    pub idx: usize,
    /// Zone seed.
    pub seed: u32,
    /// Zone polygon (clipped Voronoi cell within block).
    pub polygon: Polygon,
    /// Centroid.
    pub centroid: Point,
    /// Area in world units².
    pub area: f64,
    /// Density factor 0.0–1.0.
    pub density: f64,
    /// Reseed interval in ms (how often spawn positions shift).
    pub reseed_interval: u64,
    /// Preferred object role affinities (1–3 role IDs) per §6.4.
    pub affinity: Vec<u32>,
}

/// Deterministic point-in-polygon placement per §11.9.
pub fn point_in_polygon_seeded(polygon: &Polygon, seed: u32) -> Point {
    let (min_x, min_y, max_x, max_y) = polygon.bbox();
    let w = max_x - min_x;
    let h = max_y - min_y;

    for attempt in 0..32_u32 {
        let s = ha(seed ^ attempt ^ 0xf1a2b3c4);
        let t = hb(seed ^ attempt ^ 0xf1a2b3c4);
        let x = min_x + hash_to_f64(s) * w;
        let y = min_y + hash_to_f64(t) * h;
        let p = Point::new(x, y);
        if polygon.contains(p) {
            return p;
        }
    }

    // Fallback to centroid
    polygon.centroid()
}

/// Compute spawn position for an object in a zone.
pub fn spawn_position(zone: &Zone, role_id: u32, spawn_index: u32, time_slot: u64) -> (u32, Point) {
    let pos_seed = ha(zone.seed ^ role_id ^ spawn_index ^ time_slot as u32);
    let position = point_in_polygon_seeded(&zone.polygon, pos_seed);
    (pos_seed, position)
}

/// Apply ±10% jitter to a base reseed interval per §6.4.
/// For infinite intervals (u64::MAX), returns u64::MAX unchanged.
fn apply_reseed_jitter(base: u64, zone_seed: u32) -> u64 {
    if base == u64::MAX {
        return u64::MAX;
    }
    let jitter_frac = (ha(zone_seed ^ 0x3) & 0xFFFF) as f64 / 65535.0;
    let jitter = (base as f64 * 0.2 * jitter_frac) as u64;
    base + jitter
}

/// Derive 1–3 role affinities for a zone from its seed and block type.
/// Per §6.4: parks bias toward flora/creature roles, building blocks
/// bias toward fixture/ornament roles.
fn derive_affinity(zone_seed: u32, block_type: BlockType) -> Vec<u32> {
    let aff_seed = ha(zone_seed ^ 0x2);
    let count = 1 + (aff_seed & 0x3).min(2) as usize; // 1–3

    // Role pools by block type (using FixtureRole IDs from fixtures.rs)
    let pool: &[u32] = match block_type {
        BlockType::Building => &[0x01, 0x05, 0x06, 0x08], // illumination, utility, display, ornament
        BlockType::Park => &[0x02, 0x08, 0x09],           // seating, ornament, water_structure
        BlockType::Plaza => &[0x01, 0x02, 0x07, 0x08],    // illumination, seating, offering, ornament
        BlockType::Water => &[0x09],                       // water_structure
        BlockType::Riverbank => &[0x03, 0x08],             // boundary_marker, ornament
    };

    let mut affinities = Vec::with_capacity(count);
    for i in 0..count {
        let idx = ha(aff_seed ^ i as u32) as usize % pool.len();
        let role_id = pool[idx];
        if !affinities.contains(&role_id) {
            affinities.push(role_id);
        }
    }
    affinities
}

/// Generate zones for a block.
pub fn generate_zones(cell_key: u32, block: &Block) -> Vec<Zone> {
    let cfg = config();

    let zone_count = {
        let base = (block.area / cfg.zone_area_base).floor() as u32;
        let bonus = (block.area / cfg.zone_area_base * cfg.zone_entropy_bonus as f64 / 4.0).floor() as u32;
        let raw = base + bonus;
        raw.max(2).min(12) as usize
    };

    let base_reseed: u64 = match block.block_type {
        BlockType::Building => u64::MAX,   // Fixtures never re-seed
        BlockType::Park => 86_400_000,     // Daily
        BlockType::Plaza => u64::MAX,      // Fixed
        BlockType::Water => u64::MAX,      // Fixed
        BlockType::Riverbank => 3_600_000, // Hourly
    };

    // Generate zone seed points within the block polygon
    let mut zone_seed_pts: Vec<Point> = Vec::with_capacity(zone_count);
    for z in 0..zone_count {
        let pt_seed = ha(cell_key ^ block.idx as u32 ^ 0x7a3f ^ z as u32);
        let pt = point_in_polygon_seeded(&block.polygon, pt_seed);
        zone_seed_pts.push(pt);
    }

    if zone_seed_pts.len() < 3 {
        // Too few points for Voronoi — return the whole block as one zone
        let seed = ha(cell_key ^ block.idx as u32 ^ 0);
        let reseed_interval = apply_reseed_jitter(base_reseed, seed);
        return vec![Zone {
            idx: 0,
            seed,
            polygon: block.polygon.clone(),
            centroid: block.centroid,
            area: block.area,
            density: hash_to_f64(ha(seed ^ 0x1)),
            reseed_interval,
            affinity: derive_affinity(seed, block.block_type),
        }];
    }

    // Voronoi subdivision within the block
    let vcells = voronoi_cells(&zone_seed_pts);
    let (bmin_x, bmin_y, bmax_x, bmax_y) = block.polygon.bbox();

    let mut zones = Vec::new();
    for (z_idx, vcell) in vcells.iter().enumerate() {
        if vcell.vertices.is_empty() {
            continue;
        }

        // Clip Voronoi cell to block bounding box first, then to block polygon
        let clipped = clip_polygon(&vcell.vertices, bmin_x, bmin_y, bmax_x, bmax_y);
        if clipped.is_empty() {
            continue;
        }

        // Further clip to the actual block polygon using point-in-polygon filtering
        // (Simplified: use the Voronoi cell vertices that are inside the block)
        let zone_poly = clip_to_block(&clipped, &block.polygon);
        if zone_poly.vertices.len() < 3 {
            continue;
        }

        let seed = ha(cell_key ^ block.idx as u32 ^ z_idx as u32);
        let area = zone_poly.area();
        let centroid = zone_poly.centroid();
        let density = hash_to_f64(ha(seed ^ 0x1));
        let reseed_interval = apply_reseed_jitter(base_reseed, seed);

        zones.push(Zone {
            idx: z_idx,
            seed,
            polygon: zone_poly,
            centroid,
            area,
            density,
            reseed_interval,
            affinity: derive_affinity(seed, block.block_type),
        });
    }

    // Fallback: if Voronoi produced nothing useful, use the whole block
    if zones.is_empty() {
        let seed = ha(cell_key ^ block.idx as u32 ^ 0);
        let reseed_interval = apply_reseed_jitter(base_reseed, seed);
        zones.push(Zone {
            idx: 0,
            seed,
            polygon: block.polygon.clone(),
            centroid: block.centroid,
            area: block.area,
            density: hash_to_f64(ha(seed ^ 0x1)),
            reseed_interval,
            affinity: derive_affinity(seed, block.block_type),
        });
    }

    zones
}

/// Approximate clipping of a polygon to another polygon.
/// Uses vertex filtering + edge intersection for a reasonable result.
fn clip_to_block(subject: &[Point], clip: &Polygon) -> Polygon {
    // Simple approach: keep vertices that are inside the block,
    // and add intersection points where edges cross.
    let mut result = Vec::new();
    let n = subject.len();

    for i in 0..n {
        let curr = subject[i];
        let next = subject[(i + 1) % n];
        let curr_in = clip.contains(curr);
        let next_in = clip.contains(next);

        if curr_in {
            result.push(curr);
        }

        // If the edge crosses the block boundary, approximate the intersection
        if curr_in != next_in {
            // Binary search for the crossing point
            let mut a = curr;
            let mut b = next;
            for _ in 0..10 {
                let mid = Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
                if clip.contains(mid) == curr_in {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            result.push(Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5));
        }
    }

    Polygon::new(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::blocks::BlockType;
    use crate::types::Polygon;

    fn make_test_block(idx: usize) -> Block {
        Block {
            idx,
            polygon: Polygon::new(vec![
                Point::new(0.0, 0.0),
                Point::new(100.0, 0.0),
                Point::new(100.0, 100.0),
                Point::new(0.0, 100.0),
            ]),
            block_type: BlockType::Building,
            area: 10000.0,
            centroid: Point::new(50.0, 50.0),
            river_adjacent: false,
        }
    }

    #[test]
    fn zones_generated() {
        let block = make_test_block(0);
        let zones = generate_zones(0x5db8d8, &block);
        assert!(!zones.is_empty(), "No zones generated");
    }

    #[test]
    fn zones_cover_block() {
        let block = make_test_block(0);
        let zones = generate_zones(0x5db8d8, &block);
        let total_area: f64 = zones.iter().map(|z| z.area).sum();
        // Zone areas should roughly sum to block area (some loss from clipping)
        assert!(
            total_area > block.area * 0.3,
            "Zone total area {} is too small vs block area {}",
            total_area,
            block.area
        );
    }

    #[test]
    fn point_in_polygon_seeded_works() {
        let poly = Polygon::new(vec![
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 100.0),
        ]);
        let pt = point_in_polygon_seeded(&poly, 0x12345);
        assert!(poly.contains(pt), "Point {:?} not inside polygon", pt);
    }

    #[test]
    fn spawn_position_deterministic() {
        let block = make_test_block(0);
        let zones = generate_zones(0x5db8d8, &block);
        let zone = &zones[0];
        let (s1, p1) = spawn_position(zone, 0x01, 0, 0);
        let (s2, p2) = spawn_position(zone, 0x01, 0, 0);
        assert_eq!(s1, s2);
        assert_eq!(p1.x, p2.x);
        assert_eq!(p1.y, p2.y);
    }

    #[test]
    fn reseed_intervals() {
        let mut b = make_test_block(0);
        b.block_type = BlockType::Park;
        let zones = generate_zones(0x5db8d8, &b);
        // Park base is 86_400_000 with ±10% jitter
        let interval = zones[0].reseed_interval;
        assert!(
            interval >= 86_400_000 && interval <= 86_400_000 + 86_400_000 / 5,
            "Park reseed {} outside expected jitter range",
            interval
        );

        b.block_type = BlockType::Building;
        let zones = generate_zones(0x5db8d8, &b);
        assert_eq!(zones[0].reseed_interval, u64::MAX);
    }

    #[test]
    fn zone_affinity_populated() {
        let block = make_test_block(0);
        let zones = generate_zones(0x5db8d8, &block);
        for zone in &zones {
            assert!(!zone.affinity.is_empty(), "Zone affinity should not be empty");
            assert!(zone.affinity.len() <= 3, "Zone affinity should have at most 3 roles");
        }
    }
}

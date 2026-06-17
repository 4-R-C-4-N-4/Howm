//! Ground paint — a compact per-district raster of "zone codes" that the
//! renderer samples when a ray hits the ground, so streets and zones (parks,
//! water, riverbank, plazas) tint the ground itself.
//!
//! This sidesteps a limitation of the SDF sphere-tracer: thin flat overlay
//! geometry (a road ribbon, a zone patch) only occupies one grid layer and gets
//! skipped by rays sampling from altitude. Painting the ground material instead
//! costs nothing per ray step and reads correctly from any height or angle.

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::gen::blocks::{Block, BlockType};
use crate::gen::config::config;
use crate::gen::rivers::RiverSegment;
use crate::gen::roads::RoadNetwork;
use crate::types::Point;

/// Zone codes written into the raster. 0 = default ground (no override).
pub const CODE_GRASS: u8 = 0;
pub const CODE_PARK: u8 = 1;
pub const CODE_WATER: u8 = 2;
pub const CODE_RIVERBANK: u8 = 3;
pub const CODE_PLAZA: u8 = 4;
pub const CODE_ROAD: u8 = 5;

/// A square raster of zone codes covering one district, in world coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundPaint {
    /// World-space min corner of the painted region.
    pub ox: f64,
    pub oz: f64,
    /// Side length of the (square) region in world units.
    pub size: f64,
    /// Grid resolution (cells per side).
    pub res: usize,
    /// base64 of `res*res` zone-code bytes, row-major (z-major, then x).
    pub codes: String,
}

fn block_code(bt: BlockType) -> u8 {
    match bt {
        BlockType::Park => CODE_PARK,
        BlockType::Water => CODE_WATER,
        BlockType::Riverbank => CODE_RIVERBANK,
        BlockType::Plaza => CODE_PLAZA,
        BlockType::Building => CODE_GRASS,
    }
}

/// Rasterise the district's zones, rivers and roads into a [`GroundPaint`].
pub fn paint_ground(
    blocks: &[Block],
    roads: &RoadNetwork,
    rivers: &[RiverSegment],
    district: &crate::types::Polygon,
) -> GroundPaint {
    let (min_x, min_y, max_x, max_y) = district.bbox();
    let cx = (min_x + max_x) * 0.5;
    let cz = (min_y + max_y) * 0.5;
    let size = ((max_x - min_x).max(max_y - min_y) * 1.05).max(1.0);
    let ox = cx - size * 0.5;
    let oz = cz - size * 0.5;
    let res = ((size / 2.5) as usize).clamp(64, 140);

    let cfg = config();
    let road_half_sq = cfg.road_paint_half * cfg.road_paint_half;
    let water_sq = cfg.river_paint_half * cfg.river_paint_half;
    let bank = cfg.river_paint_half + cfg.river_paint_bank;
    let bank_sq = bank * bank;
    // Pre-flatten river bezier curves to polylines once.
    let river_lines: Vec<Vec<Point>> = rivers.iter().map(|r| r.to_polyline(48)).collect();
    let mut codes = vec![CODE_GRASS; res * res];

    for j in 0..res {
        for i in 0..res {
            let p = Point::new(
                ox + (i as f64 + 0.5) / res as f64 * size,
                oz + (j as f64 + 0.5) / res as f64 * size,
            );

            // Zone code from the block this cell falls in.
            let mut code = CODE_GRASS;
            for block in blocks {
                if block.polygon.contains(p) {
                    code = block_code(block.block_type);
                    break;
                }
            }

            // Rivers paint over zones: water channel with a riverbank margin.
            let river_d2 = river_lines
                .iter()
                .flat_map(|line| line.windows(2))
                .map(|w| p.distance_sq_to_segment(w[0], w[1]))
                .fold(f64::MAX, f64::min);
            if river_d2 < water_sq {
                code = CODE_WATER;
            } else if river_d2 < bank_sq && code != CODE_WATER {
                // Riverbank only forms where the river meets land. Where the river
                // runs through a lake (already water), there is no bank.
                code = CODE_RIVERBANK;
            }

            // Roads paint over everything (a bridge across a river).
            let on_road = roads
                .segments
                .iter()
                .any(|s| p.distance_sq_to_segment(s.a, s.b) < road_half_sq);
            if on_road {
                code = CODE_ROAD;
            }

            codes[j * res + i] = code;
        }
    }

    // Cleanup: a riverbank only forms where the river meets land. A bank cell
    // with no land neighbour is wedged between waters (the river running through
    // a lake, or a thin spit between the channel and a lake) — there is no bank
    // there, it is water. Decided against a snapshot so the pass is independent
    // of scan order and matches the `riverbank_not_in_water` audit invariant.
    let snapshot = codes.clone();
    let is_land = |c: u8| c != CODE_WATER && c != CODE_RIVERBANK;
    for j in 0..res as i32 {
        for i in 0..res as i32 {
            if snapshot[(j * res as i32 + i) as usize] != CODE_RIVERBANK {
                continue;
            }
            let touches_land = [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|&(di, dj)| {
                let (ni, nj) = (i + di, j + dj);
                // Off-grid (district edge) counts as land — no spurious conversion.
                ni < 0
                    || nj < 0
                    || ni >= res as i32
                    || nj >= res as i32
                    || is_land(snapshot[(nj * res as i32 + ni) as usize])
            });
            if !touches_land {
                codes[(j * res as i32 + i) as usize] = CODE_WATER;
            }
        }
    }

    GroundPaint {
        ox,
        oz,
        size,
        res,
        codes: base64::engine::general_purpose::STANDARD.encode(&codes),
    }
}

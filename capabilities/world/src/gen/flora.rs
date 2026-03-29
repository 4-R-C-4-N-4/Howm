//! Flora generation — block-level, road-edge, and surface growth.
//!
//! Flora is Tier 0: static positions, seed-derived form. Wind-driven
//! sway is client-local animation (Tier 0). Wind direction/intensity
//! is Tier 1 (time-synced via atmosphere.rs).

use serde::{Deserialize, Serialize};

use super::blocks::{Block, BlockType};
use super::cell::Cell;
use super::config::config;
use super::hash::{ha, hash_to_f64};
use super::objects::{compute_form_id, compute_object_id, ObjectSeeds, Tier};
use super::roads::RoadNetwork;
use super::zones::{generate_zones, point_in_polygon_seeded};
use crate::types::Point;

/// Flora growth form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrowthForm {
    Tree,
    Shrub,
    GroundCover,
    Vine,
    Fungal,
    Aquatic,
    Crystalline,
}

/// Flora density mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DensityMode {
    Sparse,
    Moderate,
    Dense,
    Canopy,
}

/// Flora placement context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FloraContext {
    BlockLevel,
    RoadEdge,
    SurfaceGrowth,
}

/// A flora instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flora {
    pub context: FloraContext,
    pub growth_form: GrowthForm,
    pub density_mode: DensityMode,
    pub position: Point,
    pub orientation: f64,
    pub scale: f64,
    pub maturity: f64,
    pub shedding: bool,
    pub form_id: u32,
    pub object_id: u64,
    pub seeds: ObjectSeeds,
    pub tier: Tier,
}

/// Result of flora generation for one block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockFlora {
    pub block_idx: usize,
    pub block_flora: Vec<Flora>,
    pub road_flora: Vec<Flora>,
    pub surface_growth: Vec<Flora>,
}

/// Derive growth form from block type, domain, and seed.
fn derive_growth_form(block_type: BlockType, seed: u32, inverted_age: f64) -> GrowthForm {
    let roll = seed & 0xFF;
    match block_type {
        BlockType::Water | BlockType::Riverbank => {
            if roll < 80 { GrowthForm::Aquatic }
            else if roll < 160 { GrowthForm::GroundCover }
            else { GrowthForm::Vine }
        }
        BlockType::Park => {
            if roll < 60 { GrowthForm::Tree }
            else if roll < 120 { GrowthForm::Shrub }
            else if roll < 180 { GrowthForm::GroundCover }
            else if inverted_age > 0.7 { GrowthForm::Fungal }
            else { GrowthForm::Vine }
        }
        _ => {
            if roll < 40 { GrowthForm::Tree }
            else if roll < 100 { GrowthForm::Shrub }
            else if roll < 180 { GrowthForm::GroundCover }
            else if inverted_age > 0.6 { GrowthForm::Crystalline }
            else { GrowthForm::Vine }
        }
    }
}

/// Derive density mode from popcount ratio.
fn derive_density(popcount_ratio: f64, seed: u32) -> DensityMode {
    let jitter = hash_to_f64(seed) * 0.2 - 0.1;
    let eff = popcount_ratio + jitter;
    if eff < 0.25 { DensityMode::Sparse }
    else if eff < 0.50 { DensityMode::Moderate }
    else if eff < 0.75 { DensityMode::Dense }
    else { DensityMode::Canopy }
}

/// Flora count per zone for a block type.
fn flora_zone_count(block_type: BlockType, density: f64) -> u32 {
    let (base, bonus) = match block_type {
        BlockType::Park => (3, 4),
        BlockType::Riverbank => (2, 3),
        BlockType::Plaza => (1, 2),
        BlockType::Water => (1, 1),
        BlockType::Building => (0, 1),
    };
    base + (density * bonus as f64).floor() as u32
}

/// Build a single flora instance.
fn build_flora(
    cell: &Cell,
    context: FloraContext,
    block_type: BlockType,
    pos_seed: u32,
    position: Point,
) -> Flora {
    let orient_seed = ha(pos_seed ^ 0x1);
    let orientation = hash_to_f64(orient_seed) * std::f64::consts::TAU;
    let object_seed = ha(pos_seed ^ 0x2);
    let seeds = ObjectSeeds::from_seed(object_seed);

    let growth_form = derive_growth_form(block_type, seeds.form_seed, cell.inverted_age);
    let density_mode = derive_density(cell.popcount_ratio, ha(seeds.form_seed ^ 0x5));

    // Scale: trees are bigger, ground cover smaller
    let base_scale = match growth_form {
        GrowthForm::Tree => 2.0 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 3.0,
        GrowthForm::Shrub => 0.8 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 1.2,
        GrowthForm::GroundCover => 0.2 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 0.5,
        GrowthForm::Vine => 1.0 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 2.0,
        GrowthForm::Fungal => 0.3 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 0.8,
        GrowthForm::Aquatic => 0.5 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 1.5,
        GrowthForm::Crystalline => 0.4 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 1.0,
    };

    // Maturity: 0.0 = seedling, 1.0 = fully mature. Driven by age axis.
    let maturity = (cell.age * 0.6 + hash_to_f64(ha(seeds.state_seed ^ 0x20)) * 0.4).min(1.0);

    // Shedding: more likely in ancient districts
    let shedding = hash_to_f64(ha(seeds.state_seed ^ 0x30)) < cell.inverted_age * 0.3;

    let form_id = compute_form_id(
        &format!("flora:{}", match growth_form {
            GrowthForm::Tree => "tree",
            GrowthForm::Shrub => "shrub",
            GrowthForm::GroundCover => "ground_cover",
            GrowthForm::Vine => "vine",
            GrowthForm::Fungal => "fungal",
            GrowthForm::Aquatic => "aquatic",
            GrowthForm::Crystalline => "crystalline",
        }),
        cell.aesthetic_bucket(),
        object_seed,
    );
    let object_id = compute_object_id(cell.key, seeds.object_seed);

    Flora {
        context,
        growth_form,
        density_mode,
        position,
        orientation,
        scale: base_scale,
        maturity,
        shedding,
        form_id,
        object_id,
        seeds,
        tier: Tier::Seedable,
    }
}

/// Generate flora for a block.
pub fn generate_flora(
    cell: &Cell,
    block: &Block,
    road_network: Option<&RoadNetwork>,
) -> BlockFlora {
    let cfg = config();
    let zones = generate_zones(cell.key, block);
    let mut block_flora = Vec::new();

    // Block-level flora via zones
    for zone in &zones {
        let count = flora_zone_count(block.block_type, zone.density);
        for i in 0..count {
            let pos_seed = ha(zone.seed ^ 0xF1 ^ i ^ 0);
            let position = point_in_polygon_seeded(&zone.polygon, pos_seed);
            let flora = build_flora(cell, FloraContext::BlockLevel, block.block_type, pos_seed, position);
            block_flora.push(flora);
        }
    }

    // Road-edge flora (street trees/hedges)
    let mut road_flora = Vec::new();
    if let Some(network) = road_network {
        let spacing = cfg.min_flora_spacing
            + hash_to_f64(ha(cell.key ^ 0xf10ea)) * (cfg.max_flora_spacing - cfg.min_flora_spacing);

        for (road_idx, segment) in network.segments.iter().enumerate() {
            let seg_len = segment.a.distance_to(segment.b);
            let count = (seg_len / spacing).floor().max(0.0) as u32;

            for i in 0..count {
                let t = (i as f64 + 0.5) / count.max(1) as f64;
                let base_pos = segment.a.lerp(segment.b, t);

                let dx = segment.b.x - segment.a.x;
                let dy = segment.b.y - segment.a.y;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-10 { continue; }
                let perp_x = -dy / len;
                let perp_y = dx / len;

                let side = if ha(cell.key ^ road_idx as u32 ^ i ^ 0xf10e) & 1 == 0 { 1.0 } else { -1.0 };
                let offset = cfg.lamp_offset + 1.5; // Flora slightly further than lamps
                let position = Point::new(
                    base_pos.x + perp_x * offset * side,
                    base_pos.y + perp_y * offset * side,
                );

                let pos_seed = ha(cell.key ^ road_idx as u32 ^ i ^ 0xf10ea5);
                let flora = build_flora(cell, FloraContext::RoadEdge, block.block_type, pos_seed, position);
                road_flora.push(flora);
            }
        }
    }

    // Surface growth (moss, ivy on ancient buildings)
    let mut surface_growth = Vec::new();
    if cell.inverted_age > cfg.surface_growth_age_threshold
        && matches!(block.block_type, BlockType::Building | BlockType::Plaza)
    {
        let growth_count = (cell.inverted_age * 3.0).floor() as u32;
        for i in 0..growth_count {
            let pos_seed = ha(cell.key ^ block.idx as u32 ^ i ^ 0x5afe);
            let position = point_in_polygon_seeded(&block.polygon, pos_seed);
            let flora = build_flora(cell, FloraContext::SurfaceGrowth, block.block_type, pos_seed, position);
            surface_growth.push(flora);
        }
    }

    BlockFlora {
        block_idx: block.idx,
        block_flora,
        road_flora,
        surface_growth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::blocks::BlockType;
    use crate::types::Polygon;

    fn test_block(bt: BlockType) -> Block {
        Block {
            idx: 0,
            polygon: Polygon::new(vec![
                Point::new(0.0, 0.0), Point::new(80.0, 0.0),
                Point::new(80.0, 80.0), Point::new(0.0, 80.0),
            ]),
            block_type: bt,
            area: 6400.0,
            centroid: Point::new(40.0, 40.0),
            river_adjacent: false,
        }
    }

    #[test]
    fn flora_generated_park() {
        let cell = Cell::from_octets(93, 184, 216);
        let result = generate_flora(&cell, &test_block(BlockType::Park), None);
        assert!(!result.block_flora.is_empty(), "Park should have flora");
    }

    #[test]
    fn flora_generated_building() {
        let cell = Cell::from_octets(93, 184, 216);
        let result = generate_flora(&cell, &test_block(BlockType::Building), None);
        // Building blocks may have 0 or few block-level flora
        let _ = result;
    }

    #[test]
    fn surface_growth_on_ancient() {
        let cell = Cell::from_octets(1, 0, 0); // ancient (low age sum = high inverted_age)
        let result = generate_flora(&cell, &test_block(BlockType::Building), None);
        assert!(!result.surface_growth.is_empty(), "Ancient building should have surface growth");
    }

    #[test]
    fn flora_deterministic() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Park);
        let r1 = generate_flora(&cell, &block, None);
        let r2 = generate_flora(&cell, &block, None);
        assert_eq!(r1.block_flora.len(), r2.block_flora.len());
        for (a, b) in r1.block_flora.iter().zip(r2.block_flora.iter()) {
            assert_eq!(a.object_id, b.object_id);
        }
    }

    // Spec test vectors from Appendix D.2
    #[test]
    fn flora_pos_seeds() {
        // 1.0.0.0 zone_0_seed = 0x49ab0b9a
        assert_eq!(ha(0x49ab0b9a ^ 0xF1 ^ 0 ^ 0), 0xe39e2401);
        // 254.254.254.0 zone_0_seed = 0xe2f5da1c
        assert_eq!(ha(0xe2f5da1c ^ 0xF1 ^ 0 ^ 0), 0xfe0fdf71);
    }

    #[test]
    fn flora_object_seeds() {
        assert_eq!(ha(0xe39e2401 ^ 0x2), 0x4f7ea502);
        assert_eq!(ha(0xfe0fdf71 ^ 0x2), 0x13c74d87);
    }
}

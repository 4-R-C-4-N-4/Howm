//! Fixture spawn pipeline — 8 roles, spawn counts, road-edge fixtures.
//!
//! Follows §6 of howm-objects-spec.md. Fixtures are Tier 0 objects:
//! static, fully deterministic, zero persistence.

use serde::{Deserialize, Serialize};

use super::blocks::{Block, BlockType};
use super::cell::Cell;
use super::config::config;
use super::hash::{ha, hash_to_f64};
use super::objects::{
    compute_form_id, compute_object_id, Attachment, FormClass, Hazard, ObjectSeeds, Tier,
};
use super::roads::RoadNetwork;
use super::zones::{generate_zones, point_in_polygon_seeded};
use crate::types::Point;

/// Fixture role identifiers per §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum FixtureRole {
    Illumination = 0x01,
    Seating = 0x02,
    BoundaryMarker = 0x03,
    NavigationAid = 0x04,
    UtilityNode = 0x05,
    DisplaySurface = 0x06,
    OfferingPoint = 0x07,
    Ornament = 0x08,
    WaterStructure = 0x09,
}

impl FixtureRole {
    pub fn id(self) -> u32 {
        self as u32
    }

    pub fn archetype_str(self) -> &'static str {
        match self {
            Self::Illumination => "fixture:illumination",
            Self::Seating => "fixture:seating",
            Self::BoundaryMarker => "fixture:boundary_marker",
            Self::NavigationAid => "fixture:navigation_aid",
            Self::UtilityNode => "fixture:utility_node",
            Self::DisplaySurface => "fixture:display_surface",
            Self::OfferingPoint => "fixture:offering_point",
            Self::Ornament => "fixture:ornament",
            Self::WaterStructure => "fixture:water_structure",
        }
    }

    /// All fixture roles.
    pub fn all() -> &'static [FixtureRole] {
        &[
            Self::Illumination,
            Self::Seating,
            Self::BoundaryMarker,
            Self::NavigationAid,
            Self::UtilityNode,
            Self::DisplaySurface,
            Self::OfferingPoint,
            Self::Ornament,
            Self::WaterStructure,
        ]
    }
}

/// Spawn count table: (base_count, bonus_count) per block type and role.
/// From §6.5 of howm-objects-spec.md.
fn spawn_counts(role: FixtureRole, block_type: BlockType) -> (u32, u32) {
    match (role, block_type) {
        // (base, bonus) — building, park, plaza
        (FixtureRole::Illumination, BlockType::Building) => (1, 1),
        (FixtureRole::Illumination, BlockType::Park) => (1, 1),
        (FixtureRole::Illumination, BlockType::Plaza) => (2, 1),

        (FixtureRole::Seating, BlockType::Building) => (0, 1),
        (FixtureRole::Seating, BlockType::Park) => (1, 2),
        (FixtureRole::Seating, BlockType::Plaza) => (1, 2),

        (FixtureRole::BoundaryMarker, BlockType::Building) => (1, 0),
        (FixtureRole::BoundaryMarker, BlockType::Park) => (0, 1),
        (FixtureRole::BoundaryMarker, BlockType::Plaza) => (1, 1),

        (FixtureRole::NavigationAid, BlockType::Building) => (0, 1),
        (FixtureRole::NavigationAid, BlockType::Park) => (0, 1),
        (FixtureRole::NavigationAid, BlockType::Plaza) => (1, 1),

        (FixtureRole::UtilityNode, BlockType::Building) => (1, 1),
        (FixtureRole::UtilityNode, BlockType::Park) => (0, 1),
        (FixtureRole::UtilityNode, BlockType::Plaza) => (0, 0),

        (FixtureRole::DisplaySurface, BlockType::Building) => (1, 2),
        (FixtureRole::DisplaySurface, BlockType::Park) => (0, 1),
        (FixtureRole::DisplaySurface, BlockType::Plaza) => (1, 2),

        (FixtureRole::OfferingPoint, BlockType::Building) => (0, 1),
        (FixtureRole::OfferingPoint, BlockType::Park) => (0, 1),
        (FixtureRole::OfferingPoint, BlockType::Plaza) => (1, 1),

        (FixtureRole::Ornament, BlockType::Building) => (0, 1),
        (FixtureRole::Ornament, BlockType::Park) => (1, 2),
        (FixtureRole::Ornament, BlockType::Plaza) => (1, 2),

        (FixtureRole::WaterStructure, BlockType::Building) => (0, 0),
        (FixtureRole::WaterStructure, BlockType::Park) => (0, 1),
        (FixtureRole::WaterStructure, BlockType::Plaza) => (0, 1),

        // Water and riverbank get reduced fixture sets
        (FixtureRole::WaterStructure, BlockType::Water) => (1, 0),
        (FixtureRole::Illumination, BlockType::Riverbank) => (1, 0),
        (FixtureRole::BoundaryMarker, BlockType::Riverbank) => (0, 1),

        // Default: no fixtures for unspecified combinations
        _ => (0, 0),
    }
}

/// A spawned fixture instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub role: FixtureRole,
    pub position: Point,
    pub orientation: f64,
    pub form_class: FormClass,
    pub attachment: Attachment,
    pub scale_height: f64,
    pub scale_footprint: f64,
    pub hazard: Hazard,
    pub active_state: bool,
    pub emissive_light: bool,
    pub emissive_sound: bool,
    pub emissive_particles: bool,
    pub form_id: u32,
    pub object_id: u64,
    pub seeds: ObjectSeeds,
    pub tier: Tier,
    /// True if this is a road-edge fixture.
    pub road_edge: bool,
}

/// Result of fixture generation for one block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockFixtures {
    pub block_idx: usize,
    pub zone_fixtures: Vec<Fixture>,
    pub road_fixtures: Vec<Fixture>,
}

/// Derive form_class from role and seed.
fn derive_form_class(role: FixtureRole, seed: u32) -> FormClass {
    let pool: &[FormClass] = match role {
        FixtureRole::Illumination => &[FormClass::Column, FormClass::Compound, FormClass::Growth],
        FixtureRole::Seating => &[FormClass::Platform, FormClass::Enclosure, FormClass::Surface],
        FixtureRole::BoundaryMarker => &[FormClass::Column, FormClass::Surface, FormClass::Growth],
        FixtureRole::NavigationAid => &[FormClass::Column, FormClass::Surface, FormClass::Compound],
        FixtureRole::UtilityNode => &[FormClass::Container, FormClass::Compound, FormClass::Column],
        FixtureRole::DisplaySurface => &[FormClass::Surface, FormClass::Column, FormClass::Compound],
        FixtureRole::OfferingPoint => &[FormClass::Container, FormClass::Platform, FormClass::Enclosure],
        FixtureRole::Ornament => &[FormClass::Column, FormClass::Growth, FormClass::Compound, FormClass::Span],
        FixtureRole::WaterStructure => &[FormClass::Container, FormClass::Enclosure, FormClass::Surface],
    };
    pool[seed as usize % pool.len()]
}

/// Derive attachment from form_class and seed.
fn derive_attachment(form_class: FormClass, seed: u32) -> Attachment {
    match form_class {
        FormClass::Column => Attachment::Floor,
        FormClass::Platform => Attachment::Floor,
        FormClass::Enclosure => Attachment::Freestanding,
        FormClass::Surface => {
            if seed & 1 == 0 { Attachment::Wall } else { Attachment::Floor }
        }
        FormClass::Container => Attachment::Floor,
        FormClass::Span => Attachment::Hanging,
        FormClass::Compound => Attachment::Freestanding,
        FormClass::Growth => Attachment::SurfaceGrowth,
    }
}

/// Build a single fixture from spawn pipeline parameters.
fn build_fixture(
    cell: &Cell,
    role: FixtureRole,
    pos_seed: u32,
    position: Point,
    road_edge: bool,
) -> Fixture {
    let orient_seed = ha(pos_seed ^ 0x1);
    let orientation = hash_to_f64(orient_seed) * std::f64::consts::TAU;
    let object_seed = ha(pos_seed ^ 0x2);
    let seeds = ObjectSeeds::from_seed(object_seed);

    let form_class = derive_form_class(role, seeds.form_seed);
    let attachment = derive_attachment(form_class, seeds.form_seed);

    // Scale derivation
    let base_height = 1.0 + hash_to_f64(ha(seeds.form_seed ^ 0x10)) * 3.0;
    let base_footprint = 0.3 + hash_to_f64(ha(seeds.form_seed ^ 0x11)) * 1.5;

    // Hazard — most fixtures are safe
    let hazard_roll = ha(seeds.state_seed ^ 0x20) & 0xFF;
    let hazard = if hazard_roll < 8 {
        Hazard::Damage
    } else if hazard_roll < 16 {
        Hazard::Impede
    } else if hazard_roll < 24 {
        Hazard::Repel
    } else {
        Hazard::None
    };

    // Active state (Tier 0: constant from seed)
    let active_state = (ha(seeds.state_seed ^ 0x30) & 3) != 0; // 75% active

    // Emissive properties by role
    let emissive_light = matches!(role, FixtureRole::Illumination | FixtureRole::DisplaySurface);
    let emissive_sound = matches!(role, FixtureRole::WaterStructure);
    let emissive_particles = matches!(role, FixtureRole::WaterStructure | FixtureRole::OfferingPoint)
        && (ha(seeds.state_seed ^ 0x40) & 1 == 1);

    let form_id = compute_form_id(
        role.archetype_str(),
        cell.aesthetic_bucket(),
        object_seed,
    );
    let object_id = compute_object_id(cell.key, seeds.object_seed);

    Fixture {
        role,
        position,
        orientation,
        form_class,
        attachment,
        scale_height: base_height,
        scale_footprint: base_footprint,
        hazard,
        active_state,
        emissive_light,
        emissive_sound,
        emissive_particles,
        form_id,
        object_id,
        seeds,
        tier: Tier::Seedable,
        road_edge,
    }
}

/// Generate fixtures for a block using the full spawn pipeline per §6.6.
pub fn generate_fixtures(
    cell: &Cell,
    block: &Block,
    road_network: Option<&RoadNetwork>,
) -> BlockFixtures {
    let zones = generate_zones(cell.key, block);
    let mut zone_fixtures = Vec::new();

    // Zone-based fixture spawning
    for zone in &zones {
        for &role in FixtureRole::all() {
            let (base, bonus) = spawn_counts(role, block.block_type);
            let count = base + (zone.density * bonus as f64).floor() as u32;
            if count == 0 {
                continue;
            }

            for i in 0..count {
                // time_slot = 0 for Tier 0 (reseed_interval = ∞ for fixtures)
                let pos_seed = ha(zone.seed ^ role.id() ^ i ^ 0);
                let position = point_in_polygon_seeded(&zone.polygon, pos_seed);
                let fixture = build_fixture(cell, role, pos_seed, position, false);
                zone_fixtures.push(fixture);
            }
        }
    }

    // Road-edge fixture spawning (illumination + navigation_aid)
    let mut road_fixtures = Vec::new();
    if let Some(network) = road_network {
        let cfg = config();
        for (road_idx, segment) in network.segments.iter().enumerate() {
            let seg_len = segment.a.distance_to(segment.b);
            let spacing = cfg.lamp_spacing_base + (ha(cell.key ^ road_idx as u32) & 0xF) as f64;
            let lamp_count = (seg_len / spacing).floor().max(1.0) as u32;

            for i in 0..lamp_count {
                let t = (i as f64 + 0.5) / lamp_count as f64;
                let base_pos = Point::new(
                    segment.a.x + t * (segment.b.x - segment.a.x),
                    segment.a.y + t * (segment.b.y - segment.a.y),
                );

                // Perpendicular direction
                let dx = segment.b.x - segment.a.x;
                let dy = segment.b.y - segment.a.y;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-10 {
                    continue;
                }
                let perp_x = -dy / len;
                let perp_y = dx / len;

                // Side selection
                let side_seed = ha(cell.key ^ road_idx as u32 ^ i);
                let side = if side_seed & 1 == 0 { 1.0 } else { -1.0 };
                let position = Point::new(
                    base_pos.x + perp_x * cfg.lamp_offset * side,
                    base_pos.y + perp_y * cfg.lamp_offset * side,
                );

                let pos_seed = ha(cell.key ^ road_idx as u32 ^ i ^ 0x1a4b);
                let fixture = build_fixture(cell, FixtureRole::Illumination, pos_seed, position, true);
                road_fixtures.push(fixture);
            }
        }
    }

    BlockFixtures {
        block_idx: block.idx,
        zone_fixtures,
        road_fixtures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::blocks::BlockType;
    use crate::types::Polygon;

    fn test_block(block_type: BlockType) -> Block {
        Block {
            idx: 0,
            polygon: Polygon::new(vec![
                Point::new(0.0, 0.0),
                Point::new(80.0, 0.0),
                Point::new(80.0, 80.0),
                Point::new(0.0, 80.0),
            ]),
            block_type,
            area: 6400.0,
            centroid: Point::new(40.0, 40.0),
            river_adjacent: false,
        }
    }

    #[test]
    fn fixtures_generated_building_block() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Building);
        let result = generate_fixtures(&cell, &block, None);
        assert!(
            !result.zone_fixtures.is_empty(),
            "No zone fixtures for building block"
        );
    }

    #[test]
    fn fixtures_generated_park_block() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Park);
        let result = generate_fixtures(&cell, &block, None);
        assert!(
            !result.zone_fixtures.is_empty(),
            "No zone fixtures for park block"
        );
    }

    #[test]
    fn fixtures_generated_plaza_block() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Plaza);
        let result = generate_fixtures(&cell, &block, None);
        assert!(
            !result.zone_fixtures.is_empty(),
            "No zone fixtures for plaza block"
        );
    }

    #[test]
    fn fixture_roles_present() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Plaza);
        let result = generate_fixtures(&cell, &block, None);
        let roles: std::collections::HashSet<_> =
            result.zone_fixtures.iter().map(|f| f.role).collect();
        assert!(roles.contains(&FixtureRole::Illumination));
        assert!(roles.contains(&FixtureRole::Seating));
        assert!(roles.contains(&FixtureRole::Ornament));
    }

    #[test]
    fn fixtures_deterministic() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Building);
        let r1 = generate_fixtures(&cell, &block, None);
        let r2 = generate_fixtures(&cell, &block, None);
        assert_eq!(r1.zone_fixtures.len(), r2.zone_fixtures.len());
        for (a, b) in r1.zone_fixtures.iter().zip(r2.zone_fixtures.iter()) {
            assert_eq!(a.object_id, b.object_id);
            assert_eq!(a.position.x, b.position.x);
            assert_eq!(a.position.y, b.position.y);
        }
    }

    #[test]
    fn illumination_always_emissive_light() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Plaza);
        let result = generate_fixtures(&cell, &block, None);
        for f in &result.zone_fixtures {
            if f.role == FixtureRole::Illumination {
                assert!(f.emissive_light, "Illumination fixture should emit light");
            }
        }
    }

    #[test]
    fn fixture_positions_inside_block() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Building);
        let result = generate_fixtures(&cell, &block, None);
        for f in &result.zone_fixtures {
            assert!(
                f.position.x >= -5.0 && f.position.x <= 85.0
                    && f.position.y >= -5.0 && f.position.y <= 85.0,
                "Fixture at {:?} outside block bounds",
                f.position,
            );
        }
    }

    // ── Spec test vectors from Appendix B.2 ──

    #[test]
    fn fixture_pos_seeds_93_184_216() {
        // zone_0_seed for 93.184.216.0 = 0x86eaf091
        // These verify the spawn position seed derivation:
        // pos_seed = ha(zone_seed ^ role_id ^ spawn_idx ^ time_slot)
        let zone_seed: u32 = 0x86eaf091;
        assert_eq!(ha(zone_seed ^ 0x01 ^ 0 ^ 0), 0x0b813c94); // illumination
        assert_eq!(ha(zone_seed ^ 0x03 ^ 0 ^ 0), 0x2bc848e7); // boundary_marker
        assert_eq!(ha(zone_seed ^ 0x06 ^ 0 ^ 0), 0xc00689a4); // display_surface
        assert_eq!(ha(zone_seed ^ 0x08 ^ 0 ^ 0), 0x795fa0ff); // ornament
    }

    #[test]
    fn water_block_gets_water_structure() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = Block {
            idx: 0,
            polygon: Polygon::new(vec![
                Point::new(0.0, 0.0),
                Point::new(80.0, 0.0),
                Point::new(80.0, 80.0),
                Point::new(0.0, 80.0),
            ]),
            block_type: BlockType::Water,
            area: 6400.0,
            centroid: Point::new(40.0, 40.0),
            river_adjacent: true,
        };
        let result = generate_fixtures(&cell, &block, None);
        let has_water = result
            .zone_fixtures
            .iter()
            .any(|f| f.role == FixtureRole::WaterStructure);
        assert!(has_water, "Water block should have water_structure fixtures");
    }
}

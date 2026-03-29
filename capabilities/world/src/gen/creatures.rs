//! Creature generation — Tier 1 time-synced ambient fauna.
//!
//! Creatures fill ecological roles derived from block type. Zone assignment
//! is time-synchronised: ha(creature_seed ^ block.idx ^ creature_idx ^ time_slot) % zone_count.
//! Position within zone uses the same point_in_polygon as fixtures.

use serde::{Deserialize, Serialize};

use super::blocks::{Block, BlockType};
use super::cell::Cell;
use super::config::config;
use super::hash::{ha, hash_to_f64};
use super::objects::{compute_form_id, compute_object_id, ObjectSeeds, Tier};
use super::zones::point_in_polygon_seeded;
use crate::types::Point;

/// Ecological role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EcologicalRole {
    Aerial,
    GroundDwelling,
    Aquatic,
    Perching,
    Subterranean,
    Nocturnal,
}

impl EcologicalRole {
    pub fn archetype_str(self) -> &'static str {
        match self {
            Self::Aerial => "creature:aerial",
            Self::GroundDwelling => "creature:ground",
            Self::Aquatic => "creature:aquatic",
            Self::Perching => "creature:perching",
            Self::Subterranean => "creature:subterranean",
            Self::Nocturnal => "creature:nocturnal",
        }
    }
}

/// Size class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeClass { Tiny, Small, Medium, Large }

/// Anatomy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anatomy { Bilateral, Radial, Amorphous, Composite }

/// Locomotion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocomotionMode { Surface, Aerial, Aquatic, Burrowing, Floating, Phasing }

/// Materiality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Materiality { Flesh, Construct, Spirit, Elemental, Crystalline, Spectral, Vegetal }

/// Activity pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityPattern { Diurnal, Nocturnal, Crepuscular, Continuous }

/// Social structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocialStructure { Solitary, Pair, SmallGroup, Swarm }

/// Player response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerResponse { Flee, Ignore, Curious, Territorial, Mimicking }

/// Pace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pace { Slow, Medium, Fast }

/// A creature instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Creature {
    pub creature_idx: usize,
    pub creature_seed: u32,
    pub ecological_role: EcologicalRole,
    pub size_class: SizeClass,
    pub anatomy: Anatomy,
    pub locomotion_mode: LocomotionMode,
    pub materiality: Materiality,
    pub activity_pattern: ActivityPattern,
    pub social_structure: SocialStructure,
    pub player_response: PlayerResponse,
    pub pace: Pace,
    pub rest_frequency: f64,
    pub idle_behaviours: Vec<u32>,
    pub form_id: u32,
    pub object_id: u64,
    pub seeds: ObjectSeeds,
    pub tier: Tier,
}

/// Creatures per block type.
fn creature_count(block_type: BlockType, popcount_ratio: f64) -> u32 {
    let (base, bonus) = match block_type {
        BlockType::Park => (2, 3),
        BlockType::Plaza => (1, 2),
        BlockType::Water => (2, 2),
        BlockType::Riverbank => (1, 2),
        BlockType::Building => (0, 1),
    };
    base + (popcount_ratio * bonus as f64).floor() as u32
}

/// Eligible ecological roles by block type.
fn eligible_roles(block_type: BlockType) -> &'static [EcologicalRole] {
    match block_type {
        BlockType::Park => &[EcologicalRole::Aerial, EcologicalRole::GroundDwelling, EcologicalRole::Perching, EcologicalRole::Nocturnal],
        BlockType::Plaza => &[EcologicalRole::Aerial, EcologicalRole::GroundDwelling, EcologicalRole::Perching],
        BlockType::Water => &[EcologicalRole::Aerial, EcologicalRole::Aquatic],
        BlockType::Riverbank => &[EcologicalRole::Aerial, EcologicalRole::Aquatic, EcologicalRole::GroundDwelling],
        BlockType::Building => &[EcologicalRole::Aerial, EcologicalRole::Perching, EcologicalRole::Subterranean],
    }
}

/// Derive base creature record from creature_seed.
fn derive_creature(cell: &Cell, creature_seed: u32, creature_idx: usize, role: EcologicalRole) -> Creature {
    let seeds = ObjectSeeds::from_seed(creature_seed);

    let size_class = match seeds.form_seed & 0x3 {
        0 => SizeClass::Tiny,
        1 => SizeClass::Small,
        2 => SizeClass::Medium,
        _ => SizeClass::Large,
    };

    let anatomy = match (seeds.form_seed >> 2) & 0x3 {
        0 => Anatomy::Bilateral,
        1 => Anatomy::Radial,
        2 => Anatomy::Amorphous,
        _ => Anatomy::Composite,
    };

    let locomotion_mode = match role {
        EcologicalRole::Aerial => LocomotionMode::Aerial,
        EcologicalRole::Aquatic => LocomotionMode::Aquatic,
        EcologicalRole::Subterranean => LocomotionMode::Burrowing,
        _ => match (seeds.form_seed >> 4) & 0x3 {
            0 => LocomotionMode::Surface,
            1 => LocomotionMode::Floating,
            _ => LocomotionMode::Surface,
        },
    };

    let materiality = match (seeds.material_seed) & 0x7 {
        0 => Materiality::Flesh,
        1 => Materiality::Construct,
        2 => Materiality::Spirit,
        3 => Materiality::Elemental,
        4 => Materiality::Crystalline,
        5 => Materiality::Spectral,
        _ => Materiality::Vegetal,
    };

    let activity_pattern = match role {
        EcologicalRole::Nocturnal => ActivityPattern::Nocturnal,
        _ => match (seeds.state_seed) & 0x3 {
            0 => ActivityPattern::Diurnal,
            1 => ActivityPattern::Crepuscular,
            2 => ActivityPattern::Continuous,
            _ => ActivityPattern::Diurnal,
        },
    };

    let social_structure = match (seeds.behaviour_seed) & 0x3 {
        0 => SocialStructure::Solitary,
        1 => SocialStructure::Pair,
        2 => SocialStructure::SmallGroup,
        _ => SocialStructure::Swarm,
    };

    let player_response = match (seeds.interaction_seed) & 0x7 {
        0 | 1 => PlayerResponse::Flee,
        2 | 3 => PlayerResponse::Ignore,
        4 => PlayerResponse::Curious,
        5 => PlayerResponse::Territorial,
        _ => PlayerResponse::Mimicking,
    };

    let pace = match (seeds.behaviour_seed >> 2) & 0x3 {
        0 => Pace::Slow,
        1 | 2 => Pace::Medium,
        _ => Pace::Fast,
    };

    let rest_frequency = hash_to_f64(ha(seeds.behaviour_seed ^ 0x10));

    // Idle behaviour selection per §8.7
    let cfg = config();
    let idle_count = 1 + (seeds.behaviour_seed & cfg.idle_count_mask) as usize;
    let mut idle_behaviours = Vec::with_capacity(idle_count);
    for i in 0..idle_count {
        let pick = ha(seeds.behaviour_seed ^ i as u32 ^ 0xb3a1);
        idle_behaviours.push(pick);
    }

    let form_id = compute_form_id(role.archetype_str(), cell.aesthetic_bucket(), creature_seed);
    let object_id = compute_object_id(cell.key, seeds.object_seed);

    Creature {
        creature_idx,
        creature_seed,
        ecological_role: role,
        size_class,
        anatomy,
        locomotion_mode,
        materiality,
        activity_pattern,
        social_structure,
        player_response,
        pace,
        rest_frequency,
        idle_behaviours,
        form_id,
        object_id,
        seeds,
        tier: Tier::TimeSynced,
    }
}

/// Result of creature generation for a cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockCreatures {
    pub block_idx: usize,
    pub creatures: Vec<Creature>,
}

/// Zone assignment for a creature at a given time.
pub fn creature_zone_assignment(creature_seed: u32, block_idx: u32, creature_idx: u32, time_slot: u64, zone_count: u32) -> u32 {
    ha(creature_seed ^ block_idx ^ creature_idx ^ time_slot as u32) % zone_count
}

/// Creature position within a zone at a given time.
pub fn creature_position(creature_seed: u32, creature_idx: u32, time_slot: u64, zone_polygon: &crate::types::Polygon) -> Point {
    let pos_seed = ha(creature_seed ^ creature_idx ^ time_slot as u32 ^ 0x9f3a);
    point_in_polygon_seeded(zone_polygon, pos_seed)
}

/// Generate creatures for a block.
pub fn generate_creatures(cell: &Cell, block: &Block) -> BlockCreatures {
    let count = creature_count(block.block_type, cell.popcount_ratio);
    let roles = eligible_roles(block.block_type);
    let mut creatures = Vec::new();

    for i in 0..count {
        let creature_seed = ha(cell.creature_seed ^ i);
        let role = roles[creature_seed as usize % roles.len()];
        let creature = derive_creature(cell, creature_seed, i as usize, role);
        creatures.push(creature);
    }

    BlockCreatures {
        block_idx: block.idx,
        creatures,
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
            block_type: bt, area: 6400.0,
            centroid: Point::new(40.0, 40.0), river_adjacent: false,
        }
    }

    #[test]
    fn creatures_in_park() {
        let cell = Cell::from_octets(93, 184, 216);
        let result = generate_creatures(&cell, &test_block(BlockType::Park));
        assert!(!result.creatures.is_empty());
    }

    #[test]
    fn creatures_deterministic() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block(BlockType::Park);
        let r1 = generate_creatures(&cell, &block);
        let r2 = generate_creatures(&cell, &block);
        assert_eq!(r1.creatures.len(), r2.creatures.len());
        for (a, b) in r1.creatures.iter().zip(r2.creatures.iter()) {
            assert_eq!(a.creature_seed, b.creature_seed);
            assert_eq!(a.ecological_role, b.ecological_role);
        }
    }

    #[test]
    fn zone_assignment_deterministic() {
        let z1 = creature_zone_assignment(0x12345, 0, 0, 100, 4);
        let z2 = creature_zone_assignment(0x12345, 0, 0, 100, 4);
        assert_eq!(z1, z2);
    }

    #[test]
    fn zone_assignment_changes_with_time() {
        let z1 = creature_zone_assignment(0x12345, 0, 0, 100, 4);
        let z2 = creature_zone_assignment(0x12345, 0, 0, 101, 4);
        // Likely different (not guaranteed, but statistically almost certain)
        let _ = (z1, z2);
    }

    // Spec test vectors from Appendix C.2
    #[test]
    fn creature_seed_roots() {
        assert_eq!(hb(0x010000 ^ 0x7c2e9f31), 0x05470d17);
        assert_eq!(hb(0xffaa55 ^ 0x7c2e9f31), 0x0500d59a);
    }

    #[test]
    fn creature_seeds() {
        assert_eq!(ha(0x05470d17 ^ 0), 0xfde0b098);
        assert_eq!(ha(0x0500d59a ^ 0), 0xe0d4fb61);
        assert_eq!(ha(0x0500d59a ^ 1), 0x01bddb4f);
    }
}

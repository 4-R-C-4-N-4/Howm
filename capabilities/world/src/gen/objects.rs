//! Universal object model — shared types and seed derivation for all
//! world objects (fixtures, flora, creatures, conveyances, buildings).

use serde::{Deserialize, Serialize};

use super::hash::ha;
use crate::types::Point;

/// Object persistence tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// Fully reconstructed from seed. No storage.
    Seedable = 0,
    /// Function of seed + coarse world time. No messages exchanged.
    TimeSynced = 1,
}

/// Form class for fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormClass {
    Column,
    Platform,
    Enclosure,
    Surface,
    Container,
    Span,
    Compound,
    Growth,
}

/// Attachment mode for fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attachment {
    Floor,
    Wall,
    Ceiling,
    Hanging,
    Embedded,
    Freestanding,
    SurfaceGrowth,
}

/// Hazard type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hazard {
    None,
    Damage,
    Impede,
    Repel,
}

/// Seeds derived from an object_seed per §11.5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSeeds {
    pub object_seed: u32,
    pub form_seed: u32,
    pub material_seed: u32,
    pub state_seed: u32,
    pub character_salt: u32,
    pub name_seed: u32,
    pub behaviour_seed: u32,
    pub interaction_seed: u32,
    pub eco_seed: u32,
    pub instance_hash: u32,
}

impl ObjectSeeds {
    /// Derive all sub-seeds from a single object_seed.
    pub fn from_seed(object_seed: u32) -> Self {
        Self {
            object_seed,
            form_seed: ha(object_seed ^ 0x1),
            material_seed: ha(object_seed ^ 0x2),
            state_seed: ha(object_seed ^ 0x3),
            character_salt: ha(object_seed ^ 0x4),
            name_seed: ha(object_seed ^ 0x5),
            behaviour_seed: ha(object_seed ^ 0x6),
            interaction_seed: ha(object_seed ^ 0x7),
            eco_seed: ha(object_seed ^ 0x8),
            instance_hash: ha(object_seed ^ 0x9),
        }
    }
}

/// The render packet — the resolved output passed to the renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPacket {
    /// Globally unique, stable: ha(cell_key ^ object_seed).
    pub object_id: u64,
    /// e.g. "fixture:illumination", "creature:aerial".
    pub archetype: String,
    /// Persistence tier.
    pub tier: Tier,

    /// Position in world space.
    pub position: Point,
    /// Height (Z coordinate).
    pub height: f64,
    /// Orientation in radians.
    pub orientation: f64,
    /// Scale multiplier.
    pub scale: f64,

    /// Renderer maps to geometry/animation/sound.
    pub form_id: u32,
    /// Renderer extracts axes per its schema.
    pub material_seed: u32,

    /// Whether the object is currently active.
    pub active: bool,
    pub state_seed: u32,

    /// Radius where player can interact.
    pub interaction_zone: f64,

    /// Per-type extensions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

/// Compute a form_id from archetype, aesthetic bucket, and object seed.
pub fn compute_form_id(archetype: &str, aesthetic_bucket: u32, object_seed: u32) -> u32 {
    // Hash the archetype string to a u32
    let mut archetype_hash: u32 = 0;
    for b in archetype.bytes() {
        archetype_hash = archetype_hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    let archetype_hash = ha(archetype_hash);
    ha(archetype_hash ^ aesthetic_bucket ^ object_seed)
}

/// Compute a globally unique object_id.
pub fn compute_object_id(cell_key: u32, object_seed: u32) -> u64 {
    ha(cell_key ^ object_seed) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_derivation() {
        let seeds = ObjectSeeds::from_seed(0xd0c2145e);
        // form_seed = ha(object_seed ^ 0x1)
        assert_eq!(seeds.form_seed, ha(0xd0c2145e ^ 0x1));
        assert_eq!(seeds.material_seed, ha(0xd0c2145e ^ 0x2));
        assert_eq!(seeds.state_seed, ha(0xd0c2145e ^ 0x3));
    }

    #[test]
    fn form_id_deterministic() {
        let f1 = compute_form_id("fixture:illumination", 42, 0x1234);
        let f2 = compute_form_id("fixture:illumination", 42, 0x1234);
        assert_eq!(f1, f2);
    }

    #[test]
    fn form_id_varies() {
        let f1 = compute_form_id("fixture:illumination", 42, 0x1234);
        let f2 = compute_form_id("fixture:seating", 42, 0x1234);
        assert_ne!(f1, f2);
    }
}

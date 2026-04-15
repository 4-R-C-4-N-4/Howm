//! Core HDL types — DescriptionGraph, Trait, Sequence, DescriptionPacket.
//!
//! These types are the shared contract between generators and renderers.
//! Paths are 3-segment dot-separated: root.branch.leaf
//! Four roots: being, behavior, effect, relation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single trait in a description graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trait {
    /// Hierarchical address: "being.surface.texture", "behavior.motion.method", etc.
    pub path: String,
    /// Semantic label from open vocabulary: "faceted", "continuous", etc.
    pub term: String,
    /// Continuous param values. Keys are axis names, values typically 0–1.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f64>,
}

impl Trait {
    pub fn new(path: &str, term: &str) -> Self {
        Self {
            path: path.to_string(),
            term: term.to_string(),
            params: BTreeMap::new(),
        }
    }

    pub fn with_param(mut self, key: &str, value: f64) -> Self {
        self.params.insert(key.to_string(), value);
        self
    }

    pub fn with_params(mut self, params: &[(&str, f64)]) -> Self {
        for (k, v) in params {
            self.params.insert(k.to_string(), *v);
        }
        self
    }
}

/// Sequence trigger — which trait and what event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceTrigger {
    pub path: String,
    pub event: String,
}

/// Sequence effect — which trait and what action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceEffect {
    pub path: String,
    pub action: String,
    /// Additional key-value data (intensity, factor, etc.)
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Sequence timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceTiming {
    pub delay: f64,
    /// Duration in seconds. None = until reset.
    pub duration: Option<f64>,
}

/// A causal relationship between traits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sequence {
    pub trigger: SequenceTrigger,
    pub effect: SequenceEffect,
    pub timing: SequenceTiming,
}

impl Sequence {
    pub fn new(
        trigger_path: &str,
        trigger_event: &str,
        effect_path: &str,
        effect_action: &str,
        delay: f64,
        duration: Option<f64>,
    ) -> Self {
        Self {
            trigger: SequenceTrigger {
                path: trigger_path.to_string(),
                event: trigger_event.to_string(),
            },
            effect: SequenceEffect {
                path: effect_path.to_string(),
                action: effect_action.to_string(),
                extra: BTreeMap::new(),
            },
            timing: SequenceTiming { delay, duration },
        }
    }

    pub fn with_effect_param(mut self, key: &str, value: serde_json::Value) -> Self {
        self.effect.extra.insert(key.to_string(), value);
        self
    }
}

/// A complete description graph for one entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptionGraph {
    pub traits: Vec<Trait>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequences: Vec<Sequence>,
}

impl DescriptionGraph {
    pub fn new() -> Self {
        Self {
            traits: Vec::new(),
            sequences: Vec::new(),
        }
    }

    pub fn push_trait(&mut self, t: Trait) {
        self.traits.push(t);
    }

    pub fn push_sequence(&mut self, s: Sequence) {
        self.sequences.push(s);
    }
}

impl Default for DescriptionGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// A description packet — the full output for one renderable entity.
/// Carries the description graph alongside seeds and district context
/// needed for renderer-side time-sync computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptionPacket {
    pub object_id: u64,
    pub archetype: String,
    pub graph: DescriptionGraph,
    /// District hue (0–360) for colour pipeline.
    pub district_hue: f64,
    /// Seeds the renderer needs for time-synced derivation.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub seeds: BTreeMap<String, u64>,
}

/// Surface growth overlay — modifies a host entity's description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceGrowthOverlay {
    /// 0–1 coverage ratio from inverted_age.
    pub coverage: f64,
    /// Traits overlaid on the host entity.
    pub traits: Vec<Trait>,
}

/// Building geometry extension — explicit geometry bypassing SDF resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingExtension {
    pub footprint: Vec<[f64; 2]>,
    pub height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<BuildingEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interior: Option<BuildingInterior>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingEntry {
    pub position: [f64; 3],
    pub orientation: [f64; 3],
    pub width: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingInterior {
    pub polygon: Vec<[f64; 2]>,
    pub height: f64,
    pub block_type: String,
}

/// HDL version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HDLVersion {
    pub version: u32,
    pub supported_roots: Vec<String>,
}

impl HDLVersion {
    pub fn v1() -> Self {
        Self {
            version: 1,
            supported_roots: vec![
                "being".into(),
                "behavior".into(),
                "effect".into(),
                "relation".into(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trait_builder() {
        let t = Trait::new("being.form.silhouette", "tall")
            .with_param("aspect", 0.42);
        assert_eq!(t.path, "being.form.silhouette");
        assert_eq!(t.term, "tall");
        assert_eq!(t.params["aspect"], 0.42);
    }

    #[test]
    fn sequence_builder() {
        let s = Sequence::new(
            "behavior.motion", "arrival",
            "effect.emission", "burst",
            0.0, Some(0.3),
        ).with_effect_param("intensity", serde_json::json!(0.8));

        assert_eq!(s.trigger.path, "behavior.motion");
        assert_eq!(s.effect.action, "burst");
        assert_eq!(s.timing.duration, Some(0.3));
    }

    #[test]
    fn description_graph_roundtrip() {
        let mut g = DescriptionGraph::new();
        g.push_trait(Trait::new("being.form.silhouette", "wide").with_param("aspect", 0.42));
        g.push_trait(Trait::new("being.surface.texture", "faceted").with_params(&[
            ("complexity", 0.7),
            ("reflectance", 0.6),
        ]));
        g.push_sequence(Sequence::new(
            "behavior.motion", "arrival",
            "effect.emission", "burst",
            0.0, Some(0.3),
        ));

        let json = serde_json::to_string(&g).unwrap();
        let parsed: DescriptionGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.traits.len(), 2);
        assert_eq!(parsed.sequences.len(), 1);
    }

    #[test]
    fn description_packet_serialize() {
        let packet = DescriptionPacket {
            object_id: 12345,
            archetype: "creature:aerial".into(),
            graph: DescriptionGraph::new(),
            district_hue: 54.1,
            seeds: BTreeMap::new(),
        };
        let json = serde_json::to_string(&packet).unwrap();
        assert!(json.contains("creature:aerial"));
    }

    #[test]
    fn hdl_version() {
        let v = HDLVersion::v1();
        assert_eq!(v.version, 1);
        assert_eq!(v.supported_roots.len(), 4);
    }

    #[test]
    fn three_segment_paths() {
        // All paths must be root.branch.leaf (3 segments)
        let paths = [
            "being.form.silhouette",
            "being.form.composition",
            "being.form.symmetry",
            "being.form.scale",
            "being.form.detail",
            "being.surface.texture",
            "being.surface.opacity",
            "being.surface.age",
            "being.material.substance",
            "being.material.density",
            "being.material.temperature",
            "behavior.motion.method",
            "behavior.motion.pace",
            "behavior.motion.regularity",
            "behavior.motion.path",
            "behavior.rest.frequency",
            "behavior.rest.posture",
            "behavior.rest.transition",
            "behavior.cycle.period",
            "behavior.cycle.response",
            "effect.emission.type",
            "effect.emission.intensity",
            "effect.emission.rhythm",
            "effect.emission.channel",
            "effect.trail.type",
            "effect.trail.duration",
            "effect.voice.type",
            "effect.voice.intensity",
            "effect.voice.spatial",
            "relation.regard.disposition",
            "relation.regard.response",
            "relation.regard.awareness",
            "relation.affinity.fixture",
            "relation.affinity.flora",
            "relation.affinity.creature",
            "relation.context.belonging",
            "relation.context.narrative",
        ];
        for p in &paths {
            let segs: Vec<_> = p.split('.').collect();
            assert_eq!(segs.len(), 3, "Path '{}' must have 3 segments", p);
            assert!(
                ["being", "behavior", "effect", "relation"].contains(&segs[0]),
                "Path '{}' has invalid root '{}'", p, segs[0]
            );
        }
    }
}

//! Description graph mapping — translates base records into HDL description graphs.
//!
//! Implements the complete mapping from howm-description-graph-mapping.md:
//! - §3: Creature mapping (being, behavior, effect, relation + sequences)
//! - §4: Fixture mapping (being, effect, behavior)
//! - §5: Flora mapping (being, behavior, effect + sequences)
//! - §6: Building mapping (being, effect + geometry extension)
//! - §7: District environment mapping (sky, ambient, lights, weather)
//! - §9: Conveyance mapping (parked + moving)

use crate::gen::aesthetic::AestheticPalette;
use crate::gen::atmosphere::AtmosphereState;
use crate::gen::buildings::{Archetype, BuildingPlot};
use crate::gen::cell::{Cell, Domain};
use crate::gen::conveyances::{Conveyance, ConveyanceType};
use crate::gen::creatures::*;
use crate::gen::fixtures::{Fixture, FixtureRole};
use crate::gen::flora::{DensityMode, Flora, GrowthForm};
use crate::gen::hash::{ha, hash_to_f64};
use crate::gen::objects::FormClass;
use crate::hdl::traits::*;


const MAX: f64 = 0xFFFF_FFFF_u32 as f64;

// ─── Helper ───────────────────────────────────────────────────────────────

fn seed_param(seed: u32, salt: u32) -> f64 {
    ha(seed ^ salt) as f64 / MAX
}

fn seed_range(seed: u32, salt: u32, min: f64, max: f64) -> f64 {
    min + seed_param(seed, salt) * (max - min)
}

// ═══════════════════════════════════════════════════════════════════════════
// §3  CREATURE MAPPING
// ═══════════════════════════════════════════════════════════════════════════

pub fn map_creature(c: &Creature, palette: &AestheticPalette) -> DescriptionGraph {
    let mut g = DescriptionGraph::new();
    let cs = c.creature_seed;
    let pr = palette.popcount_ratio;

    // ── 3.1 being.form ──

    // Silhouette (§3.1.1)
    let silhouette_term = derive_creature_silhouette(c);
    g.push_trait(
        Trait::new("being.form.silhouette", silhouette_term)
            .with_param("aspect", seed_param(cs ^ 0x1, 0xf01)),
    );

    // Composition (§3.1.3)
    let (comp_term, comp_params) = derive_creature_composition(c);
    let mut t = Trait::new("being.form.composition", comp_term);
    for (k, v) in comp_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    // Symmetry
    let sym_term = match c.anatomy {
        Anatomy::Bilateral => "bilateral",
        Anatomy::Radial => "radial",
        Anatomy::Amorphous => "asymmetric",
        Anatomy::Composite => "approximate",
    };
    g.push_trait(
        Trait::new("being.form.symmetry", sym_term)
            .with_param("fidelity", seed_param(cs, 0xf03)),
    );

    // Scale
    let scale_term = match c.size_class {
        SizeClass::Tiny => "diminutive",
        SizeClass::Small => "small",
        SizeClass::Medium => "moderate",
        SizeClass::Large => "imposing",
    };
    g.push_trait(
        Trait::new("being.form.scale", scale_term)
            .with_param("factor", match c.size_class {
                SizeClass::Tiny => 0.3,
                SizeClass::Small => 0.6,
                SizeClass::Medium => 1.0,
                SizeClass::Large => 1.8,
            }),
    );

    // Detail (§3.1.2)
    let (detail_term, detail_params) = derive_creature_detail(c);
    let mut t = Trait::new("being.form.detail", detail_term);
    for (k, v) in detail_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    // ── 3.2 being.surface ──

    let (tex_term, tex_params) = creature_surface_texture(c, pr);
    let mut t = Trait::new("being.surface.texture", tex_term);
    for (k, v) in tex_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    let (opa_term, opa_params) = creature_surface_opacity(c);
    let mut t = Trait::new("being.surface.opacity", opa_term);
    for (k, v) in opa_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    // Surface age (§3.2.1)
    let instance_norm = hash_to_f64(c.seeds.instance_hash);
    let effective_age = (palette.age + (instance_norm - 0.5) * 0.2).clamp(0.0, 1.0);
    let age_term = if effective_age < 0.15 {
        "decaying"
    } else if effective_age < 0.35 {
        "ancient"
    } else if effective_age < 0.65 {
        "weathered"
    } else if effective_age < 0.85 {
        "fresh"
    } else {
        "nascent"
    };
    g.push_trait(
        Trait::new("being.surface.age", age_term)
            .with_param("wear", 1.0 - effective_age),
    );

    // ── 3.3 being.material ──

    let substance_term = match c.materiality {
        Materiality::Flesh => "organic",
        Materiality::Construct => "constructed",
        Materiality::Spirit | Materiality::Spectral => "spectral",
        Materiality::Elemental => "elemental",
        Materiality::Crystalline => "mineral",
        Materiality::Vegetal => "organic",
    };
    g.push_trait(
        Trait::new("being.material.substance", substance_term)
            .with_param("hardness", match c.materiality {
                Materiality::Crystalline => 0.9,
                Materiality::Construct | Materiality::Elemental => 0.7,
                Materiality::Flesh => 0.3,
                Materiality::Vegetal => 0.2,
                _ => 0.1,
            })
            .with_param("hue_seed", seed_param(cs, 0xa5)),
    );

    // Density (§3.3)
    let density_term = derive_creature_density(c);
    g.push_trait(
        Trait::new("being.material.density", density_term)
            .with_param("value", seed_param(cs ^ 0xa5, 0xd01)),
    );

    // Temperature (§3.3)
    let temp_term = derive_creature_temperature(c, palette.domain);
    g.push_trait(
        Trait::new("being.material.temperature", temp_term)
            .with_param("intensity", seed_range(cs ^ 0xa5, 0xd02, 0.3, 0.8)),
    );

    // ── 3.4 behavior.motion ──

    let (method_term, method_params) = creature_motion_method(c);
    let mut t = Trait::new("behavior.motion.method", method_term);
    for (k, v) in method_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    g.push_trait(Trait::new("behavior.motion.pace", match c.pace {
        Pace::Slow => "slow",
        Pace::Medium => "moderate",
        Pace::Fast => "fast",
    }));

    let reg_term = match c.smoothness {
        Smoothness::Fluid => "rhythmic",
        Smoothness::Jerky => "irregular",
        Smoothness::Erratic => "chaotic",
        Smoothness::Mechanical => "metronomic",
    };
    g.push_trait(Trait::new("behavior.motion.regularity", reg_term));

    let path_term = match c.path_preference {
        PathPreference::Open => "wandering",
        PathPreference::Edges => "edge-following",
        PathPreference::Elevated => "vertical",
        PathPreference::Surface => "linear",
        PathPreference::Low => "wandering",
    };
    g.push_trait(Trait::new("behavior.motion.path", path_term));

    // ── 3.5 behavior.rest ──

    let rest_term = if c.rest_frequency > 0.8 {
        "dominant"
    } else if c.rest_frequency > 0.5 {
        "frequent"
    } else if c.rest_frequency > 0.2 {
        "occasional"
    } else {
        "rare"
    };
    g.push_trait(
        Trait::new("behavior.rest.frequency", rest_term)
            .with_param("value", c.rest_frequency),
    );

    let posture_term = match c.locomotion_mode {
        LocomotionMode::Aerial | LocomotionMode::Floating => "hovering",
        LocomotionMode::Aquatic => "drifting",
        LocomotionMode::Burrowing => "dormant",
        LocomotionMode::Phasing => "rigid",
        LocomotionMode::Surface => {
            if matches!(c.size_class, SizeClass::Tiny | SizeClass::Small) {
                "settled"
            } else {
                "rigid"
            }
        }
    };
    g.push_trait(Trait::new("behavior.rest.posture", posture_term));

    let transition_term = match c.smoothness {
        Smoothness::Fluid | Smoothness::Mechanical => "gradual",
        Smoothness::Jerky | Smoothness::Erratic => "instant",
    };
    g.push_trait(Trait::new("behavior.rest.transition", transition_term));

    // ── 3.6 behavior.cycle ──

    let (period_term, response_term) = match c.activity_pattern {
        ActivityPattern::Diurnal => ("diurnal", "withdraw"),
        ActivityPattern::Nocturnal => ("nocturnal", "emerge"),
        ActivityPattern::Crepuscular => ("crepuscular", "intensify"),
        ActivityPattern::Continuous => ("continuous", "none"),
    };
    g.push_trait(Trait::new("behavior.cycle.period", period_term));
    if response_term != "none" {
        g.push_trait(Trait::new("behavior.cycle.response", response_term));
    }

    // ── 3.7 effect.emission ──

    let emission = derive_creature_emission(c, palette);
    if let Some((etype, intensity, rhythm, channel)) = emission {
        g.push_trait(
            Trait::new("effect.emission.type", etype)
                .with_param("seed", ha(cs ^ 0xa5 ^ 0xe01) as f64),
        );
        g.push_trait(
            Trait::new("effect.emission.intensity", intensity)
                .with_param("value", seed_range(cs ^ 0xa5, 0xe01, 0.1, 0.5)),
        );
        g.push_trait(Trait::new("effect.emission.rhythm", rhythm));
        g.push_trait(Trait::new("effect.emission.channel", channel));
    }

    // ── 3.8 effect.voice ──

    let (voice_type, voice_intensity, voice_spatial) = creature_voice(c);
    g.push_trait(
        Trait::new("effect.voice.type", voice_type)
            .with_param("pitch_seed", c.sound_seed as f64),
    );
    g.push_trait(Trait::new("effect.voice.intensity", voice_intensity));
    g.push_trait(Trait::new("effect.voice.spatial", voice_spatial));

    // ── 3.9 effect.trail ──

    if c.leaves_trail {
        let trail_type = if matches!(c.locomotion_style, LocomotionStyle::Blinking) {
            "echo"
        } else {
            "residue"
        };
        let trail_dur = if trail_type == "echo" { "brief" } else { "lingering" };
        g.push_trait(Trait::new("effect.trail.type", trail_type));
        g.push_trait(Trait::new("effect.trail.duration", trail_dur));
    }

    // ── 3.10 relation.regard ──

    let (disp, resp, aware) = match c.player_response {
        PlayerResponse::Flee => ("wary", "withdraw", "peripheral"),
        PlayerResponse::Ignore => ("indifferent", "none", "oblivious"),
        PlayerResponse::Curious => ("curious", "approach", "attentive"),
        PlayerResponse::Territorial => ("territorial", "freeze", "fixated"),
        PlayerResponse::Mimicking => ("mimicking", "mirror", "attentive"),
    };
    g.push_trait(Trait::new("relation.regard.awareness", aware));
    g.push_trait(
        Trait::new("relation.regard.disposition", disp)
            .with_param("radius", seed_range(cs ^ 0xb3, 0xf01, 4.0, 16.0))
            .with_param("threshold", seed_range(cs ^ 0xb3, 0xf02, 1.0, 5.0)),
    );
    g.push_trait(
        Trait::new("relation.regard.response", resp)
            .with_param("speed", match c.pace {
                Pace::Fast => 2.0,
                Pace::Slow => 0.5,
                _ => 1.0,
            }),
    );

    // ── 3.11 relation.affinity ──

    let fix_aff = match c.fixture_interaction {
        FixtureInteraction::Perch => "perch",
        FixtureInteraction::Hide => "hide",
        FixtureInteraction::Nest => "nest",
        FixtureInteraction::Ignore => "ignore",
    };
    g.push_trait(Trait::new("relation.affinity.fixture", fix_aff));

    let flora_aff = match c.path_preference {
        PathPreference::Low => "shelter",
        _ => "ignore",
    };
    g.push_trait(Trait::new("relation.affinity.flora", flora_aff));

    let creature_aff = match c.social_structure {
        SocialStructure::Solitary => "avoid",
        _ => "flock",
    };
    g.push_trait(Trait::new("relation.affinity.creature", creature_aff));

    // ── 3.12 relation.context ──

    g.push_trait(Trait::new("relation.context.belonging", "native"));
    g.push_trait(Trait::new("relation.context.narrative", derive_creature_narrative(c)));

    // ── 3.13 Sequences ──

    let has_emission = emission.is_some();
    let has_trail = c.leaves_trail;
    let has_voice = !matches!(c.sound_tendency, SoundTendency::Silent);

    let mut sequence_count =
        (pr * 4.0).floor() as usize;
    if matches!(
        c.materiality,
        Materiality::Spirit | Materiality::Spectral | Materiality::Crystalline | Materiality::Elemental
    ) {
        sequence_count += 1;
    }
    if matches!(c.locomotion_style, LocomotionStyle::Blinking) {
        sequence_count += 1;
    }

    let mut eligible: Vec<Sequence> = Vec::new();

    if has_emission {
        eligible.push(
            Sequence::new("behavior.motion", "arrival", "effect.emission", "burst", 0.0, Some(0.3))
                .with_effect_param("intensity", serde_json::json!(0.8)),
        );
    }
    if has_trail {
        eligible.push(Sequence::new(
            "behavior.motion", "departure", "effect.trail", "spawn", 0.0,
            Some(if matches!(c.locomotion_style, LocomotionStyle::Blinking) { 0.5 } else { 1.5 }),
        ));
    }
    if !matches!(c.player_response, PlayerResponse::Ignore) {
        eligible.push(
            Sequence::new("relation.regard", "activated", "behavior.motion", "accelerate", 0.2, None)
                .with_effect_param("factor", serde_json::json!(1.5)),
        );
    }
    if has_voice {
        eligible.push(
            Sequence::new("behavior.rest", "begin", "effect.voice", "swell", 1.0, Some(2.0))
                .with_effect_param("intensity", serde_json::json!(0.5)),
        );
    }
    if has_emission && emission.map(|e| e.3) == Some("background") {
        eligible.push(
            Sequence::new("behavior.rest", "begin", "effect.emission", "intensify", 0.5, None)
                .with_effect_param("factor", serde_json::json!(1.3)),
        );
    }
    if matches!(c.player_response, PlayerResponse::Territorial | PlayerResponse::Mimicking) {
        eligible.push(Sequence::new(
            "relation.regard", "activated", "being.surface", "flash", 0.0, Some(0.5),
        ));
    }

    let mut remaining = eligible;
    for i in 0..sequence_count.min(remaining.len()) {
        if remaining.is_empty() {
            break;
        }
        let pick = ha(c.seeds.behaviour_seed ^ i as u32 ^ 0x5e0) as usize % remaining.len();
        g.push_sequence(remaining.remove(pick));
    }

    g
}

// ── Creature helper functions ──

fn derive_creature_silhouette(c: &Creature) -> &'static str {
    match c.locomotion_mode {
        LocomotionMode::Aerial | LocomotionMode::Floating => {
            if matches!(c.size_class, SizeClass::Tiny | SizeClass::Small) {
                "compact"
            } else {
                "wide"
            }
        }
        LocomotionMode::Aquatic => "trailing",
        LocomotionMode::Burrowing => "compact",
        LocomotionMode::Phasing => "tall",
        LocomotionMode::Surface => match c.anatomy {
            Anatomy::Bilateral => {
                if matches!(c.size_class, SizeClass::Large) { "tall" } else { "compact" }
            }
            Anatomy::Amorphous => "irregular",
            Anatomy::Composite => "wide",
            Anatomy::Radial => "compact",
        },
    }
}

fn derive_creature_composition(c: &Creature) -> (&'static str, Vec<(&'static str, f64)>) {
    match c.anatomy {
        Anatomy::Composite => {
            let count = 2.0 + (ha(c.creature_seed ^ 0xa2 ^ 0xc01) & 0x1) as f64;
            let cohesion = seed_range(c.creature_seed ^ 0xa2, 0xc02, 0.2, 0.8);
            ("clustered", vec![("count", count), ("cohesion", cohesion)])
        }
        Anatomy::Amorphous => {
            let count = 2.0 + (ha(c.creature_seed ^ 0xa2 ^ 0xc01) & 0x3) as f64;
            let cohesion = seed_range(c.creature_seed ^ 0xa2, 0xc02, 0.1, 0.4);
            ("dispersed", vec![("count", count), ("cohesion", cohesion)])
        }
        Anatomy::Bilateral | Anatomy::Radial => ("singular", vec![]),
    }
}

fn derive_creature_detail(c: &Creature) -> (&'static str, Vec<(&'static str, f64)>) {
    let fs = c.seeds.form_seed as f64;
    match c.materiality {
        Materiality::Flesh => ("organic", vec![("frequency", 2.5), ("amplitude", 0.10), ("octaves", 3.0), ("seed", fs)]),
        Materiality::Construct => ("fractured", vec![("frequency", 3.0), ("amplitude", 0.12), ("octaves", 2.0), ("seed", fs)]),
        Materiality::Spirit | Materiality::Spectral => ("smooth", vec![("frequency", 0.0), ("amplitude", 0.0), ("octaves", 0.0), ("seed", 0.0)]),
        Materiality::Elemental => ("rough", vec![("frequency", 5.0), ("amplitude", 0.06), ("octaves", 2.0), ("seed", fs)]),
        Materiality::Crystalline => ("fractured", vec![("frequency", 4.0), ("amplitude", 0.15), ("octaves", 2.0), ("seed", fs)]),
        Materiality::Vegetal => ("organic", vec![("frequency", 2.0), ("amplitude", 0.08), ("octaves", 3.0), ("seed", fs)]),
    }
}

fn creature_surface_texture(c: &Creature, pr: f64) -> (&'static str, Vec<(&'static str, f64)>) {
    match c.materiality {
        Materiality::Flesh => ("rough", vec![("complexity", 0.5 + pr * 0.3), ("reflectance", 0.1)]),
        Materiality::Construct => ("faceted", vec![("complexity", 0.4 + pr * 0.4), ("reflectance", 0.3)]),
        Materiality::Spirit => ("smooth", vec![("complexity", 0.1), ("reflectance", 0.05)]),
        Materiality::Elemental => ("granular", vec![("complexity", 0.6 + pr * 0.2), ("reflectance", 0.2)]),
        Materiality::Crystalline => ("faceted", vec![("complexity", 0.7), ("reflectance", 0.6)]),
        Materiality::Spectral => ("fluid", vec![("complexity", 0.2), ("reflectance", 0.1)]),
        Materiality::Vegetal => ("fibrous", vec![("complexity", 0.5 + pr * 0.2), ("reflectance", 0.05)]),
    }
}

fn creature_surface_opacity(c: &Creature) -> (&'static str, Vec<(&'static str, f64)>) {
    match c.materiality {
        Materiality::Spirit => ("transparent", vec![("level", 0.2)]),
        Materiality::Crystalline => ("translucent", vec![("level", 0.4)]),
        Materiality::Spectral => ("shifting", vec![("level", 0.15)]),
        _ => ("solid", vec![]),
    }
}

fn derive_creature_density(c: &Creature) -> &'static str {
    match c.materiality {
        Materiality::Spirit | Materiality::Spectral => "gossamer",
        Materiality::Elemental | Materiality::Crystalline => {
            if matches!(c.size_class, SizeClass::Large) { "dense" } else { "moderate" }
        }
        Materiality::Construct => "dense",
        Materiality::Flesh => {
            if matches!(c.size_class, SizeClass::Tiny) { "light" } else { "moderate" }
        }
        Materiality::Vegetal => "light",
    }
}

fn derive_creature_temperature(c: &Creature, domain: Domain) -> &'static str {
    let base = match c.materiality {
        Materiality::Flesh => "warm",
        Materiality::Construct | Materiality::Vegetal => "neutral",
        Materiality::Spirit | Materiality::Crystalline => "cold",
        Materiality::Elemental => "hot",
        Materiality::Spectral => "cool",
    };
    // Domain shift
    match domain {
        Domain::Loopback => if base == "warm" || base == "neutral" { "cold" } else { base },
        Domain::Multicast => if base == "cold" || base == "cool" { "warm" } else { base },
        Domain::Reserved => if base == "warm" || base == "hot" { "cool" } else { base },
        _ => base,
    }
}

fn creature_motion_method(c: &Creature) -> (&'static str, Vec<(&'static str, f64)>) {
    let cs = c.creature_seed;
    match c.locomotion_style {
        LocomotionStyle::Scurrying => ("continuous", vec![("interval", seed_range(cs ^ 0xc1, 0xe01, 0.3, 0.7))]),
        LocomotionStyle::Bounding => ("continuous", vec![("interval", seed_range(cs ^ 0xc1, 0xe01, 0.5, 1.0))]),
        LocomotionStyle::Slithering => ("continuous", vec![("interval", seed_range(cs ^ 0xc1, 0xe01, 0.8, 1.4))]),
        LocomotionStyle::Flapping => ("oscillating", vec![("interval", seed_range(cs ^ 0xc1, 0xe01, 0.2, 0.5))]),
        LocomotionStyle::Soaring => ("drifting", vec![("interval", seed_range(cs ^ 0xc1, 0xe01, 2.0, 5.0))]),
        LocomotionStyle::Drifting => ("drifting", vec![("interval", seed_range(cs ^ 0xc1, 0xe01, 1.5, 4.0))]),
        LocomotionStyle::Blinking => ("discontinuous", vec![
            ("interval", seed_range(cs ^ 0xc1, 0xe01, 0.5, 2.0)),
            ("variance", 0.3),
        ]),
    }
}

fn derive_creature_emission(c: &Creature, _palette: &AestheticPalette) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    // Materiality-driven inherent emission
    match c.materiality {
        Materiality::Spirit => Some(("glow", "subtle", "constant", "both")),
        Materiality::Elemental => Some(("sparks", "moderate", "sporadic", "foreground")),
        Materiality::Crystalline => Some(("pulse", "faint", "periodic", "background")),
        Materiality::Spectral => Some(("glow", "faint", "periodic", "background")),
        _ if c.emits_particles => Some(("sparks", "subtle", "sporadic", "foreground")),
        _ => None,
    }
}

fn creature_voice(c: &Creature) -> (&'static str, &'static str, &'static str) {
    let (vtype, base_intensity, spatial) = match c.sound_tendency {
        SoundTendency::Silent => ("silent", "whisper", "local"),
        SoundTendency::Ambient => ("drone", "quiet", "ambient"),
        SoundTendency::Reactive => ("rhythmic", "moderate", "directional"),
        SoundTendency::Constant => ("drone", "moderate", "ambient"),
    };
    // Size modulates intensity
    let intensity = match c.size_class {
        SizeClass::Large => match base_intensity {
            "whisper" => "quiet",
            "quiet" => "moderate",
            "moderate" => "loud",
            other => other,
        },
        SizeClass::Tiny => match base_intensity {
            "loud" => "moderate",
            "moderate" => "quiet",
            "quiet" => "whisper",
            other => other,
        },
        _ => base_intensity,
    };
    (vtype, intensity, spatial)
}

fn derive_creature_narrative(c: &Creature) -> &'static str {
    if c.rest_frequency > 0.8 && matches!(c.fixture_interaction, FixtureInteraction::Perch) {
        return "guardian";
    }
    if matches!(c.locomotion_style, LocomotionStyle::Drifting)
        && matches!(c.social_structure, SocialStructure::Solitary)
    {
        return "wanderer";
    }
    if matches!(c.materiality, Materiality::Spirit | Materiality::Spectral) {
        return "remnant";
    }
    if matches!(c.sound_tendency, SoundTendency::Constant)
        && matches!(c.social_structure, SocialStructure::Solitary)
    {
        return "herald";
    }
    if matches!(c.activity_pattern, ActivityPattern::Crepuscular)
        && matches!(c.player_response, PlayerResponse::Curious)
    {
        return "cipher";
    }
    "wanderer"
}

// ═══════════════════════════════════════════════════════════════════════════
// §4  FIXTURE MAPPING
// ═══════════════════════════════════════════════════════════════════════════

pub fn map_fixture(f: &Fixture, palette: &AestheticPalette) -> DescriptionGraph {
    let mut g = DescriptionGraph::new();
    let pr = palette.popcount_ratio;
    let inv_age = palette.inverted_age;
    let _os = f.seeds.object_seed;

    // ── 4.1 being.form ──

    let (sil, comp, sym) = fixture_form(f.form_class);
    g.push_trait(Trait::new("being.form.silhouette", sil));
    g.push_trait(Trait::new("being.form.composition", comp));
    g.push_trait(Trait::new("being.form.symmetry", sym));

    // Scale from height
    let scale_term = if f.scale_height < 1.5 {
        "small"
    } else if f.scale_height < 3.0 {
        "moderate"
    } else if f.scale_height < 5.0 {
        "large"
    } else {
        "imposing"
    };
    g.push_trait(
        Trait::new("being.form.scale", scale_term)
            .with_param("factor", f.scale_height / 4.0),
    );

    // Detail from district
    let detail = if inv_age > 0.8 {
        Trait::new("being.form.detail", "fractured")
            .with_params(&[("frequency", 3.0), ("amplitude", 0.08), ("octaves", 2.0)])
    } else if inv_age > 0.6 {
        Trait::new("being.form.detail", "rough")
            .with_params(&[("frequency", 4.0), ("amplitude", 0.04), ("octaves", 2.0)])
    } else if pr > 0.7 {
        Trait::new("being.form.detail", "textured")
            .with_params(&[("frequency", 3.0), ("amplitude", 0.05), ("octaves", 1.0)])
    } else {
        Trait::new("being.form.detail", "smooth")
            .with_params(&[("frequency", 0.0), ("amplitude", 0.0), ("octaves", 0.0)])
    };
    g.push_trait(detail);

    // ── 4.2 being.surface + material ──

    let substance = match palette.domain {
        Domain::Loopback => "mineral",
        Domain::Reserved => "elemental",
        Domain::Documentation => "mineral",
        _ => "constructed",
    };
    g.push_trait(Trait::new("being.material.substance", substance));

    let texture = if pr < 0.3 { "smooth" } else if pr < 0.7 { "faceted" } else { "rough" };
    g.push_trait(Trait::new("being.surface.texture", texture));
    g.push_trait(Trait::new("being.surface.opacity", "solid"));

    let temp = match palette.domain {
        Domain::Public => "neutral",
        Domain::Private | Domain::Multicast => "warm",
        Domain::Loopback => "cold",
        _ => "cool",
    };
    g.push_trait(Trait::new("being.material.temperature", temp));

    let density = match f.form_class {
        FormClass::Column | FormClass::Platform => "dense",
        FormClass::Span | FormClass::Growth => "light",
        _ => "moderate",
    };
    g.push_trait(Trait::new("being.material.density", density));

    // Surface age
    let age_term = if inv_age > 0.8 { "ancient" } else if inv_age > 0.5 { "weathered" } else { "fresh" };
    g.push_trait(Trait::new("being.surface.age", age_term));

    // ── 4.3 effect.emission ──

    if f.role == FixtureRole::Illumination {
        g.push_trait(Trait::new("effect.emission.type", "glow"));
        g.push_trait(Trait::new("effect.emission.intensity", "moderate"));
        let rhythm = if f.active_state { "periodic" } else { "constant" };
        g.push_trait(Trait::new("effect.emission.rhythm", rhythm));
        g.push_trait(Trait::new("effect.emission.channel", "both"));
    } else if f.role == FixtureRole::DisplaySurface && f.emissive_light {
        g.push_trait(Trait::new("effect.emission.type", "glow"));
        g.push_trait(Trait::new("effect.emission.intensity", "faint"));
        g.push_trait(Trait::new("effect.emission.rhythm", "constant"));
        g.push_trait(Trait::new("effect.emission.channel", "foreground"));
    } else if f.role == FixtureRole::Ornament && f.emissive_light {
        g.push_trait(Trait::new("effect.emission.type", "pulse"));
        g.push_trait(Trait::new("effect.emission.intensity", "subtle"));
        g.push_trait(Trait::new("effect.emission.rhythm", "periodic"));
        g.push_trait(Trait::new("effect.emission.channel", "background"));
    }

    // ── 4.4 behavior (state-cycling fixtures) ──

    if f.active_state {
        g.push_trait(Trait::new("behavior.motion.method", "anchored"));
        g.push_trait(Trait::new("behavior.cycle.period", "continuous"));
        g.push_trait(Trait::new("behavior.cycle.response", "transform"));

        // State-cycling emissive sequence
        if f.emissive_light {
            g.push_sequence(
                Sequence::new("behavior.cycle", "activate", "effect.emission", "intensify", 0.0, None)
                    .with_effect_param("factor", serde_json::json!(2.0)),
            );
        }
    }

    g
}

fn fixture_form(fc: FormClass) -> (&'static str, &'static str, &'static str) {
    match fc {
        FormClass::Column => ("tall", "singular", "radial"),
        FormClass::Platform => ("wide", "singular", "bilateral"),
        FormClass::Enclosure => ("compact", "nested", "bilateral"),
        FormClass::Surface => ("wide", "singular", "bilateral"),
        FormClass::Container => ("compact", "singular", "radial"),
        FormClass::Span => ("wide", "layered", "bilateral"),
        FormClass::Compound => ("irregular", "clustered", "asymmetric"),
        FormClass::Growth => ("irregular", "dispersed", "asymmetric"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// §5  FLORA MAPPING
// ═══════════════════════════════════════════════════════════════════════════

pub fn map_flora(f: &Flora, palette: &AestheticPalette) -> DescriptionGraph {
    let mut g = DescriptionGraph::new();
    let os = f.seeds.object_seed;
    let _ = palette;

    // ── 5.1 being.form ──

    let (sil, sym) = flora_silhouette_symmetry(f);
    g.push_trait(Trait::new("being.form.silhouette", sil));
    g.push_trait(Trait::new("being.form.symmetry", sym));

    // Composition from density_mode
    let (comp_term, comp_params) = match f.density_mode {
        DensityMode::Sparse => ("singular", vec![]),
        DensityMode::Moderate => ("clustered", vec![("count", 3.0), ("cohesion", 0.6)]),
        DensityMode::Dense => ("dispersed", vec![("count", 6.0), ("cohesion", 0.4)]),
        DensityMode::Canopy => ("layered", vec![("count", 4.0), ("cohesion", 0.8)]),
    };
    let mut t = Trait::new("being.form.composition", comp_term);
    for (k, v) in comp_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    // Scale
    let factor = f.scale / 4.0;
    let scale_term = if factor < 0.3 {
        "diminutive"
    } else if factor < 0.7 {
        "small"
    } else if factor < 1.3 {
        "moderate"
    } else {
        "large"
    };
    g.push_trait(
        Trait::new("being.form.scale", scale_term)
            .with_param("factor", factor),
    );

    // Detail from growth stage (maturity)
    let (detail_term, detail_params) = flora_growth_detail(f);
    let mut t = Trait::new("being.form.detail", detail_term);
    for (k, v) in detail_params {
        t = t.with_param(k, v);
    }
    g.push_trait(t);

    // ── 5.2 being.surface + material ──

    g.push_trait(Trait::new("being.material.substance", "organic"));

    let density = if f.maturity < 0.15 {
        "gossamer"
    } else if f.maturity > 0.85 {
        "light"
    } else {
        "moderate"
    };
    g.push_trait(Trait::new("being.material.density", density));
    g.push_trait(Trait::new("being.material.temperature", "neutral"));

    let texture = flora_texture(f);
    g.push_trait(Trait::new("being.surface.texture", texture));

    let (opa_term, opa_level) = if f.maturity < 0.15 {
        ("translucent", 0.3)
    } else if f.maturity > 0.85 {
        ("translucent", 0.5)
    } else {
        ("solid", 1.0)
    };
    g.push_trait(
        Trait::new("being.surface.opacity", opa_term)
            .with_param("level", opa_level),
    );

    // Age from growth stage
    let age_term = if f.maturity < 0.15 {
        "nascent"
    } else if f.maturity < 0.35 {
        "fresh"
    } else if f.maturity < 0.65 {
        "weathered"
    } else if f.maturity < 0.85 {
        "ancient"
    } else {
        "decaying"
    };
    g.push_trait(Trait::new("being.surface.age", age_term));

    // ── 5.3 behavior.motion (wind response) ──

    let wind_response = seed_param(os, 0x0a01);
    g.push_trait(
        Trait::new("behavior.motion.method", "oscillating")
            .with_param("interval", 2.0 + (1.0 - wind_response) * 4.0)
            .with_param("amplitude", wind_response * 0.3),
    );
    g.push_trait(Trait::new("behavior.motion.pace", "glacial"));
    g.push_trait(Trait::new("behavior.motion.regularity", "rhythmic"));

    // ── 5.4 effect.emission (shedding) ──

    if f.shedding {
        let shed_type = match f.growth_form {
            GrowthForm::Tree | GrowthForm::Shrub => "shed",
            GrowthForm::Fungal => "vapor",
            GrowthForm::Crystalline => "sparks",
            _ => "shed",
        };
        let shed_rate = seed_param(os, 0xc5);
        let intensity = if shed_rate < 0.3 { "faint" } else if shed_rate < 0.7 { "subtle" } else { "moderate" };

        g.push_trait(
            Trait::new("effect.emission.type", shed_type)
                .with_param("rate", shed_rate)
                .with_param("seed", ha(os ^ 0xc5) as f64),
        );
        g.push_trait(Trait::new("effect.emission.intensity", intensity));
        g.push_trait(Trait::new("effect.emission.rhythm", "sporadic"));
        g.push_trait(Trait::new("effect.emission.channel", "foreground"));

        // §5.5 Wind-shed sequence
        g.push_sequence(
            Sequence::new("behavior.motion", "peak", "effect.emission", "burst", 0.0, Some(0.5))
                .with_effect_param("intensity", serde_json::json!(0.4)),
        );
    }

    g
}

fn flora_silhouette_symmetry(f: &Flora) -> (&'static str, &'static str) {
    match f.growth_form {
        GrowthForm::Tree => ("tall", "radial"),
        GrowthForm::Shrub => ("compact", "approximate"),
        GrowthForm::GroundCover => ("wide", "asymmetric"),
        GrowthForm::Vine => ("trailing", "asymmetric"),
        GrowthForm::Fungal => ("compact", "radial"),
        GrowthForm::Aquatic => ("wide", "radial"),
        GrowthForm::Crystalline => ("tall", "radial"),
    }
}

fn flora_growth_detail(f: &Flora) -> (&'static str, Vec<(&'static str, f64)>) {
    let fs = f.seeds.form_seed as f64;
    if f.maturity < 0.15 {
        ("smooth", vec![("frequency", 0.0), ("amplitude", 0.0), ("octaves", 0.0), ("seed", 0.0)])
    } else if f.maturity < 0.35 {
        ("textured", vec![("frequency", 2.0), ("amplitude", 0.04), ("octaves", 1.0), ("seed", fs)])
    } else if f.maturity < 0.65 {
        ("organic", vec![("frequency", 2.5), ("amplitude", 0.10), ("octaves", 3.0), ("seed", fs)])
    } else if f.maturity < 0.85 {
        ("rough", vec![("frequency", 4.0), ("amplitude", 0.08), ("octaves", 2.0), ("seed", fs)])
    } else {
        ("fractured", vec![("frequency", 3.0), ("amplitude", 0.15), ("octaves", 2.0), ("seed", fs)])
    }
}

fn flora_texture(f: &Flora) -> &'static str {
    match f.growth_form {
        GrowthForm::GroundCover | GrowthForm::Vine => "fibrous",
        GrowthForm::Tree | GrowthForm::Shrub => "rough",
        GrowthForm::Aquatic => "smooth",
        GrowthForm::Fungal => "granular",
        GrowthForm::Crystalline => "faceted",
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// §6  BUILDING MAPPING
// ═══════════════════════════════════════════════════════════════════════════

pub fn map_building(plot: &BuildingPlot, palette: &AestheticPalette) -> DescriptionGraph {
    let mut g = DescriptionGraph::new();
    let pr = palette.popcount_ratio;
    let inv_age = palette.inverted_age;

    // ── 6.1 being.form ──

    let (sil, comp, sym) = building_form(plot.archetype);
    g.push_trait(Trait::new("being.form.silhouette", sil));
    g.push_trait(Trait::new("being.form.composition", comp));
    g.push_trait(Trait::new("being.form.symmetry", sym));

    // Scale from height
    let scale_term = if plot.height < 3.0 {
        "small"
    } else if plot.height < 6.0 {
        "moderate"
    } else if plot.height < 10.0 {
        "large"
    } else {
        "imposing"
    };
    g.push_trait(
        Trait::new("being.form.scale", scale_term)
            .with_param("factor", plot.height / 15.0), // normalise to MAX_HEIGHT
    );

    // Detail
    let detail = if inv_age > 0.7 {
        "fractured"
    } else if pr > 0.7 {
        "rough"
    } else if pr < 0.3 {
        "smooth"
    } else {
        "textured"
    };
    g.push_trait(Trait::new("being.form.detail", detail));

    // ── 6.2 being.surface + material ──

    let substance = match palette.domain {
        Domain::Loopback => "mineral",
        Domain::Reserved => "elemental",
        Domain::Documentation => "mineral",
        _ => "constructed",
    };
    g.push_trait(Trait::new("being.material.substance", substance));

    let texture = match plot.archetype {
        Archetype::Monolith | Archetype::Block => "smooth",
        Archetype::Growth | Archetype::Ruin => "rough",
        Archetype::Spire | Archetype::Dome | Archetype::Arch => "faceted",
        _ => if pr < 0.3 { "smooth" } else if pr < 0.7 { "faceted" } else { "rough" },
    };
    g.push_trait(Trait::new("being.surface.texture", texture));

    let opacity = match (palette.domain, plot.archetype) {
        (Domain::Loopback, _) => ("translucent", 0.3),
        (_, Archetype::Ruin) => ("translucent", 0.6),
        _ => ("solid", 1.0),
    };
    g.push_trait(
        Trait::new("being.surface.opacity", opacity.0)
            .with_param("level", opacity.1),
    );

    let age_term = if inv_age > 0.8 { "ancient" } else if inv_age > 0.5 { "weathered" } else { "fresh" };
    g.push_trait(Trait::new("being.surface.age", age_term));

    let temp = match palette.domain {
        Domain::Public => "neutral",
        Domain::Private | Domain::Multicast => "warm",
        Domain::Loopback => "cold",
        _ => "cool",
    };
    g.push_trait(Trait::new("being.material.temperature", temp));
    g.push_trait(Trait::new("being.material.density", "dense"));

    // ── 6.3 effect.emission ──

    if plot.is_public {
        g.push_trait(Trait::new("effect.emission.type", "glow"));
        let em_int = if plot.height > 6.0 { "moderate" } else { "faint" };
        g.push_trait(Trait::new("effect.emission.intensity", em_int));
        g.push_trait(Trait::new("effect.emission.rhythm", "constant"));
        g.push_trait(Trait::new("effect.emission.channel", "background"));
    }

    g
}

fn building_form(arch: Archetype) -> (&'static str, &'static str, &'static str) {
    match arch {
        Archetype::Tower => ("tall", "singular", "bilateral"),
        Archetype::Spire => ("tall", "layered", "radial"),
        Archetype::Stack => ("tall", "layered", "approximate"),
        Archetype::Block | Archetype::Hall => ("wide", "singular", "bilateral"),
        Archetype::Compound => ("irregular", "clustered", "asymmetric"),
        Archetype::Dome => ("compact", "singular", "radial"),
        Archetype::Arch => ("wide", "nested", "bilateral"),
        Archetype::Monolith => ("tall", "singular", "bilateral"),
        Archetype::Growth | Archetype::Ruin => ("irregular", "dispersed", "asymmetric"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// §9  CONVEYANCE MAPPING
// ═══════════════════════════════════════════════════════════════════════════

pub fn map_conveyance(c: &Conveyance, palette: &AestheticPalette) -> DescriptionGraph {
    let mut g = DescriptionGraph::new();
    let pr = palette.popcount_ratio;

    // being.form — all conveyances
    g.push_trait(Trait::new("being.form.silhouette", "wide"));
    g.push_trait(Trait::new("being.form.composition", "singular"));
    g.push_trait(Trait::new("being.form.scale", "moderate"));
    g.push_trait(Trait::new("being.form.symmetry", "bilateral"));

    // Detail from district
    let detail = if pr > 0.7 { "textured" } else if pr < 0.3 { "smooth" } else { "faceted" };
    g.push_trait(Trait::new("being.form.detail", detail));

    // Surface — same as fixtures
    let substance = match palette.domain {
        Domain::Loopback => "mineral",
        Domain::Reserved => "elemental",
        _ => "constructed",
    };
    g.push_trait(Trait::new("being.material.substance", substance));
    g.push_trait(Trait::new("being.material.density", "dense"));
    g.push_trait(Trait::new("being.surface.opacity", "solid"));

    let temp = match palette.domain {
        Domain::Private | Domain::Multicast => "warm",
        Domain::Loopback => "cold",
        _ => "neutral",
    };
    g.push_trait(Trait::new("being.material.temperature", temp));

    match c.conveyance_type {
        ConveyanceType::Parked => {
            g.push_trait(Trait::new("behavior.motion.method", "anchored"));
        }
        ConveyanceType::RouteFollowing => {
            g.push_trait(Trait::new("behavior.motion.method", "continuous"));
            g.push_trait(Trait::new("behavior.motion.pace", "moderate"));
            g.push_trait(Trait::new("behavior.motion.regularity", "metronomic"));
            g.push_trait(Trait::new("behavior.motion.path", "linear"));
            g.push_trait(Trait::new("effect.trail.type", "fade"));
            g.push_trait(Trait::new("effect.trail.duration", "instant"));
        }
    }

    g
}

// ═══════════════════════════════════════════════════════════════════════════
// §7  DISTRICT ENVIRONMENT MAPPING
// ═══════════════════════════════════════════════════════════════════════════

/// District environment description — sky, ambient light, weather.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DistrictEnvironment {
    pub sky_colour: [f64; 3],
    pub ambient_light: f64,
    pub sun_direction: [f64; 3],
    pub sun_intensity: f64,
    pub sun_colour: [f64; 3],
    pub is_raining: bool,
    pub wind_direction: f64,
    pub wind_intensity: f64,
}

pub fn map_district_environment(
    _cell: &Cell,
    atmo: &AtmosphereState,
    palette: &AestheticPalette,
) -> DistrictEnvironment {
    let hue_rad = palette.hue.to_radians();
    let pr = palette.popcount_ratio;
    let tod = atmo.time_of_day;

    // §7.1 Sky colour — hue-derived base, time-modulated
    let base_r = 0.4 + 0.3 * hue_rad.cos();
    let base_g = 0.5 + 0.2 * (hue_rad + 2.094).cos();
    let base_b = 0.6 + 0.3 * (hue_rad + 4.189).cos();

    let (sky_r, sky_g, sky_b) = if tod < 0.25 {
        // Night
        (base_r * 0.1, base_g * 0.1, base_b * 0.15)
    } else if tod < 0.30 {
        // Dawn
        let t = (tod - 0.25) / 0.05;
        (
            base_r * 0.1 + t * (base_r * 0.8),
            base_g * 0.1 + t * (base_g * 0.6),
            base_b * 0.15 + t * (base_b * 0.5),
        )
    } else if tod < 0.75 {
        // Day
        (base_r, base_g, base_b)
    } else if tod < 0.833 {
        // Dusk
        let t = (tod - 0.75) / 0.083;
        (
            base_r * (1.0 - t * 0.7),
            base_g * (1.0 - t * 0.8),
            base_b * (1.0 - t * 0.6),
        )
    } else {
        // Night
        (base_r * 0.1, base_g * 0.1, base_b * 0.15)
    };

    // Domain modulation
    let (sky_r, sky_g, sky_b) = match palette.domain {
        Domain::Reserved => {
            let desat = 0.7;
            let avg = (sky_r + sky_g + sky_b) / 3.0;
            (avg + (sky_r - avg) * desat, avg + (sky_g - avg) * desat, avg + (sky_b - avg) * desat)
        }
        Domain::Loopback => (1.0 - sky_r, 1.0 - sky_g, 1.0 - sky_b),
        Domain::Multicast => (
            (sky_r * 1.3).min(1.0),
            (sky_g * 1.3).min(1.0),
            (sky_b * 1.3).min(1.0),
        ),
        _ => (sky_r, sky_g, sky_b),
    };

    // §7.2 Ambient light
    let base_ambient = 0.3 + pr * 0.15;
    let ambient = if atmo.phase as u8 == 0 {
        // Night
        base_ambient * 0.3
    } else if matches!(atmo.phase, crate::gen::atmosphere::Phase::Dawn | crate::gen::atmosphere::Phase::Dusk) {
        base_ambient * 0.6
    } else {
        base_ambient
    };
    let ambient = if atmo.is_raining { ambient * 0.7 } else { ambient };

    // §7.3 Sun/moon
    let sun_dir = [
        -0.4,
        -1.0,
        (tod * std::f64::consts::TAU).cos() * 0.6,
    ];
    let sun_colour = if atmo.phase as u8 == 0 {
        [60.0 / 255.0, 70.0 / 255.0, 120.0 / 255.0]
    } else {
        [1.0, 245.0 / 255.0, 220.0 / 255.0]
    };

    DistrictEnvironment {
        sky_colour: [sky_r.clamp(0.0, 1.0), sky_g.clamp(0.0, 1.0), sky_b.clamp(0.0, 1.0)],
        ambient_light: ambient,
        sun_direction: sun_dir,
        sun_intensity: atmo.sun_intensity,
        sun_colour,
        is_raining: atmo.is_raining,
        wind_direction: atmo.wind_direction,
        wind_intensity: atmo.wind_intensity,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// §8  SURFACE GROWTH OVERLAY
// ═══════════════════════════════════════════════════════════════════════════

pub fn map_surface_growth(coverage: f64, f: &Flora) -> SurfaceGrowthOverlay {
    let mut traits = Vec::new();

    // Blend host texture toward fibrous
    traits.push(
        Trait::new("being.surface.texture", "fibrous")
            .with_param("blend", coverage),
    );

    // Shift age toward ancient
    traits.push(
        Trait::new("being.surface.age", "ancient")
            .with_param("blend", coverage),
    );

    // Shedding emission if applicable
    if f.shedding {
        let shed_rate = seed_param(f.seeds.object_seed, 0xc5);
        traits.push(
            Trait::new("effect.emission.type", "shed")
                .with_param("rate", coverage * shed_rate),
        );
    }

    SurfaceGrowthOverlay { coverage, traits }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::cell::Cell;
    use crate::gen::aesthetic::AestheticPalette;
    use crate::gen::flora::FloraContext;
    use crate::gen::objects::ObjectSeeds;

    fn test_palette(cell: &Cell) -> AestheticPalette {
        AestheticPalette::from_cell(cell)
    }

    fn make_test_creature(cell: &Cell) -> Creature {
        let seeds = ObjectSeeds::from_seed(0xd0c2145e);
        Creature {
            creature_idx: 0,
            creature_seed: 0xd0c2145e,
            ecological_role: EcologicalRole::Aerial,
            size_class: SizeClass::Medium,
            anatomy: Anatomy::Amorphous,
            locomotion_mode: LocomotionMode::Floating,
            materiality: Materiality::Crystalline,
            activity_pattern: ActivityPattern::Crepuscular,
            social_structure: SocialStructure::Pair,
            player_response: PlayerResponse::Flee,
            pace: Pace::Medium,
            locomotion_style: LocomotionStyle::Blinking,
            smoothness: Smoothness::Jerky,
            path_preference: PathPreference::Open,
            sound_tendency: SoundTendency::Constant,
            sound_seed: ha(0xd0c2145e ^ 0x50d1),
            fixture_interaction: FixtureInteraction::Ignore,
            emits_particles: false,
            leaves_trail: true,
            rest_frequency: 0.841,
            idle_behaviours: vec![],
            form_id: 0,
            object_id: 0,
            seeds,
            tier: crate::gen::objects::Tier::TimeSynced,
        }
    }

    #[test]
    fn creature_graph_has_all_roots() {
        let cell = Cell::from_octets(1, 0, 0);
        let palette = test_palette(&cell);
        let creature = make_test_creature(&cell);
        let graph = map_creature(&creature, &palette);

        let roots: std::collections::HashSet<&str> = graph
            .traits
            .iter()
            .map(|t| t.path.split('.').next().unwrap())
            .collect();

        assert!(roots.contains("being"), "missing being root");
        assert!(roots.contains("behavior"), "missing behavior root");
        assert!(roots.contains("effect"), "missing effect root");
        assert!(roots.contains("relation"), "missing relation root");
    }

    #[test]
    fn creature_graph_paths_are_valid() {
        let cell = Cell::from_octets(1, 0, 0);
        let palette = test_palette(&cell);
        let creature = make_test_creature(&cell);
        let graph = map_creature(&creature, &palette);

        for t in &graph.traits {
            let segs: Vec<_> = t.path.split('.').collect();
            assert_eq!(segs.len(), 3, "bad path: {}", t.path);
            assert!(
                ["being", "behavior", "effect", "relation"].contains(&segs[0]),
                "bad root in path: {}",
                t.path
            );
        }
    }

    #[test]
    fn creature_crystalline_has_emission() {
        let cell = Cell::from_octets(1, 0, 0);
        let palette = test_palette(&cell);
        let creature = make_test_creature(&cell);
        let graph = map_creature(&creature, &palette);

        let has_emission = graph
            .traits
            .iter()
            .any(|t| t.path == "effect.emission.type");
        assert!(has_emission, "crystalline creature should have emission");
    }

    #[test]
    fn creature_blinking_has_trail() {
        let cell = Cell::from_octets(1, 0, 0);
        let palette = test_palette(&cell);
        let creature = make_test_creature(&cell);
        let graph = map_creature(&creature, &palette);

        let trail = graph
            .traits
            .iter()
            .find(|t| t.path == "effect.trail.type");
        assert!(trail.is_some(), "blinking creature should have trail");
        assert_eq!(trail.unwrap().term, "echo");
    }

    #[test]
    fn creature_has_sequences() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = test_palette(&cell);
        let creature = make_test_creature(&cell);
        let graph = map_creature(&creature, &palette);
        // Crystalline + blinking = at least sequence_count=2
        assert!(!graph.sequences.is_empty(), "should have at least 1 sequence");
    }

    #[test]
    fn creature_silhouette_derivation() {
        let cell = Cell::from_octets(1, 0, 0);
        let mut c = make_test_creature(&cell);

        // Floating + medium = wide
        c.locomotion_mode = LocomotionMode::Floating;
        c.size_class = SizeClass::Medium;
        assert_eq!(derive_creature_silhouette(&c), "wide");

        // Floating + tiny = compact
        c.size_class = SizeClass::Tiny;
        assert_eq!(derive_creature_silhouette(&c), "compact");

        // Surface + bilateral + large = tall
        c.locomotion_mode = LocomotionMode::Surface;
        c.anatomy = Anatomy::Bilateral;
        c.size_class = SizeClass::Large;
        assert_eq!(derive_creature_silhouette(&c), "tall");

        // Aquatic = trailing
        c.locomotion_mode = LocomotionMode::Aquatic;
        assert_eq!(derive_creature_silhouette(&c), "trailing");
    }

    #[test]
    fn creature_narrative_derivation() {
        let cell = Cell::from_octets(1, 0, 0);
        let mut c = make_test_creature(&cell);

        // Spirit + solitary = remnant
        c.materiality = Materiality::Spirit;
        c.social_structure = SocialStructure::Solitary;
        assert_eq!(derive_creature_narrative(&c), "remnant");

        // High rest + perch = guardian
        c.materiality = Materiality::Flesh;
        c.rest_frequency = 0.9;
        c.fixture_interaction = FixtureInteraction::Perch;
        assert_eq!(derive_creature_narrative(&c), "guardian");
    }

    #[test]
    fn fixture_graph_valid_paths() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = test_palette(&cell);
        let fixture = Fixture {
            role: FixtureRole::Illumination,
            position: crate::types::Point::new(10.0, 20.0),
            orientation: 0.0,
            form_class: FormClass::Column,
            attachment: crate::gen::objects::Attachment::Floor,
            scale_height: 2.5,
            scale_footprint: 0.5,
            hazard: crate::gen::objects::Hazard::None,
            active_state: true,
            emissive_light: true,
            emissive_sound: false,
            emissive_particles: false,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0x1234),
            tier: crate::gen::objects::Tier::Seedable,
            road_edge: false,
        };
        let graph = map_fixture(&fixture, &palette);

        for t in &graph.traits {
            let segs: Vec<_> = t.path.split('.').collect();
            assert_eq!(segs.len(), 3, "bad path: {}", t.path);
        }
        // Illumination should have emission
        assert!(graph.traits.iter().any(|t| t.path == "effect.emission.type"));
    }

    #[test]
    fn illumination_fixture_has_sequence() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = test_palette(&cell);
        let fixture = Fixture {
            role: FixtureRole::Illumination,
            position: crate::types::Point::new(10.0, 20.0),
            orientation: 0.0,
            form_class: FormClass::Column,
            attachment: crate::gen::objects::Attachment::Floor,
            scale_height: 2.5,
            scale_footprint: 0.5,
            hazard: crate::gen::objects::Hazard::None,
            active_state: true,
            emissive_light: true,
            emissive_sound: false,
            emissive_particles: false,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0x1234),
            tier: crate::gen::objects::Tier::Seedable,
            road_edge: false,
        };
        let graph = map_fixture(&fixture, &palette);
        assert!(!graph.sequences.is_empty(), "state-cycling illumination should have a sequence");
    }

    #[test]
    fn flora_graph_valid() {
        let cell = Cell::from_octets(15, 255, 255);
        let palette = test_palette(&cell);
        let flora = Flora {
            context: FloraContext::BlockLevel,
            growth_form: GrowthForm::Tree,
            density_mode: DensityMode::Moderate,
            position: crate::types::Point::new(50.0, 50.0),
            orientation: 0.5,
            scale: 4.0,
            maturity: 0.5,
            shedding: true,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0x5678),
            tier: crate::gen::objects::Tier::Seedable,
        };
        let graph = map_flora(&flora, &palette);

        // Should have being, behavior, effect
        let roots: std::collections::HashSet<&str> = graph
            .traits
            .iter()
            .map(|t| t.path.split('.').next().unwrap())
            .collect();
        assert!(roots.contains("being"));
        assert!(roots.contains("behavior"));
        assert!(roots.contains("effect"), "shedding tree should have effect");
        // Should have wind-shed sequence
        assert!(!graph.sequences.is_empty());
    }

    #[test]
    fn building_graph_valid() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = test_palette(&cell);
        let plot = BuildingPlot {
            plot_idx: 0,
            polygon: crate::types::Polygon::new(vec![
                crate::types::Point::new(0.0, 0.0),
                crate::types::Point::new(10.0, 0.0),
                crate::types::Point::new(10.0, 10.0),
                crate::types::Point::new(0.0, 10.0),
            ]),
            centroid: crate::types::Point::new(5.0, 5.0),
            area: 100.0,
            plot_seed: 0x9abc,
            archetype: Archetype::Tower,
            height: 8.0,
            is_public: true,
            public_subtype: None,
            interior_light: Some(0.5),
            entry: None,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0x9abc),
        };
        let graph = map_building(&plot, &palette);

        let has_scale = graph.traits.iter().any(|t| t.path == "being.form.scale");
        assert!(has_scale);
        // Tower should be "tall"
        let sil = graph.traits.iter().find(|t| t.path == "being.form.silhouette").unwrap();
        assert_eq!(sil.term, "tall");
        // Public building should emit
        assert!(graph.traits.iter().any(|t| t.path == "effect.emission.type"));
    }

    #[test]
    fn conveyance_parked_graph() {
        let cell = Cell::from_octets(10, 0, 0);
        let palette = test_palette(&cell);
        let conv = Conveyance {
            idx: 0,
            conveyance_type: ConveyanceType::Parked,
            position: crate::types::Point::new(30.0, 30.0),
            orientation: 0.0,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0xdef0),
            tier: crate::gen::objects::Tier::Seedable,
            route: None,
            loop_period_ms: None,
        };
        let graph = map_conveyance(&conv, &palette);
        let method = graph.traits.iter().find(|t| t.path == "behavior.motion.method").unwrap();
        assert_eq!(method.term, "anchored");
    }

    #[test]
    fn conveyance_moving_graph() {
        let cell = Cell::from_octets(10, 0, 0);
        let palette = test_palette(&cell);
        let conv = Conveyance {
            idx: 0,
            conveyance_type: ConveyanceType::RouteFollowing,
            position: crate::types::Point::new(30.0, 30.0),
            orientation: 0.0,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0xdef0),
            tier: crate::gen::objects::Tier::TimeSynced,
            route: Some(vec![0, 1, 2]),
            loop_period_ms: Some(30000),
        };
        let graph = map_conveyance(&conv, &palette);
        let method = graph.traits.iter().find(|t| t.path == "behavior.motion.method").unwrap();
        assert_eq!(method.term, "continuous");
        // Should have trail
        assert!(graph.traits.iter().any(|t| t.path == "effect.trail.type"));
    }

    #[test]
    fn district_environment_day_night() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = test_palette(&cell);

        // Midday
        let atmo_day = AtmosphereState {
            time_of_day: 0.5,
            hour: 12,
            phase: crate::gen::atmosphere::Phase::Day,
            phase_t: 0.0,
            sun_altitude: 1.0,
            sun_intensity: 0.8,
            is_raining: false,
            rain_intensity: 0.0,
            wind_direction: 0.0,
            wind_intensity: 0.1,
            weather_group: 0,
        };
        let env_day = map_district_environment(&cell, &atmo_day, &palette);

        // Midnight
        let atmo_night = AtmosphereState {
            time_of_day: 0.1,
            hour: 2,
            phase: crate::gen::atmosphere::Phase::Night,
            phase_t: 0.0,
            sun_altitude: -0.8,
            sun_intensity: 0.03,
            is_raining: false,
            rain_intensity: 0.0,
            wind_direction: 0.0,
            wind_intensity: 0.05,
            weather_group: 0,
        };
        let env_night = map_district_environment(&cell, &atmo_night, &palette);

        // Day should be brighter
        assert!(env_day.ambient_light > env_night.ambient_light);
        assert!(env_day.sky_colour[0] > env_night.sky_colour[0]);
    }

    #[test]
    fn surface_growth_overlay() {
        let flora = Flora {
            context: FloraContext::SurfaceGrowth,
            growth_form: GrowthForm::Vine,
            density_mode: DensityMode::Moderate,
            position: crate::types::Point::new(0.0, 0.0),
            orientation: 0.0,
            scale: 2.0,
            maturity: 0.7,
            shedding: true,
            form_id: 0,
            object_id: 0,
            seeds: ObjectSeeds::from_seed(0xabcd),
            tier: crate::gen::objects::Tier::Seedable,
        };
        let overlay = map_surface_growth(0.7, &flora);
        assert_eq!(overlay.coverage, 0.7);
        assert!(overlay.traits.len() >= 2); // texture + age at minimum
    }
}

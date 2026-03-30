//! being.surface + being.material → Astral Material translation.
//!
//! Maps HDL surface texture, opacity, substance, temperature traits to
//! Astral's Material: baseColor, brightness, emissive, roughness,
//! reflectivity, glyphStyle, motionBehavior.

use serde::{Deserialize, Serialize};

use crate::hdl::traits::{DescriptionGraph, Trait};

/// Astral Color (0–255 RGB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Color {
    pub fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }

    /// HSL to RGB (h in degrees, s and l in 0–1), output 0–255.
    pub fn from_hsl(h: f64, s: f64, l: f64) -> Self {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let h2 = h / 60.0;
        let x = c * (1.0 - (h2 % 2.0 - 1.0).abs());
        let (r1, g1, b1) = match h2 as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c * 0.5;
        Self {
            r: ((r1 + m) * 255.0).clamp(0.0, 255.0),
            g: ((g1 + m) * 255.0).clamp(0.0, 255.0),
            b: ((b1 + m) * 255.0).clamp(0.0, 255.0),
        }
    }
}

/// Astral GlyphStyle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GlyphStyle {
    #[serde(rename = "dense")]
    Dense,
    #[serde(rename = "light")]
    Light,
    #[serde(rename = "round")]
    Round,
    #[serde(rename = "angular")]
    Angular,
    #[serde(rename = "line")]
    Line,
    #[serde(rename = "noise")]
    Noise,
    #[serde(rename = "block")]
    Block,
    #[serde(rename = "symbolic")]
    Symbolic,
}

/// Astral MotionBehavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionBehavior {
    #[serde(rename = "type")]
    pub motion_type: String,
    pub speed: f64,
}

/// Astral Material — mirrors the TypeScript Material interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    #[serde(rename = "baseColor")]
    pub base_color: Color,
    pub brightness: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emissive: Option<f64>,
    pub roughness: f64,
    pub reflectivity: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transparency: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "glyphStyle")]
    pub glyph_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "motionBehavior")]
    pub motion_behavior: Option<MotionBehavior>,
}

/// Resolve an HDL description graph into an Astral Material.
///
/// Uses district_hue for base colour derivation.
pub fn resolve_material(graph: &DescriptionGraph, district_hue: f64) -> Material {
    let texture = find_trait(graph, "being.surface.texture");
    let opacity = find_trait(graph, "being.surface.opacity");
    let substance = find_trait(graph, "being.material.substance");
    let temperature = find_trait(graph, "being.material.temperature");
    let density = find_trait(graph, "being.material.density");
    let emission = find_trait(graph, "effect.emission.type");
    let motion = find_trait(graph, "behavior.motion.method");

    // ── Base colour from substance palette + temperature shift + hue rotation ──
    // Per astral-projection.md §6.2.1

    // Step 1: Substance palette — fixed RGB base per substance type
    let (mut br, mut bg, mut bb): (f64, f64, f64) = match substance.map(|t| t.term.as_str()) {
        Some("mineral") => (140.0, 160.0, 200.0),       // blue-grey
        Some("organic") => (120.0, 160.0, 90.0),        // green-brown
        Some("spectral") => (180.0, 180.0, 220.0),      // pale lavender
        Some("constructed") => (170.0, 160.0, 140.0),   // warm grey
        Some("elemental") => (200.0, 140.0, 80.0),      // amber
        _ => (160.0, 155.0, 150.0),                      // neutral grey
    };

    // Step 2: Temperature shift — additive RGB offsets
    let (tr, tg, tb) = match temperature.map(|t| t.term.as_str()) {
        Some("cold") => (-30.0, -10.0, 40.0),
        Some("cool") => (-15.0, 0.0, 20.0),
        Some("warm") => (20.0, 5.0, -15.0),
        Some("hot") => (40.0, -5.0, -30.0),
        _ => (0.0, 0.0, 0.0), // neutral
    };
    br = (br + tr).clamp(0.0_f64, 255.0);
    bg = (bg + tg).clamp(0.0_f64, 255.0);
    bb = (bb + tb).clamp(0.0_f64, 255.0);

    // Step 3: Hue rotation — rotate the base colour by the district hue,
    // modulated by the per-entity hue_seed for variety within a district.
    let hue_seed = substance
        .and_then(|t| t.params.get("hue_seed"))
        .copied()
        .unwrap_or(0.5);
    let rotation_deg = district_hue * 0.5 + hue_seed * 40.0 - 20.0;
    let base_color = rotate_rgb(Color::new(br, bg, bb), rotation_deg);

    // ── Brightness from density ──

    let brightness = match density.map(|t| t.term.as_str()) {
        Some("gossamer") => 0.9,
        Some("light") => 0.7,
        Some("moderate") => 0.6,
        Some("dense") => 0.5,
        Some("massive") => 0.4,
        _ => 0.6,
    };

    // ── Roughness from texture ──

    let roughness = match texture.map(|t| t.term.as_str()) {
        Some("smooth") | Some("polished") | Some("glazed") => 0.1,
        Some("faceted") | Some("crystalline") | Some("geometric") => 0.3,
        Some("fibrous") | Some("woven") | Some("organic") => 0.6,
        Some("rough") | Some("granular") | Some("pitted") => 0.8,
        Some("fluid") | Some("rippled") => 0.2,
        _ => 0.5,
    };

    // ── Reflectivity from texture params ──

    let reflectivity = texture
        .and_then(|t| t.params.get("reflectance"))
        .copied()
        .unwrap_or(0.1);

    // ── Transparency from opacity ──

    let transparency = match opacity.map(|t| t.term.as_str()) {
        Some("transparent") => Some(0.8),
        Some("translucent") => {
            let level = opacity.and_then(|t| t.params.get("level")).copied().unwrap_or(0.4);
            Some(1.0 - level)
        }
        Some("shifting") => Some(0.5),
        _ => None,
    };

    // ── Emissive from effect.emission ──

    let emissive = emission.map(|_| {
        let intensity_trait = find_trait(graph, "effect.emission.intensity");
        match intensity_trait.map(|t| t.term.as_str()) {
            Some("overwhelming") => 1.0,
            Some("strong") => 0.8,
            Some("moderate") => 0.5,
            Some("subtle") => 0.3,
            Some("faint") => 0.15,
            _ => 0.2,
        }
    });

    // ── Glyph style from texture term ──

    let glyph_style = texture.map(|t| match t.term.as_str() {
        "smooth" | "polished" | "glazed" | "liquid" => "round",
        "faceted" | "crystalline" | "geometric" | "angular" => "angular",
        "fibrous" | "woven" | "organic" | "bark" => "noise",
        "rough" | "granular" | "pitted" | "corroded" => "dense",
        "fluid" | "rippled" | "turbulent" => "line",
        "inscribed" | "runic" | "etched" => "symbolic",
        "bolted" | "riveted" | "plated" => "block",
        _ => "noise",
    }).map(String::from);

    // ── Motion behavior from behavior.motion ──

    let motion_behavior = motion.and_then(|m| {
        match m.term.as_str() {
            "anchored" => None, // static
            "continuous" | "drifting" => Some(MotionBehavior {
                motion_type: "flow".into(),
                speed: m.params.get("interval").copied().unwrap_or(1.0),
            }),
            "oscillating" => Some(MotionBehavior {
                motion_type: "pulse".into(),
                speed: m.params.get("interval").copied().unwrap_or(2.0),
            }),
            "discontinuous" => Some(MotionBehavior {
                motion_type: "flicker".into(),
                speed: m.params.get("interval").copied().unwrap_or(1.0),
            }),
            _ => None,
        }
    });

    Material {
        base_color,
        brightness,
        emissive,
        roughness,
        reflectivity,
        transparency,
        glyph_style,
        motion_behavior,
    }
}

/// Rotate an RGB colour by a hue angle (degrees).
/// Converts to HSL, shifts hue, converts back.
fn rotate_rgb(c: Color, degrees: f64) -> Color {
    // RGB 0-255 → 0-1
    let r = c.r / 255.0;
    let g = c.g / 255.0;
    let b = c.b / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < 1e-10 {
        // Achromatic — just shift lightness slightly
        return Color::from_hsl(degrees.rem_euclid(360.0), 0.05, l);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < 1e-10 {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) * 60.0
    } else if (max - g).abs() < 1e-10 {
        ((b - r) / d + 2.0) * 60.0
    } else {
        ((r - g) / d + 4.0) * 60.0
    };

    let new_h = (h + degrees).rem_euclid(360.0);
    Color::from_hsl(new_h, s, l)
}

fn find_trait<'a>(graph: &'a DescriptionGraph, path: &str) -> Option<&'a Trait> {
    graph.traits.iter().find(|t| t.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdl::traits::{DescriptionGraph, Trait};

    #[test]
    fn crystalline_material() {
        let mut g = DescriptionGraph::new();
        g.push_trait(Trait::new("being.surface.texture", "faceted")
            .with_param("complexity", 0.7)
            .with_param("reflectance", 0.6));
        g.push_trait(Trait::new("being.surface.opacity", "translucent")
            .with_param("level", 0.4));
        g.push_trait(Trait::new("being.material.substance", "mineral")
            .with_param("hue_seed", 0.5));
        g.push_trait(Trait::new("being.material.temperature", "cold"));
        g.push_trait(Trait::new("being.material.density", "moderate"));
        g.push_trait(Trait::new("effect.emission.type", "pulse"));
        g.push_trait(Trait::new("effect.emission.intensity", "faint"));

        let mat = resolve_material(&g, 54.0);

        assert!(mat.roughness < 0.5, "faceted should be low roughness");
        assert!(mat.reflectivity > 0.3, "high reflectance");
        assert!(mat.transparency.is_some(), "translucent should have transparency");
        assert!(mat.emissive.is_some(), "emission should set emissive");
        assert_eq!(mat.glyph_style.as_deref(), Some("angular"));
    }

    #[test]
    fn organic_material() {
        let mut g = DescriptionGraph::new();
        g.push_trait(Trait::new("being.surface.texture", "fibrous"));
        g.push_trait(Trait::new("being.surface.opacity", "solid"));
        g.push_trait(Trait::new("being.material.substance", "organic"));
        g.push_trait(Trait::new("being.material.temperature", "warm"));
        g.push_trait(Trait::new("being.material.density", "light"));

        let mat = resolve_material(&g, 120.0);

        assert!(mat.roughness > 0.4, "fibrous should be rough");
        assert!(mat.transparency.is_none(), "solid = no transparency");
        assert!(mat.emissive.is_none(), "no emission = no emissive");
        assert_eq!(mat.glyph_style.as_deref(), Some("noise"));
    }

    #[test]
    fn motion_behavior_mapping() {
        let mut g = DescriptionGraph::new();
        g.push_trait(Trait::new("behavior.motion.method", "oscillating")
            .with_param("interval", 3.0));

        let mat = resolve_material(&g, 0.0);
        let mb = mat.motion_behavior.unwrap();
        assert_eq!(mb.motion_type, "pulse");
        assert!((mb.speed - 3.0).abs() < 0.01);
    }

    #[test]
    fn substance_palettes_differentiate() {
        // Different substance types should produce visibly different base colours
        let substances = ["mineral", "organic", "spectral", "constructed", "elemental"];
        let mut colors = Vec::new();
        for sub in &substances {
            let mut g = DescriptionGraph::new();
            g.push_trait(Trait::new("being.material.substance", sub).with_param("hue_seed", 0.5));
            g.push_trait(Trait::new("being.material.temperature", "neutral"));
            let mat = resolve_material(&g, 180.0);
            colors.push((mat.base_color.r, mat.base_color.g, mat.base_color.b));
        }
        // Each pair should differ by at least 15 in some channel
        for i in 0..colors.len() {
            for j in (i+1)..colors.len() {
                let dr = (colors[i].0 - colors[j].0).abs();
                let dg = (colors[i].1 - colors[j].1).abs();
                let db = (colors[i].2 - colors[j].2).abs();
                let max_diff = dr.max(dg).max(db);
                assert!(max_diff > 15.0,
                    "{} vs {} differ by only {:.0} max channel",
                    substances[i], substances[j], max_diff);
            }
        }
    }

    #[test]
    fn hsl_to_rgb_red() {
        let c = Color::from_hsl(0.0, 1.0, 0.5);
        assert!((c.r - 255.0).abs() < 1.0);
        assert!(c.g < 1.0);
        assert!(c.b < 1.0);
    }
}

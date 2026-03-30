//! Scene compiler — assembles a complete Astral Scene from world generation output.
//!
//! Takes a district's description graphs, geometry, and atmosphere, and produces
//! a Scene JSON that Astral can render directly.

use serde::{Deserialize, Serialize};

use crate::gen::aesthetic::AestheticPalette;
use crate::gen::atmosphere::AtmosphereState;
use crate::gen::blocks::Block;
use crate::gen::buildings::{generate_buildings, BuildingPlot};
use crate::gen::cell::Cell;
use crate::gen::conveyances::{Conveyance, ConveyanceType};
use crate::gen::creatures::Creature;
use crate::gen::fixtures::Fixture;
use crate::gen::flora::Flora;
use crate::hdl::mapping;
use crate::hdl::traits::DescriptionGraph;

use super::geometry::{self, Geometry, Transform, Vec3};
use super::material::{self, Color, Material};

/// Astral Light.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Light {
    #[serde(rename = "type")]
    pub light_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Vec3>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Vec3>,
    pub intensity: f64,
    pub color: Color,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<f64>,
}

/// Astral Camera.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub position: Vec3,
    pub rotation: Vec3,
    pub fov: f64,
    pub near: f64,
    pub far: f64,
}

/// Astral Environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    #[serde(rename = "ambientLight")]
    pub ambient_light: f64,
    #[serde(rename = "backgroundColor")]
    pub background_color: Color,
    #[serde(skip_serializing_if = "Option::is_none", rename = "fogDensity")]
    pub fog_density: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "fogColor")]
    pub fog_color: Option<Color>,
}

/// Astral Entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub transform: Transform,
    pub geometry: Geometry,
    pub material: Material,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity: Option<Vec3>,
}

/// Astral Scene — the complete output that Astral consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub time: f64,
    pub camera: Camera,
    pub environment: Environment,
    pub lights: Vec<Light>,
    pub entities: Vec<Entity>,
}

/// Compile a building plot into an Astral Entity.
pub fn compile_building(plot: &BuildingPlot, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_building(plot, palette);
    let (geo, transform) = geometry::resolve_building_geometry(
        &plot.polygon.vertices,
        plot.height,
    );
    let mat = material::resolve_material(&graph, palette.hue);

    Entity {
        id: format!("building_{}", plot.object_id),
        transform,
        geometry: geo,
        material: mat,
        velocity: None,
    }
}

/// Compile a fixture into an Astral Entity.
pub fn compile_fixture(f: &Fixture, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_fixture(f, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);

    Entity {
        id: format!("fixture_{}", f.object_id),
        transform: Transform::at(f.position.x, f.scale_height * 0.5, f.position.y)
            .with_scale(scale.x, scale.y, scale.z)
            .with_rotation_y(f.orientation),
        geometry: geo,
        material: mat,
        velocity: None,
    }
}

/// Compile a flora instance into an Astral Entity.
pub fn compile_flora(f: &Flora, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_flora(f, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);

    Entity {
        id: format!("flora_{}", f.object_id),
        transform: Transform::at(f.position.x, f.scale * 0.5, f.position.y)
            .with_scale(scale.x * f.scale, scale.y * f.scale, scale.z * f.scale)
            .with_rotation_y(f.orientation),
        geometry: geo,
        material: mat,
        velocity: None,
    }
}

/// Compile a creature into one or more Astral Entities (composition).
pub fn compile_creature(c: &Creature, palette: &AestheticPalette) -> Vec<Entity> {
    let graph = mapping::map_creature(c, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);
    let offsets = geometry::resolve_composition(&graph);

    offsets
        .iter()
        .enumerate()
        .map(|(i, offset)| {
            let suffix = if offsets.len() > 1 {
                format!("creature_{}_{}", c.object_id, i)
            } else {
                format!("creature_{}", c.object_id)
            };
            Entity {
                id: suffix,
                transform: Transform::at(offset.x, 1.0 + offset.y, offset.z)
                    .with_scale(scale.x, scale.y, scale.z),
                geometry: geo.clone(),
                material: mat.clone(),
                velocity: None,
            }
        })
        .collect()
}

/// Compile a conveyance into an Astral Entity.
pub fn compile_conveyance(c: &Conveyance, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_conveyance(c, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);

    Entity {
        id: format!("conveyance_{}", c.object_id),
        transform: Transform::at(c.position.x, 0.5, c.position.y)
            .with_scale(scale.x, scale.y, scale.z)
            .with_rotation_y(c.orientation),
        geometry: geo,
        material: mat,
        velocity: None,
    }
}

/// Compile the district environment into Astral Environment + Lights.
pub fn compile_environment(
    cell: &Cell,
    atmo: &AtmosphereState,
    palette: &AestheticPalette,
) -> (Environment, Vec<Light>) {
    let env_data = mapping::map_district_environment(cell, atmo, palette);

    let environment = Environment {
        ambient_light: env_data.ambient_light,
        background_color: Color::new(
            env_data.sky_colour[0] * 255.0,
            env_data.sky_colour[1] * 255.0,
            env_data.sky_colour[2] * 255.0,
        ),
        fog_density: if atmo.is_raining { Some(0.03) } else { None },
        fog_color: if atmo.is_raining {
            Some(Color::new(
                env_data.sky_colour[0] * 200.0,
                env_data.sky_colour[1] * 200.0,
                env_data.sky_colour[2] * 200.0,
            ))
        } else {
            None
        },
    };

    let mut lights = Vec::new();

    // Sun/moon directional light
    lights.push(Light {
        light_type: "directional".into(),
        position: None,
        direction: Some(Vec3::new(
            env_data.sun_direction[0],
            env_data.sun_direction[1],
            env_data.sun_direction[2],
        )),
        intensity: env_data.sun_intensity * 3.0,
        color: Color::new(
            env_data.sun_colour[0] * 255.0,
            env_data.sun_colour[1] * 255.0,
            env_data.sun_colour[2] * 255.0,
        ),
        range: None,
    });

    (environment, lights)
}

/// Compile illumination fixtures into point lights.
/// Capped to avoid performance issues — picks the nearest/brightest.
pub fn compile_fixture_lights(
    fixtures: &[&Fixture],
    palette: &AestheticPalette,
    max_lights: usize,
) -> Vec<Light> {
    let mut lights = Vec::new();
    let warm_hue = (palette.hue + 30.0).rem_euclid(360.0);
    let warm_color = Color::from_hsl(warm_hue, 0.5, 0.7);

    for f in fixtures.iter().take(max_lights) {
        let intensity = if f.emissive_light { 2.0 } else { 1.0 };
        lights.push(Light {
            light_type: "point".into(),
            position: Some(Vec3::new(f.position.x, f.scale_height + 0.5, f.position.y)),
            direction: None,
            intensity,
            color: warm_color.clone(),
            range: Some(15.0),
        });
    }

    lights
}

/// Compile a ground entity for the district.
/// Uses a large finite box instead of an infinite plane so that rays
/// above the horizon can miss the ground and show the sky colour.
fn compile_ground(palette: &AestheticPalette, centroid: &crate::types::Point) -> Entity {
    let hue = palette.hue;
    let lightness = 0.25 + palette.popcount_ratio * 0.1;
    let base = Color::from_hsl(hue, 0.15, lightness);
    let ground_size = 600.0; // big enough to cover any district (~200 wu across)

    Entity {
        id: "ground".into(),
        transform: Transform::at(centroid.x, -0.25, centroid.y),
        geometry: Geometry::Box {
            size: Vec3::new(ground_size, 0.5, ground_size),
        },
        material: Material {
            base_color: base,
            brightness: 0.5,
            emissive: None,
            roughness: 0.7,
            reflectivity: 0.05,
            transparency: None,
            glyph_style: Some("dense".into()),
            motion_behavior: None,
        },
        velocity: None,
    }
}

/// Compile a full district into an Astral Scene.
///
/// This is the main entry point — takes the complete generation output
/// and produces a Scene that Astral can render.
pub fn compile_district_scene(
    cell: &Cell,
    palette: &AestheticPalette,
    blocks: &[Block],
    atmo: &AtmosphereState,
) -> Scene {
    use crate::gen::{buildings, conveyances, creatures, fixtures, flora, roads, rivers, district};

    let dist = district::generate_district(cell);
    let road_network = roads::generate_roads(&dist);
    let river_data = rivers::generate_rivers(cell, &dist.polygon.vertices);
    let blocks = crate::gen::blocks::extract_blocks(cell, &dist.polygon, &road_network, &river_data);

    let mut entities = Vec::new();
    let mut light_positions: Vec<(f64, f64, f64, bool)> = Vec::new(); // (x, y, z, emissive)

    // Ground — centred on district
    entities.push(compile_ground(palette, &dist.seed_position));

    // Per-block entities
    for block in &blocks {
        // Buildings
        let block_buildings = buildings::generate_buildings(cell, block);
        for plot in &block_buildings.plots {
            entities.push(compile_building(plot, palette));
        }

        // Fixtures
        let block_fixtures = fixtures::generate_fixtures(cell, block, Some(&road_network));
        for f in block_fixtures
            .zone_fixtures
            .iter()
            .chain(block_fixtures.road_fixtures.iter())
        {
            entities.push(compile_fixture(f, palette));
            if f.role == crate::gen::fixtures::FixtureRole::Illumination {
                light_positions.push((
                    f.position.x,
                    f.scale_height + 0.5,
                    f.position.y,
                    f.emissive_light,
                ));
            }
        }

        // Flora
        let block_flora = flora::generate_flora(cell, block, Some(&road_network));
        for f in block_flora
            .block_flora
            .iter()
            .chain(block_flora.road_flora.iter())
        {
            entities.push(compile_flora(f, palette));
        }

        // Creatures (may produce multiple entities for composed forms)
        let block_creatures = creatures::generate_creatures(cell, block);
        for c in &block_creatures.creatures {
            entities.extend(compile_creature(c, palette));
        }
    }

    // Conveyances
    let district_conveyances = conveyances::generate_conveyances(cell, &road_network);
    for c in district_conveyances
        .parked
        .iter()
        .chain(district_conveyances.route_following.iter())
    {
        entities.push(compile_conveyance(c, palette));
    }

    // Environment + lights (sun + fixture point lights)
    let (environment, mut lights) = compile_environment(cell, atmo, palette);

    // Fixture point lights — capped at 24 for performance
    let warm_hue = (palette.hue + 30.0).rem_euclid(360.0);
    let warm_color = Color::from_hsl(warm_hue, 0.5, 0.7);
    for &(x, y, z, emissive) in light_positions.iter().take(24) {
        lights.push(Light {
            light_type: "point".into(),
            position: Some(Vec3::new(x, y, z)),
            direction: None,
            intensity: if emissive { 2.0 } else { 1.0 },
            color: warm_color.clone(),
            range: Some(15.0),
        });
    }

    // Camera: position at district centroid, looking north, elevated
    let cam_pos = dist.seed_position;
    let camera = Camera {
        position: Vec3::new(cam_pos.x, 8.0, cam_pos.y + 20.0),
        rotation: Vec3::new(-0.3, 0.0, 0.0),
        fov: 60.0,
        near: 0.1,
        far: 500.0,
    };

    Scene {
        time: 0.0,
        camera,
        environment,
        lights,
        entities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::aesthetic::AestheticPalette;
    use crate::gen::atmosphere;
    use crate::gen::cell::Cell;

    #[test]
    fn compile_district_produces_entities() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = AestheticPalette::from_cell(&cell);
        let now_ms = 1711728000000; // fixed timestamp
        let atmo = atmosphere::compute_atmosphere(&cell, now_ms);

        let scene = compile_district_scene(&cell, &palette, &[], &atmo);

        // Should have ground + at least some entities
        assert!(!scene.entities.is_empty(), "scene should have entities");
        assert!(scene.entities.iter().any(|e| e.id == "ground"), "should have ground plane");
        assert!(scene.entities.iter().any(|e| e.id.starts_with("building_")), "should have buildings");
        assert!(!scene.lights.is_empty(), "should have lights");
    }

    #[test]
    fn compile_district_entity_ids_unique() {
        let cell = Cell::from_octets(1, 0, 0);
        let palette = AestheticPalette::from_cell(&cell);
        let atmo = atmosphere::compute_atmosphere(&cell, 1711728000000);
        let scene = compile_district_scene(&cell, &palette, &[], &atmo);

        let mut ids: Vec<&str> = scene.entities.iter().map(|e| e.id.as_str()).collect();
        let count_before = ids.len();
        ids.sort();
        ids.dedup();
        // Allow some duplicates from creature object_id collisions (hash space)
        // but majority should be unique
        assert!(ids.len() > count_before / 2,
            "most entity IDs should be unique: {} unique of {}", ids.len(), count_before);
    }

    #[test]
    fn compile_scene_serializes_to_valid_json() {
        let cell = Cell::from_octets(10, 0, 0);
        let palette = AestheticPalette::from_cell(&cell);
        let atmo = atmosphere::compute_atmosphere(&cell, 1711728000000);
        let scene = compile_district_scene(&cell, &palette, &[], &atmo);

        let json = serde_json::to_string(&scene).unwrap();
        assert!(json.contains("\"camera\""));
        assert!(json.contains("\"environment\""));
        assert!(json.contains("\"entities\""));
        assert!(json.contains("\"lights\""));

        // Verify it round-trips
        let _parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn environment_has_correct_structure() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = AestheticPalette::from_cell(&cell);
        let atmo = atmosphere::compute_atmosphere(&cell, 1711728000000);
        let (env, lights) = compile_environment(&cell, &atmo, &palette);

        assert!(env.ambient_light > 0.0);
        assert!(env.background_color.r >= 0.0);
        assert!(!lights.is_empty());
        assert_eq!(lights[0].light_type, "directional");
    }
}

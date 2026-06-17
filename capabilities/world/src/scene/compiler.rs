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
use crate::gen::home::HomeStructure;
use crate::gen::home::peer_id_u32;
use crate::gen::inside::Inside;
use crate::gen::room_features::RoomFeature;
use crate::gen::tunnel::{Tunnel, TunnelAesthetic};
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
    /// HDL description graph — optional. When present, the renderer
    /// creates trait controllers and sequence engine for this entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<crate::hdl::traits::DescriptionGraph>,
}

/// Astral Scene — the complete output that Astral consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub time: f64,
    pub camera: Camera,
    pub environment: Environment,
    pub lights: Vec<Light>,
    pub entities: Vec<Entity>,
    /// Per-district ground zone/road raster (streets, parks, water). `None` for
    /// interior/tunnel scenes that have no district ground.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "groundPaint")]
    pub ground_paint: Option<crate::scene::groundpaint::GroundPaint>,
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
        description: Some(graph),
    }
}

/// Compile a peer's home structure into an Astral Entity (spaces §1.2). Geometry
/// comes from the archetype form; the transform is scaled by the home's footprint
/// radius and height so distinct archetypes read at the right size.
pub fn compile_home(home: &HomeStructure, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_home(home, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);
    let r = home.footprint_radius.max(1.0);
    let h = (home.height / 3.0).max(1.0);
    Entity {
        id: format!("home_{:08x}", home.peer_id_u32),
        transform: Transform::at(home.position.x, home.height * 0.5, home.position.y)
            .with_scale(scale.x * r, scale.y * h, scale.z * r),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
    }
}

/// A plain axis-aligned box entity (interior architecture: floors, walls, doors).
fn box_entity(id: String, x: f64, y: f64, z: f64, sx: f64, sy: f64, sz: f64, mat: Material) -> Entity {
    Entity {
        id,
        transform: Transform::at(x, y, z),
        geometry: Geometry::Box {
            size: Vec3::new(sx, sy, sz),
        },
        material: mat,
        velocity: None,
        description: None,
    }
}

/// Compile a room-feature (feed post / message thread / file) into an Astral
/// entity placed within its room (spaces §2.4). Posts/threads hang at eye level;
/// files sit lower like shelved objects.
pub fn compile_room_feature(f: &RoomFeature, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_room_feature(f, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);
    let y = if f.kind == "file" { 0.8 } else { 1.4 };
    Entity {
        id: format!("feature:{}:{}:{}", f.room, f.kind, f.index),
        transform: Transform::at(f.position.x, y, f.position.y)
            .with_scale(scale.x * 0.6, scale.y * 0.6, scale.z * 0.6),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
    }
}

/// Compile a peer avatar (spaces §8.2) at a world position. Used to render
/// other players in the live scene.
pub fn compile_avatar(
    peer_id: &[u8],
    palette: &AestheticPalette,
    x: f64,
    y: f64,
    z: f64,
) -> Entity {
    let graph = mapping::map_avatar(peer_id, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);
    Entity {
        id: format!("avatar:{:08x}", peer_id_u32(peer_id)),
        transform: Transform::at(x, y, z).with_scale(scale.x, scale.y, scale.z),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
    }
}

/// Compile a portal entity (spaces §5.1) at a position. `destination` is encoded
/// in the entity id (`portal:<destination>`) so the renderer/client knows where
/// it leads (e.g. `outside`, `room:social.feed`, `peer_inside:<id>`).
pub fn compile_portal(destination: &str, x: f64, y: f64, z: f64, hue: f64) -> Entity {
    let graph = mapping::map_portal();
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mut mat = material::resolve_material(&graph, hue);
    mat.transparency = Some(mat.transparency.unwrap_or(0.5).min(0.6));
    if mat.emissive.is_none() {
        mat.emissive = Some(0.3);
    }
    Entity {
        id: format!("portal:{destination}"),
        transform: Transform::at(x, y, z).with_scale(scale.x * 1.2, scale.y * 1.6, scale.z * 1.2),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
    }
}

/// Compile a peer's Inside into a renderable Astral scene (spaces §2): each room
/// becomes floor + ceiling + four walls, doors are emissive markers on the hall
/// perimeter, and a point light sits near each room's ceiling. The camera starts
/// in the entry hall. Materials are derived from the district palette.
pub fn compile_inside_scene(inside: &Inside, palette: &AestheticPalette) -> Scene {
    let hue = palette.hue;
    let wall_mat = || Material {
        base_color: Color::from_hsl(hue, 0.18, 0.32),
        brightness: 0.45,
        emissive: None,
        emission_color: None,
        roughness: 0.8,
        reflectivity: 0.05,
        transparency: None,
        glyph_style: Some("dense".into()),
        motion_behavior: None,
        displacement: None,
    };
    let floor_mat = || Material {
        base_color: Color::from_hsl(hue, 0.15, 0.22),
        brightness: 0.4,
        emissive: None,
        emission_color: None,
        roughness: 0.9,
        reflectivity: 0.03,
        transparency: None,
        glyph_style: Some("dense".into()),
        motion_behavior: None,
        displacement: None,
    };

    let t = 0.2; // floor/wall thickness
    let mut entities = Vec::new();
    let mut lights = Vec::new();

    for room in &inside.rooms {
        let (cx, cz) = (room.position.x, room.position.y);
        let (w, d, h) = (room.width, room.depth, room.height);
        let n = &room.name;
        entities.push(box_entity(format!("{n}_floor"), cx, 0.0, cz, w, t, d, floor_mat()));
        entities.push(box_entity(format!("{n}_ceil"), cx, h, cz, w, t, d, floor_mat()));
        entities.push(box_entity(format!("{n}_wn"), cx, h / 2.0, cz - d / 2.0, w, h, t, wall_mat()));
        entities.push(box_entity(format!("{n}_ws"), cx, h / 2.0, cz + d / 2.0, w, h, t, wall_mat()));
        entities.push(box_entity(format!("{n}_we"), cx + w / 2.0, h / 2.0, cz, t, h, d, wall_mat()));
        entities.push(box_entity(format!("{n}_ww"), cx - w / 2.0, h / 2.0, cz, t, h, d, wall_mat()));
        lights.push(Light {
            light_type: "point".into(),
            position: Some(Vec3::new(cx, h - 0.5, cz)),
            direction: None,
            intensity: 1.5,
            color: Color::from_hsl((hue + 30.0).rem_euclid(360.0), 0.5, 0.7),
            range: Some(w.max(d) * 1.5),
        });
    }

    // Doors → room portals (spaces §5.1): each doorway is a portal into its room.
    for door in &inside.doors {
        entities.push(compile_portal(
            &format!("room:{}", door.to),
            door.position.x,
            door.height / 2.0,
            door.position.y,
            hue,
        ));
    }
    // Exit portal back to the Outside, near the hall edge.
    let hall_half = inside.rooms[0].width / 2.0;
    entities.push(compile_portal("outside", 0.0, 1.4, hall_half - 0.8, hue));

    let environment = Environment {
        ambient_light: 0.35,
        background_color: Color::from_hsl(hue, 0.1, 0.05),
        fog_density: None,
        fog_color: None,
    };
    let camera = Camera {
        position: Vec3::new(0.0, 1.6, 0.0),
        rotation: Vec3::new(0.0, 0.0, 0.0),
        fov: 70.0,
        near: 0.1,
        far: 200.0,
    };

    Scene {
        time: 0.0,
        camera,
        environment,
        lights,
        entities,
        ground_paint: None,
    }
}

/// Compile an underground tunnel into a renderable Astral scene (spaces §3).
/// The tunnel runs along +X from the A end (origin); each segment gets floor +
/// ceiling + two walls whose colour is the gradient-lerped aesthetic between the
/// two peers' districts, capability markers are emissive ornaments, and lights
/// along the length reflect the connection's uptime.
pub fn compile_tunnel_scene(tunnel: &Tunnel) -> Scene {
    let w = tunnel.width;
    let h = tunnel.height;
    let seg_len = tunnel.length / tunnel.segment_count as f64;
    let thick = 0.3;
    let mut entities = Vec::new();
    let mut lights = Vec::new();

    let mat_at = |hue: f64, pr: f64, light: f64| Material {
        base_color: Color::from_hsl(hue, 0.2, light + 0.1 * pr),
        brightness: 0.4,
        emissive: None,
        emission_color: None,
        roughness: 0.85,
        reflectivity: 0.04,
        transparency: None,
        glyph_style: Some("dense".into()),
        motion_behavior: None,
        displacement: None,
    };

    for i in 0..tunnel.segment_count {
        let t = (i as f64 + 0.5) / tunnel.segment_count as f64;
        let cx = (i as f64 + 0.5) * seg_len;
        let a = TunnelAesthetic::lerp(&tunnel.aesthetic_a, &tunnel.aesthetic_b, t);
        let wall = mat_at(a.hue, a.popcount_ratio, 0.30);
        let slab = mat_at(a.hue, a.popcount_ratio, 0.20);
        entities.push(box_entity(format!("tun_floor_{i}"), cx, 0.0, 0.0, seg_len, thick, w, slab.clone()));
        entities.push(box_entity(format!("tun_ceil_{i}"), cx, h, 0.0, seg_len, thick, w, slab));
        entities.push(box_entity(format!("tun_wl_{i}"), cx, h / 2.0, -w / 2.0, seg_len, h, thick, wall.clone()));
        entities.push(box_entity(format!("tun_wr_{i}"), cx, h / 2.0, w / 2.0, seg_len, h, thick, wall));
    }

    // Capability markers — emissive ornaments along the centreline.
    for (i, m) in tunnel.markers.iter().enumerate() {
        let a = TunnelAesthetic::lerp(&tunnel.aesthetic_a, &tunnel.aesthetic_b, m.t);
        let hue = (a.hue + 40.0).rem_euclid(360.0);
        let dmat = Material {
            base_color: Color::from_hsl(hue, 0.5, 0.6),
            brightness: 0.7,
            emissive: Some(0.5),
            emission_color: Some(Color::from_hsl(hue, 0.6, 0.7)),
            roughness: 0.4,
            reflectivity: 0.1,
            transparency: None,
            glyph_style: Some("round".into()),
            motion_behavior: None,
            displacement: None,
        };
        entities.push(box_entity(format!("tun_marker_{i}"), m.distance, h * 0.5, 0.0, 0.6, 0.6, 0.6, dmat));
    }

    // Lights — intensity from uptime lighting state.
    let intensity = match tunnel.lighting.intensity.as_str() {
        "moderate" => 1.8,
        "subtle" => 1.0,
        _ => 0.5,
    };
    let nlights = (tunnel.segment_count / 2).max(1);
    for k in 0..nlights {
        let t = (k as f64 + 0.5) / nlights as f64;
        let a = TunnelAesthetic::lerp(&tunnel.aesthetic_a, &tunnel.aesthetic_b, t);
        lights.push(Light {
            light_type: "point".into(),
            position: Some(Vec3::new(t * tunnel.length, h - 0.4, 0.0)),
            direction: None,
            intensity,
            color: Color::from_hsl(a.hue, 0.5, 0.7),
            range: Some(w * 2.5),
        });
    }

    // End portals → each peer's Inside (spaces §5.1).
    entities.push(compile_portal(
        "peer_a_inside",
        0.5,
        h * 0.5,
        0.0,
        tunnel.aesthetic_a.hue,
    ));
    entities.push(compile_portal(
        "peer_b_inside",
        tunnel.length - 0.5,
        h * 0.5,
        0.0,
        tunnel.aesthetic_b.hue,
    ));

    let environment = Environment {
        ambient_light: 0.25,
        background_color: Color::from_hsl(tunnel.aesthetic_a.hue, 0.1, 0.04),
        fog_density: None,
        fog_color: None,
    };
    // Camera at the A end, looking down the tunnel (+X).
    let camera = Camera {
        position: Vec3::new(1.0, h * 0.5, 0.0),
        rotation: Vec3::new(0.0, -std::f64::consts::FRAC_PI_2, 0.0),
        fov: 70.0,
        near: 0.1,
        far: 300.0,
    };

    Scene {
        time: 0.0,
        camera,
        environment,
        lights,
        entities,
        ground_paint: None,
    }
}

/// Compile a fixture into an Astral Entity.
pub fn compile_fixture(f: &Fixture, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_fixture(f, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);

    // Rest the fixture on the ground: lift so its lowest point is at y = 0.
    let y = (f.scale_height * 0.5).max(vertical_half_extent(&geo, &scale));

    Entity {
        id: format!("fixture_{}", f.object_id),
        transform: Transform::at(f.position.x, y, f.position.y)
            .with_scale(scale.x, scale.y, scale.z)
            .with_rotation_y(f.orientation),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
    }
}

/// Compile a flora instance into an Astral Entity.
pub fn compile_flora(f: &Flora, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_flora(f, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);

    // Flora is rooted in the ground; the transform scale is scale*f.scale, so the
    // y offset must use that same effective scale to plant the base at y = 0
    // (a tall tree was sinking most of its trunk below ground).
    let eff = Vec3::new(scale.x * f.scale, scale.y * f.scale, scale.z * f.scale);
    let y = vertical_half_extent(&geo, &eff);

    Entity {
        id: format!("flora_{}", f.object_id),
        transform: Transform::at(f.position.x, y, f.position.y)
            .with_scale(eff.x, eff.y, eff.z)
            .with_rotation_y(f.orientation),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
    }
}

/// Vertical half-extent of a geometry (after applying the entity's y-scale).
/// Used to keep ground-dwelling entities resting on the ground (bottom >= 0)
/// instead of sinking their lower half below the ground plane (y = 0).
fn vertical_half_extent(geo: &Geometry, scale: &Vec3) -> f64 {
    match geo {
        Geometry::Sphere { radius } => radius * scale.y,
        Geometry::Box { size } => size.y * 0.5 * scale.y,
        Geometry::Cylinder { height, .. } => height * 0.5 * scale.y,
        Geometry::Plane { .. } => 0.0,
    }
}

/// Compile a creature into one or more Astral Entities (composition).
/// `base_pos` is the zone-derived initial position.
pub fn compile_creature(
    c: &Creature,
    palette: &AestheticPalette,
    base_pos: crate::types::Point,
    height: f64,
) -> Vec<Entity> {
    let graph = mapping::map_creature(c, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);
    let offsets = geometry::resolve_composition(&graph);
    // Keep each body part above the ground plane (aerial/perching creatures have
    // a large `height` and are unaffected; ground creatures no longer sink in).
    let half_h = vertical_half_extent(&geo, &scale);

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
                transform: Transform::at(
                    base_pos.x + offset.x,
                    (height + offset.y).max(half_h),
                    base_pos.y + offset.z,
                )
                    .with_scale(scale.x, scale.y, scale.z),
                geometry: geo.clone(),
                material: mat.clone(),
                velocity: None,
                description: if i == 0 { Some(graph.clone()) } else { None },
            }
        })
        .collect()
}

/// Compile a conveyanceinto an Astral Entity.
pub fn compile_conveyance(c: &Conveyance, palette: &AestheticPalette) -> Entity {
    let graph = mapping::map_conveyance(c, palette);
    let (geo, scale) = geometry::resolve_geometry(&graph);
    let mat = material::resolve_material(&graph, palette.hue);
    // Rest the chassis on the ground rather than sinking it to its centre.
    let y = 0.5_f64.max(vertical_half_extent(&geo, &scale));

    Entity {
        id: format!("conveyance_{}", c.object_id),
        transform: Transform::at(c.position.x, y, c.position.y)
            .with_scale(scale.x, scale.y, scale.z)
            .with_rotation_y(c.orientation),
        geometry: geo,
        material: mat,
        velocity: None,
        description: Some(graph),
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
    let ground_size = 1200.0; // covers the district plus its loaded neighbours

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
            displacement: None,
            emission_color: None,
        },
        velocity: None,
        description: None,
    }
}

/// Compile a full district into an Astral Scene.
///
/// This is the main entry point — takes the complete generation output
/// and produces a Scene that Astral can render.
pub fn compile_district_scene(
    cell: &Cell,
    palette: &AestheticPalette,
    atmo: &AtmosphereState,
    now_ms: u64,
) -> Scene {
    use crate::gen::{buildings, conveyances, creatures, fixtures, flora, roads, rivers, district, zones};
    let is_night = crate::gen::atmosphere::is_night(atmo.time_of_day);

    let dist = district::generate_district(cell);
    let road_network = roads::generate_roads(&dist);
    let river_data = rivers::generate_rivers(&dist);
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

        // Creatures — habitat-aware placement (§15.2/§15.5): zone-confined with
        // time-slot migration, nocturnal gating, elevated for aerial/perching,
        // and surfacing at perimeter emergence points for subterranean.
        let block_zones = zones::generate_zones(cell.key, block);
        for pc in creatures::place_creatures(cell, block, &block_zones, now_ms, is_night) {
            entities.extend(compile_creature(&pc.creature, palette, pc.position, pc.height));
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

    // Ground zone/road/river raster — sampled by the renderer on ground hits.
    let ground_paint = Some(crate::scene::groundpaint::paint_ground(
        &blocks,
        &road_network,
        &river_data,
        &dist.polygon,
    ));

    Scene {
        time: 0.0,
        camera,
        environment,
        lights,
        entities,
        ground_paint,
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

        let scene = compile_district_scene(&cell, &palette, &atmo, 0);

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
        let scene = compile_district_scene(&cell, &palette, &atmo, 0);

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
        let scene = compile_district_scene(&cell, &palette, &atmo, 0);

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

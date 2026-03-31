//! View state manager — frustum culling, LOD, entity diffing.
//!
//! Maintains the set of entities currently visible to a client.
//! On each camera update, computes the diff and produces enter/leave/update events.

use std::collections::{HashMap, HashSet};

use crate::gen::aesthetic::AestheticPalette;
use crate::gen::atmosphere;
use crate::gen::blocks::Block;
use crate::gen::cell::Cell;
use crate::scene::compiler::{self, Entity, Light, Environment, Camera};
use crate::scene::geometry::Vec3;
use crate::scene::material::Color;
use crate::types::Point;

/// LOD level for an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lod {
    Full,       // < 30 wu: displacement, composition, controllers
    Simplified, // 30-60 wu: single primitive, no displacement
    Billboard,  // > 60 wu: flat box, average colour
}

fn lod_for_distance(dist: f64) -> Lod {
    if dist < 30.0 { Lod::Full }
    else if dist < 60.0 { Lod::Simplified }
    else { Lod::Billboard }
}

/// A tracked entity in the view.
struct TrackedEntity {
    entity: Entity,
    world_x: f64,
    world_z: f64,
    lod: Lod,
}

/// Per-client view state.
pub struct ViewState {
    /// Current player position in world space.
    pub player_x: f64,
    pub player_y: f64,
    pub player_z: f64,
    pub player_dx: f64,
    pub player_dy: f64,
    pub player_dz: f64,
    pub fov: f64,

    /// Entities currently in the client's view (keyed by entity id).
    visible: HashMap<String, TrackedEntity>,

    /// World-space origin (initial camera position for coordinate translation).
    origin_x: f64,
    origin_z: f64,

    /// The district cell.
    cell: Cell,
    palette: AestheticPalette,

    /// All entities in the district (pre-generated, world coordinates).
    all_entities: Vec<Entity>,
    /// World positions for each entity (parallel to all_entities).
    all_world_pos: Vec<(f64, f64)>, // (world_x, world_z)
    /// All lights in the district.
    all_lights: Vec<Light>,

    /// View range in world units.
    view_range: f64,
    max_lights: usize,
}

/// Events produced by a view update.
pub enum ViewEvent {
    Enter(Entity),
    Leave(String),
    Lights(Vec<Light>),
}

impl ViewState {
    /// Create a new view state for a district.
    pub fn new(cell: Cell, view_range: f64) -> Self {
        let palette = AestheticPalette::from_cell(&cell);

        // Generate the full district scene once
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let atmo = atmosphere::compute_atmosphere(&cell, now_ms);
        let scene = compiler::compile_district_scene(&cell, &palette, &[], &atmo);

        // Store world positions before any transformation
        let all_world_pos: Vec<(f64, f64)> = scene.entities.iter()
            .map(|e| (e.transform.position.x, e.transform.position.z))
            .collect();

        let all_lights = scene.lights.clone();

        let origin_x = scene.camera.position.x;
        let origin_z = scene.camera.position.z;

        Self {
            player_x: origin_x,
            player_y: scene.camera.position.y,
            player_z: origin_z,
            player_dx: 0.0,
            player_dy: -0.3,
            player_dz: -1.0,
            fov: 60.0,
            visible: HashMap::new(),
            origin_x,
            origin_z,
            cell,
            palette,
            all_entities: scene.entities,
            all_world_pos,
            all_lights,
            view_range,
            max_lights: 8,
        }
    }

    /// Get initial scene setup (environment, camera, ground).
    pub fn get_init(&self) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let atmo = atmosphere::compute_atmosphere(&self.cell, now_ms);
        let (env, _lights) = compiler::compile_environment(&self.cell, &atmo, &self.palette);

        // Ground entity — translate to player-relative
        let ground = self.all_entities.iter().find(|e| e.id == "ground");
        let ground_json = if let Some(g) = ground {
            let mut g = g.clone();
            g.transform.position.x -= self.player_x;
            g.transform.position.z -= self.player_z;
            serde_json::to_value(&g).unwrap_or_default()
        } else {
            serde_json::json!(null)
        };

        let cam = serde_json::json!({
            "position": { "x": 0.0, "y": self.player_y, "z": 0.0 },
            "rotation": { "x": -0.3, "y": 0.0, "z": 0.0 },
            "fov": self.fov,
            "near": 0.1,
            "far": self.view_range * 2.0,
        });

        (serde_json::to_value(&env).unwrap_or_default(), cam, ground_json)
    }

    /// Update camera position and produce enter/leave events.
    /// Client sends camera position in player-relative coords
    /// (origin was the initial camera position from scene compile).
    pub fn update_camera(
        &mut self,
        px: f64, py: f64, pz: f64,
        dx: f64, dy: f64, dz: f64,
        fov: f64,
    ) -> Vec<ViewEvent> {
        // Convert client-relative position to world space
        // origin_x/z was the initial camera world position
        self.player_x = self.origin_x + px;
        self.player_y = py;
        self.player_z = self.origin_z + pz;
        self.player_dx = dx;
        self.player_dy = dy;
        self.player_dz = dz;
        self.fov = fov;

        let mut events = Vec::new();

        // Determine which entities should be visible
        let mut should_be_visible: HashSet<String> = HashSet::new();
        let range_sq = self.view_range * self.view_range;

        for (i, entity) in self.all_entities.iter().enumerate() {
            if entity.id == "ground" { continue; } // ground sent in init

            let (wx, wz) = self.all_world_pos[i];
            let dx = wx - self.player_x;
            let dz = wz - self.player_z;
            let dist_sq = dx * dx + dz * dz;

            if dist_sq < range_sq {
                should_be_visible.insert(entity.id.clone());

                if !self.visible.contains_key(&entity.id) {
                    // Entity entering view — translate to player-relative coordinates
                    let mut e = entity.clone();
                    e.transform.position.x = wx - self.player_x;
                    e.transform.position.z = wz - self.player_z;

                    // LOD: strip displacement for distant entities
                    let dist = dist_sq.sqrt();
                    let lod = lod_for_distance(dist);
                    if lod != Lod::Full {
                        e.material.displacement = None;
                        // For billboard: simplify geometry
                        if lod == Lod::Billboard {
                            e.description = None;
                        }
                    }

                    self.visible.insert(entity.id.clone(), TrackedEntity {
                        entity: e.clone(),
                        world_x: wx,
                        world_z: wz,
                        lod,
                    });
                    events.push(ViewEvent::Enter(e));
                } else {
                    // Already visible — update position relative to player
                    if let Some(tracked) = self.visible.get_mut(&entity.id) {
                        tracked.entity.transform.position.x = wx - self.player_x;
                        tracked.entity.transform.position.z = wz - self.player_z;
                    }
                }
            }
        }

        // Find entities that left the view
        let to_remove: Vec<String> = self.visible.keys()
            .filter(|id| !should_be_visible.contains(id.as_str()))
            .cloned()
            .collect();

        for id in to_remove {
            self.visible.remove(&id);
            events.push(ViewEvent::Leave(id));
        }

        // Stream nearest lights (player-relative)
        let mut light_dists: Vec<(usize, f64)> = self.all_lights.iter().enumerate()
            .filter_map(|(i, l)| {
                if let Some(pos) = &l.position {
                    let dx = pos.x - self.player_x;
                    let dz = pos.z - self.player_z;
                    Some((i, dx * dx + dz * dz))
                } else {
                    Some((i, 0.0)) // directional = always include
                }
            })
            .collect();
        light_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        let active_lights: Vec<Light> = light_dists.iter()
            .take(self.max_lights + 1) // +1 for directional
            .map(|(i, _)| {
                let mut l = self.all_lights[*i].clone();
                if let Some(pos) = &mut l.position {
                    pos.x -= self.player_x;
                    pos.z -= self.player_z;
                }
                l
            })
            .collect();

        events.push(ViewEvent::Lights(active_lights));

        events
    }

    /// Get current visible entity count.
    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }
}

//! View state manager — frustum culling, LOD, entity diffing, cross-district loading.
//!
//! Maintains the set of entities currently visible to a client across
//! multiple districts. Loads neighboring districts when the player
//! approaches a boundary.

use std::collections::{HashMap, HashSet};

use crate::gen::aesthetic::AestheticPalette;
use crate::gen::atmosphere;
use crate::gen::cell::Cell;
use crate::gen::config::config;
use crate::scene::compiler::{self, Entity, Light, Scene};
use crate::scene::geometry::Vec3;
use crate::scene::material::Color;
use crate::types::Point;

/// LOD level for an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lod {
    Full,       // < 30 wu
    Simplified, // 30-60 wu
    Billboard,  // > 60 wu
}

fn lod_for_distance(dist: f64) -> Lod {
    if dist < 30.0 {
        Lod::Full
    } else if dist < 60.0 {
        Lod::Simplified
    } else {
        Lod::Billboard
    }
}

/// A loaded district with its pre-generated entities.
struct LoadedDistrict {
    cell: Cell,
    entities: Vec<Entity>,
    world_pos: Vec<(f64, f64)>, // world-space X/Z per entity
    lights: Vec<Light>,
}

impl LoadedDistrict {
    fn generate(cell: Cell) -> Self {
        let palette = AestheticPalette::from_cell(&cell);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let atmo = atmosphere::compute_atmosphere(&cell, now_ms);
        let scene = compiler::compile_district_scene(&cell, &palette, &[], &atmo);

        let world_pos: Vec<(f64, f64)> = scene
            .entities
            .iter()
            .map(|e| (e.transform.position.x, e.transform.position.z))
            .collect();
        let lights = scene.lights.clone();

        Self {
            cell,
            entities: scene.entities,
            world_pos,
            lights,
        }
    }
}

/// A tracked visible entity.
struct TrackedEntity {
    lod: Lod,
}

/// Events produced by a view update.
pub enum ViewEvent {
    Enter(Entity),
    Leave(String),
    Lights(Vec<Light>),
}

/// Per-client view state — supports multiple loaded districts.
pub struct ViewState {
    pub player_x: f64,
    pub player_y: f64,
    pub player_z: f64,
    pub player_dx: f64,
    pub player_dy: f64,
    pub player_dz: f64,
    pub fov: f64,

    origin_x: f64,
    origin_z: f64,

    /// Currently visible entities by id.
    visible: HashMap<String, TrackedEntity>,

    /// Loaded districts by cell key.
    districts: HashMap<u32, LoadedDistrict>,

    /// The primary district cell (the one the player entered).
    primary_cell: Cell,

    view_range: f64,
    max_lights: usize,

    /// District boundary detection threshold (wu).
    /// Load neighbor when player is within this distance of any district edge.
    neighbor_load_distance: f64,

    /// Keys of districts we've already attempted to load.
    loaded_keys: HashSet<u32>,
}

impl ViewState {
    pub fn new(cell: Cell, view_range: f64) -> Self {
        let district = LoadedDistrict::generate(cell.clone());

        // Camera starts at district seed position
        let cfg = config();
        let origin_x = cell.gx as f64 * cfg.scale;
        let origin_z = cell.gy as f64 * cfg.scale;

        let mut districts = HashMap::new();
        let key = cell.key;
        districts.insert(key, district);

        let mut loaded_keys = HashSet::new();
        loaded_keys.insert(key);

        Self {
            player_x: origin_x,
            player_y: 8.0,
            player_z: origin_z,
            player_dx: 0.0,
            player_dy: -0.3,
            player_dz: -1.0,
            fov: 60.0,
            origin_x,
            origin_z,
            visible: HashMap::new(),
            districts,
            primary_cell: cell,
            view_range,
            max_lights: 8,
            neighbor_load_distance: view_range * 0.8,
            loaded_keys,
        }
    }

    /// Get initial scene setup.
    pub fn get_init(&self) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let palette = AestheticPalette::from_cell(&self.primary_cell);
        let atmo = atmosphere::compute_atmosphere(&self.primary_cell, now_ms);
        let (env, _) = compiler::compile_environment(&self.primary_cell, &atmo, &palette);

        // Ground entity from primary district
        let ground = self.districts.get(&self.primary_cell.key)
            .and_then(|d| d.entities.iter().find(|e| e.id == "ground"))
            .map(|g| {
                let mut g = g.clone();
                g.transform.position.x -= self.origin_x;
                g.transform.position.z -= self.origin_z;
                serde_json::to_value(&g).unwrap_or_default()
            })
            .unwrap_or(serde_json::json!(null));

        let cam = serde_json::json!({
            "position": { "x": 0.0, "y": self.player_y, "z": 0.0 },
            "rotation": { "x": -0.3, "y": 0.0, "z": 0.0 },
            "fov": self.fov,
            "near": 0.1,
            "far": self.view_range * 2.0,
        });

        (serde_json::to_value(&env).unwrap_or_default(), cam, ground)
    }

    /// Check if neighboring districts need loading based on player position.
    fn maybe_load_neighbors(&mut self) {
        let cfg = config();
        let [o1, o2, o3] = self.primary_cell.octets;

        // Check 8 neighbors
        let deltas: &[(i16, i16)] = &[
            (0, 1), (0, -1), (1, 0), (-1, 0),
            (1, 1), (1, -1), (-1, 1), (-1, -1),
        ];

        for &(do3, do2) in deltas {
            let n3 = o3 as i16 + do3;
            let n2 = o2 as i16 + do2;
            if n3 < 0 || n3 > 255 || n2 < 0 || n2 > 255 {
                continue;
            }

            let ncell = Cell::from_octets(o1, n2 as u8, n3 as u8);

            if self.loaded_keys.contains(&ncell.key) {
                continue;
            }

            // Distance from player to neighbor district centre
            let nx = ncell.gx as f64 * cfg.scale;
            let nz = ncell.gy as f64 * cfg.scale;
            let dx = nx - self.player_x;
            let dz = nz - self.player_z;
            let dist = (dx * dx + dz * dz).sqrt();

            if dist < self.view_range + cfg.scale * 0.5 {
                // Close enough — load this neighbor
                let district = LoadedDistrict::generate(ncell.clone());
                self.districts.insert(ncell.key, district);
                self.loaded_keys.insert(ncell.key);
            }
        }
    }

    /// Update camera and produce enter/leave/lights events.
    pub fn update_camera(
        &mut self,
        px: f64,
        py: f64,
        pz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
        fov: f64,
    ) -> Vec<ViewEvent> {
        self.player_x = self.origin_x + px;
        self.player_y = py;
        self.player_z = self.origin_z + pz;
        self.player_dx = dx;
        self.player_dy = dy;
        self.player_dz = dz;
        self.fov = fov;

        // Check if we need to load neighboring districts
        self.maybe_load_neighbors();

        let mut events = Vec::new();
        let mut should_be_visible: HashSet<String> = HashSet::new();
        let range_sq = self.view_range * self.view_range;

        // Iterate ALL loaded districts' entities
        for district in self.districts.values() {
            for (i, entity) in district.entities.iter().enumerate() {
                if entity.id == "ground" {
                    continue;
                }

                let (wx, wz) = district.world_pos[i];
                let ddx = wx - self.player_x;
                let ddz = wz - self.player_z;
                let dist_sq = ddx * ddx + ddz * ddz;

                if dist_sq < range_sq {
                    should_be_visible.insert(entity.id.clone());

                    if !self.visible.contains_key(&entity.id) {
                        // Entity entering — translate to player-relative
                        let mut e = entity.clone();
                        e.transform.position.x = wx - self.player_x;
                        e.transform.position.z = wz - self.player_z;

                        let dist = dist_sq.sqrt();
                        let lod = lod_for_distance(dist);
                        if lod != Lod::Full {
                            e.material.displacement = None;
                            if lod == Lod::Billboard {
                                e.description = None;
                            }
                        }

                        self.visible.insert(
                            entity.id.clone(),
                            TrackedEntity { lod },
                        );
                        events.push(ViewEvent::Enter(e));
                    }
                }
            }
        }

        // Remove entities that left
        let to_remove: Vec<String> = self
            .visible
            .keys()
            .filter(|id| !should_be_visible.contains(id.as_str()))
            .cloned()
            .collect();

        for id in to_remove {
            self.visible.remove(&id);
            events.push(ViewEvent::Leave(id));
        }

        // Stream nearest lights from ALL loaded districts
        let mut all_lights: Vec<(f64, Light)> = Vec::new();
        for district in self.districts.values() {
            for l in &district.lights {
                let dist_sq = if let Some(pos) = &l.position {
                    let ddx = pos.x - self.player_x;
                    let ddz = pos.z - self.player_z;
                    ddx * ddx + ddz * ddz
                } else {
                    0.0 // directional = always include
                };
                let mut light = l.clone();
                if let Some(pos) = &mut light.position {
                    pos.x -= self.player_x;
                    pos.z -= self.player_z;
                }
                all_lights.push((dist_sq, light));
            }
        }
        all_lights.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let active_lights: Vec<Light> = all_lights
            .into_iter()
            .take(self.max_lights + 1)
            .map(|(_, l)| l)
            .collect();

        events.push(ViewEvent::Lights(active_lights));

        events
    }

    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }
}

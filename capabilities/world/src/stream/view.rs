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
use crate::scene::compiler::{self, Entity, Light};

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
    paint: Option<crate::scene::groundpaint::GroundPaint>,
}

impl LoadedDistrict {
    fn generate(cell: Cell) -> Self {
        let palette = AestheticPalette::from_cell(&cell);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let atmo = atmosphere::compute_atmosphere(&cell, now_ms);
        let scene = compiler::compile_district_scene(&cell, &palette, &atmo, now_ms);

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
            paint: scene.ground_paint,
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
    GroundPaint(serde_json::Value),
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

    /// Cell key whose ground paint was last sent to the client.
    paint_sent_key: u32,

    /// Keys of districts currently loaded (kept in sync with `districts`).
    loaded_keys: HashSet<u32>,
}

impl ViewState {
    pub fn new(cell: Cell, view_range: f64) -> Self {
        let district = LoadedDistrict::generate(cell.clone());

        // Origin = camera position from scene compilation (seed_position + offset)
        // Must match what the client sees after HowmSceneProvider.recentreToOrigin()
        let cam_x = district.entities.iter()
            .find(|e| e.id == "ground")
            .map(|g| g.transform.position.x)
            .unwrap_or(cell.gx as f64 * config().scale);
        let cam_z = district.entities.iter()
            .find(|e| e.id == "ground")
            .map(|g| g.transform.position.z)
            .unwrap_or(cell.gy as f64 * config().scale);
        let origin_x = cam_x;
        let origin_z = cam_z;

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
            max_lights: 24,
            paint_sent_key: key,
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

        // Grounds stream per-district (namespaced) in the entity loop so the floor
        // tiles across the loaded window; nothing to send up-front here.
        let ground = serde_json::json!(null);

        let cam = serde_json::json!({
            "position": { "x": 0.0, "y": self.player_y, "z": 0.0 },
            "rotation": { "x": -0.3, "y": 0.0, "z": 0.0 },
            "fov": self.fov,
            "near": 0.1,
            "far": self.view_range * 2.0,
        });

        (serde_json::to_value(&env).unwrap_or_default(), cam, ground)
    }

    /// Ground paint for a district, shifted into shared-origin client space.
    fn paint_for(&self, cell_key: u32) -> serde_json::Value {
        self.districts
            .get(&cell_key)
            .and_then(|d| d.paint.as_ref())
            .map(|p| {
                let mut p = p.clone();
                p.ox -= self.origin_x;
                p.oz -= self.origin_z;
                serde_json::to_value(&p).unwrap_or(serde_json::Value::Null)
            })
            .unwrap_or(serde_json::Value::Null)
    }

    /// Ground paint for the district the player currently stands in.
    pub fn center_paint_json(&self) -> serde_json::Value {
        self.paint_for(self.current_cell().key)
    }

    /// The district the player currently stands in (nearest seed — Voronoi cell).
    fn current_cell(&self) -> Cell {
        let cfg = config();
        let mut best = self.primary_cell.clone();
        let mut best_d = f64::MAX;
        for district in self.districts.values() {
            let cx = district.cell.gx as f64 * cfg.scale;
            let cz = district.cell.gy as f64 * cfg.scale;
            let dx = cx - self.player_x;
            let dz = cz - self.player_z;
            let d = dx * dx + dz * dz;
            if d < best_d {
                best_d = d;
                best = district.cell.clone();
            }
        }
        best
    }

    /// Keep the 3x3 ring around the player's current cell loaded, and drop
    /// districts that have fallen outside the window — a moving window that
    /// follows the camera so crossing any boundary reveals the neighbour.
    fn maybe_load_neighbors(&mut self) {
        let cfg = config();
        let center = self.current_cell();

        // Load the ring around the current cell (neighbor_key wraps correctly,
        // including across the octet2/octet1 boundary).
        for dx in -1..=1 {
            for dy in -1..=1 {
                let ncell = Cell::from_key(center.neighbor_key(dx, dy));
                if self.loaded_keys.contains(&ncell.key) {
                    continue;
                }
                let nx = ncell.gx as f64 * cfg.scale;
                let nz = ncell.gy as f64 * cfg.scale;
                let ddx = nx - self.player_x;
                let ddz = nz - self.player_z;
                if (ddx * ddx + ddz * ddz).sqrt() < self.view_range + cfg.scale * 1.5 {
                    let district = LoadedDistrict::generate(ncell.clone());
                    self.districts.insert(ncell.key, district);
                    self.loaded_keys.insert(ncell.key);
                }
            }
        }

        // Prune districts more than 2 cells from the current centre.
        let to_drop: Vec<u32> = self
            .districts
            .values()
            .filter(|d| {
                let ddx = (d.cell.gx as i32 - center.gx as i32).abs();
                let ddy = (d.cell.gy as i32 - center.gy as i32).abs();
                ddx.max(ddy) > 2
            })
            .map(|d| d.cell.key)
            .collect();
        for k in to_drop {
            self.districts.remove(&k);
            self.loaded_keys.remove(&k);
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
        // The ground box is large and overlapping, so only the district the
        // player stands in contributes one — keeps the always-evaluated global
        // candidate count to one box.
        let center_key = self.current_cell().key;

        // When the player crosses into a different district, repaint the ground.
        if center_key != self.paint_sent_key {
            let pj = self.paint_for(center_key);
            if !pj.is_null() {
                events.push(ViewEvent::GroundPaint(pj));
                self.paint_sent_key = center_key;
            }
        }

        // Iterate ALL loaded districts' entities. Ids are namespaced per district
        // so grounds (and any same-named entities) from different districts don't
        // collide in the client's entity map.
        for district in self.districts.values() {
            let key = district.cell.key;
            for (i, entity) in district.entities.iter().enumerate() {
                let is_ground = entity.id == "ground";
                if is_ground && key != center_key {
                    continue;
                }
                let (wx, wz) = district.world_pos[i];
                let ddx = wx - self.player_x;
                let ddz = wz - self.player_z;
                let dist_sq = ddx * ddx + ddz * ddz;

                // Ground is large and always relevant for a loaded district;
                // everything else is range-culled.
                if is_ground || dist_sq < range_sq {
                    let nid = format!("{}#{}", key, entity.id);
                    should_be_visible.insert(nid.clone());

                    if !self.visible.contains_key(&nid) {
                        // Entering — anchor to the shared origin (world-anchored),
                        // NOT player-relative, so it stays put as the camera moves.
                        let mut e = entity.clone();
                        e.id = nid.clone();
                        e.transform.position.x = wx - self.origin_x;
                        e.transform.position.z = wz - self.origin_z;

                        let dist = dist_sq.sqrt();
                        let lod = if is_ground { Lod::Full } else { lod_for_distance(dist) };
                        if lod != Lod::Full {
                            e.material.displacement = None;
                            if lod == Lod::Billboard {
                                e.description = None;
                            }
                        }

                        self.visible.insert(nid, TrackedEntity { lod });
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
                    // Origin-relative, matching the world-anchored entities.
                    pos.x -= self.origin_x;
                    pos.z -= self.origin_z;
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

    pub fn loaded_count(&self) -> usize {
        self.districts.len()
    }

    /// Determine which district the player is currently standing in.
    pub fn current_district_ip(&self) -> String {
        let cfg = config();
        // Find the district whose centre is nearest to the player
        let mut best_key = self.primary_cell.key;
        let mut best_dist = f64::MAX;
        for (key, district) in &self.districts {
            let cx = district.cell.gx as f64 * cfg.scale;
            let cz = district.cell.gy as f64 * cfg.scale;
            let dx = cx - self.player_x;
            let dz = cz - self.player_z;
            let d = dx * dx + dz * dz;
            if d < best_dist {
                best_dist = d;
                best_key = *key;
            }
        }
        self.districts.get(&best_key)
            .map(|d| d.cell.ip_prefix())
            .unwrap_or_else(|| self.primary_cell.ip_prefix())
    }

    /// The current district's seed in shared-origin space — the presence anchor
    /// for this space. Peer poses are sent relative to it so they align.
    pub fn current_anchor(&self) -> [f64; 2] {
        let s = crate::gen::district::seed_position(&self.current_cell());
        [s.x - self.origin_x, s.y - self.origin_z]
    }
}

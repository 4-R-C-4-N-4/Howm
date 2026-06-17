//! `DistrictData` — the deterministic structural layer of a chunk, generated
//! once and cached.
//!
//! `district → roads → rivers → blocks` is time-independent and the expensive
//! part of generation (Voronoi tessellation + PSLG face extraction). It used to
//! be re-run independently by the scene compiler, the ASCII/SVG map, the audit
//! and the stream view — which is drift-prone: a change in one place (e.g. the
//! river corridor) silently disagreed with the others. Now every consumer reads
//! the same cached `DistrictData`, so they cannot diverge, and repeated access
//! to the same district (the stream view's moving window, multiple endpoints for
//! one IP) is free.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::blocks::{extract_blocks, Block};
use super::cell::Cell;
use super::district::{generate_district, DistrictGeometry};
use super::rivers::{generate_rivers, RiverSegment};
use super::roads::{generate_roads, RoadNetwork};

/// The deterministic structural layer of one district: geometry, road network,
/// rivers and blocks. Everything time-dependent (atmosphere, creatures, moving
/// conveyances) stays out of here so this can be cached.
pub struct DistrictData {
    pub geometry: DistrictGeometry,
    pub roads: RoadNetwork,
    pub rivers: Vec<RiverSegment>,
    pub blocks: Vec<Block>,
}

impl DistrictData {
    /// Run the structural pipeline for a cell. Prefer [`district_data`], which
    /// caches; use this only when you explicitly want a fresh build.
    pub fn generate(cell: &Cell) -> Self {
        let geometry = generate_district(cell);
        let roads = generate_roads(&geometry);
        let rivers = generate_rivers(&geometry);
        let blocks = extract_blocks(cell, &geometry.polygon, &roads, &rivers);
        Self {
            geometry,
            roads,
            rivers,
            blocks,
        }
    }
}

/// Soft cap on cached districts. Generation is deterministic, so evicting and
/// regenerating is always safe — this just bounds memory.
const CACHE_CAP: usize = 512;

fn cache() -> &'static Mutex<HashMap<u32, Arc<DistrictData>>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, Arc<DistrictData>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cached structural data for a district, generated on first access. Keyed by
/// `cell.key`, so the same `/24` always returns the identical instance.
pub fn district_data(cell: &Cell) -> Arc<DistrictData> {
    if let Some(dd) = cache().lock().unwrap().get(&cell.key) {
        return dd.clone();
    }
    // Generate outside the lock so concurrent requests for different districts
    // don't serialise on the expensive pipeline.
    let dd = Arc::new(DistrictData::generate(cell));
    let mut map = cache().lock().unwrap();
    if map.len() >= CACHE_CAP {
        map.clear();
    }
    // A peer thread may have inserted the same key meanwhile; both are identical.
    map.entry(cell.key).or_insert_with(|| dd.clone());
    dd
}

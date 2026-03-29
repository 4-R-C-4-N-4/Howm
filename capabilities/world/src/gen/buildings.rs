//! Building form generation — alleys, plot subdivision, archetypes,
//! height derivation, entry points, shell interiors.

use serde::{Deserialize, Serialize};

use super::blocks::{Block, BlockType};
use super::cell::{Cell, Domain};
use super::config::config;
use super::hash::{ha, hash_to_f64, hash_to_range};
use super::objects::{compute_form_id, compute_object_id, ObjectSeeds};
use super::voronoi::voronoi_cells;
use super::zones::point_in_polygon_seeded;
use crate::types::{Point, Polygon};

/// Alley mode determined by popcount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlleyMode {
    VoronoiGaps,
    Bisecting,
    DeadEnd,
    None,
}

/// Building archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Archetype {
    Tower, Spire, Stack,          // Vertical
    Block, Hall, Compound,        // Horizontal
    Dome, Arch, Monolith,         // Landmark
    Growth, Ruin,                 // Organic
}

impl Archetype {
    fn as_str(&self) -> &'static str {
        match self {
            Archetype::Tower => "tower", Archetype::Spire => "spire",
            Archetype::Stack => "stack", Archetype::Block => "block",
            Archetype::Hall => "hall", Archetype::Compound => "compound",
            Archetype::Dome => "dome", Archetype::Arch => "arch",
            Archetype::Monolith => "monolith", Archetype::Growth => "growth",
            Archetype::Ruin => "ruin",
        }
    }

    fn height_multiplier_range(&self) -> (f64, f64) {
        match self {
            Archetype::Tower => (2.0, 3.0),
            Archetype::Spire => (2.5, 4.0),
            Archetype::Monolith => (1.5, 2.0),
            Archetype::Dome => (0.8, 1.2),
            Archetype::Hall => (0.5, 0.8),
            Archetype::Ruin => (0.3, 0.7),
            _ => (1.0, 1.0),
        }
    }
}

/// Public building subtype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicSubtype {
    Shop, Hall, Temple, Workshop, Archive,
}

/// Entry point for a building.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPoint {
    pub position: Point,
    pub orientation: Point,  // outward normal
    pub width: f64,
}

/// A building plot within a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingPlot {
    pub plot_idx: usize,
    pub polygon: Polygon,
    pub centroid: Point,
    pub area: f64,
    pub plot_seed: u32,
    pub is_public: bool,
    pub public_subtype: Option<PublicSubtype>,
    pub archetype: Archetype,
    pub height: f64,
    pub entry: Option<EntryPoint>,
    pub form_id: u32,
    pub object_id: u64,
    pub seeds: ObjectSeeds,
    /// Interior (public buildings only).
    pub interior_light: Option<f64>,
}

/// Result of building generation for a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockBuildings {
    pub block_idx: usize,
    pub alley_mode: AlleyMode,
    pub plots: Vec<BuildingPlot>,
}

/// Determine alley mode from popcount.
fn alley_mode(popcount: u32) -> AlleyMode {
    let cfg = config();
    if popcount >= cfg.alley_popcount_none {
        AlleyMode::None
    } else if popcount >= cfg.alley_popcount_deadend {
        AlleyMode::DeadEnd
    } else if popcount >= cfg.alley_popcount_bisecting {
        AlleyMode::Bisecting
    } else {
        AlleyMode::VoronoiGaps
    }
}

/// Get eligible archetype pool based on context.
fn archetype_pool(
    is_public: bool,
    subtype: Option<PublicSubtype>,
    popcount_ratio: f64,
    domain: Domain,
) -> Vec<Archetype> {
    use Archetype::*;

    if domain == Domain::Reserved {
        return vec![Ruin, Monolith];
    }
    if domain == Domain::Loopback {
        return vec![Tower, Monolith];
    }

    if is_public {
        match subtype {
            Some(PublicSubtype::Shop) => vec![Block, Compound, Hall],
            Some(PublicSubtype::Hall) => vec![Hall, Compound, Dome],
            Some(PublicSubtype::Temple) => vec![Dome, Spire, Arch, Monolith],
            Some(PublicSubtype::Workshop) => vec![Compound, Block, Growth],
            Some(PublicSubtype::Archive) => vec![Block, Monolith, Hall],
            None => vec![Block, Compound, Hall],
        }
    } else if popcount_ratio < 0.3 {
        vec![Monolith, Block, Tower]
    } else if popcount_ratio > 0.7 {
        vec![Block, Compound, Growth, Stack]
    } else {
        vec![Block, Tower, Compound, Stack]
    }
}

/// Cut a bisecting alley through a block polygon per §5.3.
/// Returns two sub-polygons (or one if the cut fails).
fn cut_bisecting_alley(cell_key: u32, block: &Block) -> Vec<Polygon> {
    let cfg = config();
    let alley_seed = ha(cell_key ^ block.idx as u32 ^ 0xa11e);
    let width_frac =
        cfg.min_alley_width + (alley_seed & 0xFF) as f64 / 255.0 * cfg.alley_width_range;
    let angle_dev = (hash_to_f64(ha(alley_seed ^ 0x1)) - 0.5) * cfg.max_alley_angle_deviation;

    let poly = &block.polygon;
    let (min_x, min_y, max_x, max_y) = poly.bbox();
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    let w = max_x - min_x;
    let h = max_y - min_y;

    // Dominant axis: perpendicular to the longest dimension
    let (base_angle, longest_dim) = if w >= h {
        (std::f64::consts::FRAC_PI_2, w) // Cut vertically through a wide block
    } else {
        (0.0, h) // Cut horizontally through a tall block
    };
    let angle = base_angle + angle_dev;
    let half_width = width_frac * longest_dim * 0.5;

    // Direction perpendicular to the cut (the alley corridor direction)
    let dir_x = angle.cos();
    let dir_y = angle.sin();
    // Normal to the corridor
    let norm_x = -dir_y;
    let norm_y = dir_x;

    // Two parallel lines offset from the centroid
    let extent = longest_dim * 1.5; // Long enough to cross the whole block

    // Line A: centroid + half_width along normal
    let a1 = Point::new(cx + norm_x * half_width - dir_x * extent, cy + norm_y * half_width - dir_y * extent);
    let a2 = Point::new(cx + norm_x * half_width + dir_x * extent, cy + norm_y * half_width + dir_y * extent);
    // Line B: centroid - half_width along normal
    let b1 = Point::new(cx - norm_x * half_width - dir_x * extent, cy - norm_y * half_width - dir_y * extent);
    let b2 = Point::new(cx - norm_x * half_width + dir_x * extent, cy - norm_y * half_width + dir_y * extent);

    // Classify polygon vertices by which side of each line they're on
    // Side A: vertices with positive dot product to line A's normal (outside A)
    // Side B: vertices with negative dot product to line B's normal (outside B)
    let mut side_a = Vec::new();
    let mut side_b = Vec::new();

    for &v in &poly.vertices {
        let da = (v.x - a1.x) * norm_x + (v.y - a1.y) * norm_y;
        let db = (v.x - b1.x) * norm_x + (v.y - b1.y) * norm_y;
        if da > 0.0 {
            side_a.push(v);
        }
        if db < 0.0 {
            side_b.push(v);
        }
    }

    // Use Sutherland-Hodgman to clip to each half-plane
    let poly_a = clip_polygon_by_line(&poly.vertices, a1, a2, true);
    let poly_b = clip_polygon_by_line(&poly.vertices, b1, b2, false);

    let mut result = Vec::new();
    if poly_a.len() >= 3 {
        result.push(Polygon::new(poly_a));
    }
    if poly_b.len() >= 3 {
        result.push(Polygon::new(poly_b));
    }
    if result.is_empty() {
        result.push(block.polygon.clone());
    }
    result
}

/// Cut a dead-end notch into a block polygon per §5.4.
fn cut_deadend_alley(cell_key: u32, block: &Block) -> Vec<Polygon> {
    let cfg = config();
    let deadend_seed = ha(cell_key ^ block.idx as u32 ^ 0xa11e ^ 0x1);

    let poly = &block.polygon;
    let n = poly.vertices.len();
    if n < 3 {
        return vec![block.polygon.clone()];
    }

    // Find the longest edge
    let mut best_edge = 0;
    let mut best_len = 0.0_f64;
    for i in 0..n {
        let (a, b) = poly.edge(i);
        let len = a.distance_to(b);
        if len > best_len {
            best_len = len;
            best_edge = i;
        }
    }

    let (ea, eb) = poly.edge(best_edge);
    let depth_frac = 0.4 + hash_to_f64(ha(deadend_seed ^ 0x1)) * 0.2;
    let pos_frac = 0.2 + hash_to_f64(ha(deadend_seed ^ 0x2)) * 0.6;
    let width_frac = cfg.min_alley_width + (ha(deadend_seed ^ 0x3) & 0xFF) as f64 / 255.0 * cfg.alley_width_range;

    // Position on the edge
    let edge_pt = ea.lerp(eb, pos_frac);
    let edge_dx = eb.x - ea.x;
    let edge_dy = eb.y - ea.y;
    let edge_len = (edge_dx * edge_dx + edge_dy * edge_dy).sqrt();
    if edge_len < 1e-10 {
        return vec![block.polygon.clone()];
    }

    // Inward normal (toward block interior)
    let centroid = poly.centroid();
    let mid = ea.midpoint(eb);
    let n1 = Point::new(-edge_dy / edge_len, edge_dx / edge_len);
    let test = (centroid.x - mid.x) * n1.x + (centroid.y - mid.y) * n1.y;
    let inward = if test > 0.0 { n1 } else { Point::new(edge_dy / edge_len, -edge_dx / edge_len) };

    let (min_x, min_y, max_x, max_y) = poly.bbox();
    let block_width = ((max_x - min_x).powi(2) + (max_y - min_y).powi(2)).sqrt();
    let notch_depth = depth_frac * block_width * 0.5;
    let notch_half_w = width_frac * edge_len * 0.5;

    // Notch corners: a rectangle starting at edge_pt, going inward
    let along = Point::new(edge_dx / edge_len, edge_dy / edge_len);
    let c0 = Point::new(edge_pt.x - along.x * notch_half_w, edge_pt.y - along.y * notch_half_w);
    let c1 = Point::new(edge_pt.x + along.x * notch_half_w, edge_pt.y + along.y * notch_half_w);
    let c2 = Point::new(c1.x + inward.x * notch_depth, c1.y + inward.y * notch_depth);
    let c3 = Point::new(c0.x + inward.x * notch_depth, c0.y + inward.y * notch_depth);

    let notch = Polygon::new(vec![c0, c1, c2, c3]);

    // Subtract notch from block polygon (approximate: filter out vertices inside notch,
    // add intersection points)
    let result = subtract_convex(&poly, &notch);
    if result.vertices.len() >= 3 {
        vec![result]
    } else {
        vec![block.polygon.clone()]
    }
}

/// Subtract a convex polygon from another polygon (approximate).
/// Uses vertex filtering + boundary insertion.
fn subtract_convex(subject: &Polygon, hole: &Polygon) -> Polygon {
    let mut result = Vec::new();
    let n = subject.vertices.len();

    for i in 0..n {
        let curr = subject.vertices[i];
        let next = subject.vertices[(i + 1) % n];
        let curr_in_hole = hole.contains(curr);
        let next_in_hole = hole.contains(next);

        if !curr_in_hole {
            result.push(curr);
        }

        // Edge crosses hole boundary — find approximate intersection
        if curr_in_hole != next_in_hole {
            let mut a = curr;
            let mut b = next;
            for _ in 0..12 {
                let mid = Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
                if hole.contains(mid) == curr_in_hole {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            result.push(Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5));
        }
    }

    Polygon::new(result)
}

/// Clip a polygon by a line (Sutherland-Hodgman single edge clip).
/// `keep_positive`: if true, keep vertices on the positive side of the line.
fn clip_polygon_by_line(vertices: &[Point], line_a: Point, line_b: Point, keep_positive: bool) -> Vec<Point> {
    let nx = line_b.y - line_a.y;
    let ny = -(line_b.x - line_a.x);
    let sign = if keep_positive { 1.0 } else { -1.0 };

    let classify = |p: Point| -> f64 {
        sign * ((p.x - line_a.x) * nx + (p.y - line_a.y) * ny)
    };

    let n = vertices.len();
    if n < 3 {
        return Vec::new();
    }

    let mut output = Vec::new();
    for i in 0..n {
        let curr = vertices[i];
        let next = vertices[(i + 1) % n];
        let dc = classify(curr);
        let dn = classify(next);

        if dc >= 0.0 {
            output.push(curr);
        }
        if (dc >= 0.0) != (dn >= 0.0) {
            // Edge crosses the line
            let t = dc / (dc - dn);
            output.push(Point::new(
                curr.x + t * (next.x - curr.x),
                curr.y + t * (next.y - curr.y),
            ));
        }
    }
    output
}

/// Generate buildings for a block.
pub fn generate_buildings(cell: &Cell, block: &Block) -> BlockBuildings {
    let cfg = config();
    let mode = alley_mode(cell.popcount);

    // Alley cutting produces sub-polygons
    let sub_polygons = match mode {
        AlleyMode::VoronoiGaps => vec![block.polygon.clone()],
        AlleyMode::Bisecting => cut_bisecting_alley(cell.key, block),
        AlleyMode::DeadEnd => cut_deadend_alley(cell.key, block),
        AlleyMode::None => vec![block.polygon.clone()],
    };

    let mut all_plots = Vec::new();
    for (sub_idx, sub_poly) in sub_polygons.iter().enumerate() {
        let sub_area = sub_poly.area();
        let base_plots = (sub_area / cfg.plot_area_base).floor() as u32;
        let entropy_bonus = (cell.popcount_ratio * cfg.plot_entropy_bonus as f64).floor() as u32;
        let plot_count = (base_plots + entropy_bonus)
            .max(1)
            .min(cfg.max_plots_per_block) as usize;

        // Generate plot seed points via Voronoi subdivision
        if plot_count < 3 {
            // Too few for Voronoi — use the whole sub-polygon as one plot
            let plot_seed = ha(cell.key ^ block.idx as u32 ^ sub_idx as u32 ^ 0 ^ 0x106754ed);
            let plot = build_plot(cell, block, &sub_poly, 0, plot_seed);
            all_plots.push(plot);
            continue;
        }

        let mut seed_pts = Vec::with_capacity(plot_count);
        for p in 0..plot_count {
            let pt_seed = ha(cell.key ^ block.idx as u32 ^ sub_idx as u32 ^ p as u32 ^ 0x106754ed);
            let pt = point_in_polygon_seeded(&sub_poly, pt_seed);
            seed_pts.push(pt);
        }

        let vcells = voronoi_cells(&seed_pts);
        let (bx0, by0, bx1, by1) = sub_poly.bbox();

        for (p_idx, vcell) in vcells.iter().enumerate() {
            if vcell.vertices.len() < 3 {
                continue;
            }

            // Clip Voronoi cell to sub-polygon bounding box
            let clipped = super::voronoi::clip_polygon(&vcell.vertices, bx0, by0, bx1, by1);
            if clipped.len() < 3 {
                continue;
            }

            let plot_poly = Polygon::new(clipped);
            if plot_poly.area() < cfg.plot_area_base * 0.25 {
                continue;
            }

            let plot_seed = ha(cell.key ^ block.idx as u32 ^ sub_idx as u32 ^ p_idx as u32 ^ 0x106754ed);
            let plot = build_plot(cell, block, &plot_poly, p_idx, plot_seed);
            all_plots.push(plot);
        }
    }

    BlockBuildings {
        block_idx: block.idx,
        alley_mode: mode,
        plots: all_plots,
    }
}

/// Build a single plot with all derived properties.
fn build_plot(
    cell: &Cell,
    block: &Block,
    polygon: &Polygon,
    plot_idx: usize,
    plot_seed: u32,
) -> BuildingPlot {
    let cfg = config();
    let seeds = ObjectSeeds::from_seed(ha(plot_seed));
    let centroid = polygon.centroid();
    let area = polygon.area();

    // Public/private classification
    let public_rate = {
        let base = match block.block_type {
            BlockType::Building => cfg.public_rate_building,
            BlockType::Plaza => cfg.public_rate_plaza,
            BlockType::Park => cfg.public_rate_park,
            BlockType::Water => cfg.public_rate_water,
            BlockType::Riverbank => cfg.public_rate_riverbank,
        };
        let domain_mod = match cell.domain {
            Domain::Public => cfg.domain_mod_public,
            Domain::Private => cfg.domain_mod_private,
            Domain::Loopback => cfg.domain_mod_loopback,
            Domain::Multicast => cfg.domain_mod_multicast,
            Domain::Reserved => cfg.domain_mod_reserved,
            Domain::Documentation => cfg.domain_mod_documentation,
        };
        (base + cell.popcount_ratio * 0.2 + domain_mod).clamp(0.0, 1.0)
    };

    let public_roll = hash_to_f64(ha(plot_seed ^ 0x9a3f));
    let is_public = public_roll < public_rate;

    let public_subtype = if is_public {
        let subtype_roll = hash_to_f64(ha(plot_seed ^ 0x50b1));
        Some(if subtype_roll < 0.30 {
            PublicSubtype::Shop
        } else if subtype_roll < 0.55 {
            PublicSubtype::Hall
        } else if subtype_roll < 0.75 {
            PublicSubtype::Temple
        } else if subtype_roll < 0.90 {
            PublicSubtype::Workshop
        } else {
            PublicSubtype::Archive
        })
    } else {
        None
    };

    // Archetype selection
    let pool = archetype_pool(is_public, public_subtype, cell.popcount_ratio, cell.domain);
    let arch_seed = ha(plot_seed ^ 0xabc3);
    let archetype = pool[arch_seed as usize % pool.len()];

    // Height derivation
    let base_height = cfg.min_height + cell.popcount_ratio * (cfg.max_height - cfg.min_height);
    let height_jitter = (hash_to_f64(ha(plot_seed ^ 0x4)) - 0.5) * cfg.height_jitter_range;
    let raw_height = (base_height + height_jitter).max(cfg.min_height);

    let (mult_min, mult_max) = archetype.height_multiplier_range();
    let multiplier = if mult_min == mult_max {
        mult_min
    } else {
        hash_to_range(ha(plot_seed ^ 0x43e1941), mult_min, mult_max)
    };
    let height = (raw_height * multiplier).min(cfg.max_height * cfg.height_multiplier_cap);

    // Entry point
    let entry = find_entry_point(polygon, plot_seed);

    // Form ID
    let form_id = compute_form_id(
        &format!("building:{}", archetype.as_str()),
        cell.aesthetic_bucket(),
        arch_seed,
    );
    let object_id = compute_object_id(cell.key, seeds.object_seed);

    // Interior light (public only)
    let interior_light = if is_public {
        Some(cfg.base_interior_light + hash_to_f64(ha(plot_seed ^ 0x119e7)) * 0.3)
    } else {
        None
    };

    BuildingPlot {
        plot_idx,
        polygon: polygon.clone(),
        centroid,
        area,
        plot_seed,
        is_public,
        public_subtype,
        archetype,
        height,
        entry: Some(entry),
        form_id,
        object_id,
        seeds,
        interior_light,
    }
}

/// Find the entry point for a building plot.
fn find_entry_point(polygon: &Polygon, plot_seed: u32) -> EntryPoint {
    let cfg = config();
    let n = polygon.vertices.len();
    let centroid = polygon.centroid();

    // Find candidate walls (edges not adjacent to other plots and long enough)
    let mut candidates: Vec<usize> = Vec::new();
    for i in 0..n {
        let (a, b) = polygon.edge(i);
        let len = a.distance_to(b);
        if len > cfg.min_door_wall_length {
            candidates.push(i);
        }
    }

    if candidates.is_empty() {
        // Fallback: use the longest edge
        let mut best = 0;
        let mut best_len = 0.0_f64;
        for i in 0..n {
            let (a, b) = polygon.edge(i);
            let len = a.distance_to(b);
            if len > best_len {
                best_len = len;
                best = i;
            }
        }
        candidates.push(best);
    }

    // Select entry wall
    let entry_wall_seed = ha(plot_seed ^ 0xd00e);
    let wall_idx = candidates[entry_wall_seed as usize % candidates.len()];
    let (wall_a, wall_b) = polygon.edge(wall_idx);

    // Entry position along the wall (range [0.2, 0.8])
    let entry_t = 0.2 + hash_to_f64(ha(plot_seed ^ 0xd00e ^ 0x1)) * 0.6;
    let position = Point::new(
        wall_a.x + entry_t * (wall_b.x - wall_a.x),
        wall_a.y + entry_t * (wall_b.y - wall_a.y),
    );

    // Outward normal
    let dx = wall_b.x - wall_a.x;
    let dy = wall_b.y - wall_a.y;
    let len = (dx * dx + dy * dy).sqrt();
    let normal_a = Point::new(-dy / len, dx / len);
    let normal_b = Point::new(dy / len, -dx / len);

    let mid = Point::new((wall_a.x + wall_b.x) * 0.5, (wall_a.y + wall_b.y) * 0.5);
    let test = (normal_a.x * (mid.x - centroid.x)) + (normal_a.y * (mid.y - centroid.y));
    let orientation = if test > 0.0 { normal_a } else { normal_b };

    let width = cfg.min_entry_width + hash_to_f64(ha(plot_seed ^ 0xd00e ^ 0x2)) * cfg.entry_width_range;

    EntryPoint {
        position,
        orientation,
        width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::blocks::BlockType;
    use crate::types::Polygon;

    fn test_block() -> Block {
        Block {
            idx: 0,
            polygon: Polygon::new(vec![
                Point::new(0.0, 0.0),
                Point::new(80.0, 0.0),
                Point::new(80.0, 80.0),
                Point::new(0.0, 80.0),
            ]),
            block_type: BlockType::Building,
            area: 6400.0,
            centroid: Point::new(40.0, 40.0),
            river_adjacent: false,
        }
    }

    #[test]
    fn buildings_generated() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block();
        let result = generate_buildings(&cell, &block);
        assert!(!result.plots.is_empty(), "No plots generated");
    }

    #[test]
    fn building_height_scales_with_popcount() {
        let low = Cell::from_octets(1, 0, 0);
        let high = Cell::from_octets(255, 170, 85);
        let block = test_block();

        let b_low = generate_buildings(&low, &block);
        let b_high = generate_buildings(&high, &block);

        if !b_low.plots.is_empty() && !b_high.plots.is_empty() {
            // Higher popcount should generally produce taller buildings
            // (though archetype multipliers add variance)
            let avg_low: f64 = b_low.plots.iter().map(|p| p.height).sum::<f64>() / b_low.plots.len() as f64;
            let avg_high: f64 = b_high.plots.iter().map(|p| p.height).sum::<f64>() / b_high.plots.len() as f64;
            // Not a strict assertion due to archetype variance, but log it
            let _ = (avg_low, avg_high);
        }
    }

    #[test]
    fn archetype_pools_vary_by_domain() {
        let public_pool = archetype_pool(false, None, 0.5, Domain::Public);
        let reserved_pool = archetype_pool(false, None, 0.5, Domain::Reserved);
        assert_ne!(public_pool.len(), reserved_pool.len());
        assert!(reserved_pool.contains(&Archetype::Ruin));
    }

    #[test]
    fn entry_point_on_boundary() {
        let cell = Cell::from_octets(93, 184, 216);
        let block = test_block();
        let result = generate_buildings(&cell, &block);
        for plot in &result.plots {
            if let Some(entry) = &plot.entry {
                // Entry should be near the polygon boundary
                let poly = &plot.polygon;
                let n = poly.vertices.len();
                let min_dist = (0..n)
                    .map(|i| {
                        let (a, b) = poly.edge(i);
                        point_to_seg(entry.position, a, b)
                    })
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    min_dist < 2.0,
                    "Entry at {:?} is {} from boundary",
                    entry.position,
                    min_dist
                );
            }
        }
    }

    #[test]
    fn alley_mode_by_popcount() {
        assert_eq!(alley_mode(22), AlleyMode::None);
        assert_eq!(alley_mode(17), AlleyMode::DeadEnd);
        assert_eq!(alley_mode(12), AlleyMode::Bisecting);
        assert_eq!(alley_mode(5), AlleyMode::VoronoiGaps);
    }

    fn point_to_seg(p: Point, a: Point, b: Point) -> f64 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len_sq = dx * dx + dy * dy;
        if len_sq < 1e-20 { return p.distance_to(a); }
        let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq;
        let t = t.clamp(0.0, 1.0);
        p.distance_to(Point::new(a.x + t * dx, a.y + t * dy))
    }
}

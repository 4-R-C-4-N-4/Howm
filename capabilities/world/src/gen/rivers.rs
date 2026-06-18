//! River system.
//!
//! Rivers are identified by their `gx` value. Each `gx` either hosts a river
//! or does not, determined by a hash threshold. Rivers flow north to south
//! (decreasing `gy`) through every cell at that `gx`.

use serde::{Deserialize, Serialize};

use super::cell::Cell;
use super::config::config;
use super::district::{DistrictGeometry, SharedEdge};
use super::hash::{ha, hash_to_f64};
use crate::types::Point;

/// A river segment within a single district cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiverSegment {
    /// The `gx` value identifying this river.
    pub river_gx: u32,
    /// Entry point (from the north / gy+1 edge).
    pub entry: Point,
    /// Exit point (to the south / gy-1 edge).
    pub exit: Point,
    /// Cubic bezier control point 1.
    pub cp1: Point,
    /// Cubic bezier control point 2.
    pub cp2: Point,
}

impl RiverSegment {
    /// Evaluate the bezier curve at parameter t in [0, 1].
    pub fn at(&self, t: f64) -> Point {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        Point::new(
            mt3 * self.entry.x
                + 3.0 * mt2 * t * self.cp1.x
                + 3.0 * mt * t2 * self.cp2.x
                + t3 * self.exit.x,
            mt3 * self.entry.y
                + 3.0 * mt2 * t * self.cp1.y
                + 3.0 * mt * t2 * self.cp2.y
                + t3 * self.exit.y,
        )
    }

    /// Approximate the bezier as a polyline with `n` segments.
    pub fn to_polyline(&self, n: usize) -> Vec<Point> {
        (0..=n).map(|i| self.at(i as f64 / n as f64)).collect()
    }
}

/// Test whether a given `gx` value hosts a river.
pub fn is_river(gx: u32) -> bool {
    let cfg = config();
    let threshold =
        (cfg.river_density_percent / 100.0 * 0xFFFF_FFFF_u32 as f64) as u32;
    ha(gx ^ cfg.river_salt) < threshold
}

/// Compute the river edge crossing point on the shared boundary between
/// two adjacent cells for river R.
///
/// The `edge_start` and `edge_end` must be canonicalized (sorted by
/// x + y * 100000) so both cells compute identical positions.
fn river_crossing(
    key_a: u32,
    key_b: u32,
    river_gx: u32,
    edge_start: Point,
    edge_end: Point,
) -> Point {
    let min_key = key_a.min(key_b);
    let max_key = key_a.max(key_b);
    let hash_input =
        min_key ^ ((max_key & 0xFFFF) << 8) ^ river_gx.wrapping_mul(0x9e3779b9);
    let t = 0.1 + hash_to_f64(ha(hash_input)) * 0.8;

    Point::new(
        edge_start.x + t * (edge_end.x - edge_start.x),
        edge_start.y + t * (edge_end.y - edge_start.y),
    )
}

/// Canonical sort key for an edge vertex: x + y * 100000.
fn vertex_sort_key(p: &Point) -> f64 {
    p.x + p.y * 100000.0
}

/// Canonicalize edge endpoints so both cells see the same ordering.
fn canonicalize_edge(a: Point, b: Point) -> (Point, Point) {
    if vertex_sort_key(&a) <= vertex_sort_key(&b) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Find the shared edge between a cell and its grid neighbour at `dy` offset,
/// using the district's already-computed `shared_edges` (exact — matched by
/// neighbour key). Returns the canonicalised edge (start, end) and the neighbour
/// key, or None when the cell isn't Voronoi-adjacent to that grid neighbour (so
/// the river legitimately can't cross there).
///
/// Both districts derive the SAME canonical edge for a shared boundary, so the
/// river crossing matches across the border — the old seed-bisector heuristic
/// could pick mismatched edges and break continuity.
fn find_vertical_neighbor_edge(
    cell: &Cell,
    shared_edges: &[SharedEdge],
    dy: i32,
) -> Option<(Point, Point, u32)> {
    let neighbor_key = cell.neighbor_key(0, dy);
    shared_edges
        .iter()
        .find(|e| e.neighbor_key == neighbor_key)
        .map(|e| {
            let (ca, cb) = canonicalize_edge(e.start, e.end);
            (ca, cb, neighbor_key)
        })
}

/// Generate river segments for a cell.
///
/// Returns segments for each river that passes through this cell's `gx`.
pub fn generate_rivers(district: &DistrictGeometry) -> Vec<RiverSegment> {
    let cell = &district.cell;
    let gx = cell.gx;

    if !is_river(gx) {
        return vec![];
    }

    // North (gy+1) and south (gy-1) shared edges, from the district's exact
    // shared-edge list.
    let north_edge = find_vertical_neighbor_edge(cell, &district.shared_edges, 1);
    let south_edge = find_vertical_neighbor_edge(cell, &district.shared_edges, -1);

    let (north_start, north_end, north_key) = match north_edge {
        Some(e) => e,
        None => return vec![], // No north neighbor edge — can't route river
    };
    let (south_start, south_end, south_key) = match south_edge {
        Some(e) => e,
        None => return vec![],
    };

    let entry = river_crossing(cell.key, north_key, gx, north_start, north_end);
    let exit = river_crossing(cell.key, south_key, gx, south_start, south_end);

    // For Catmull-Rom, we need the points at gy+2 and gy-2
    let north2_cell = Cell::from_key(cell.neighbor_key(0, 2));
    let south2_cell = Cell::from_key(cell.neighbor_key(0, -2));

    // Approximate the far crossing points. For full accuracy, we'd need
    // the polygon of the gy+1 and gy-1 cells. For now, use a linear
    // extrapolation based on the entry/exit positions.
    let pt_nn = if is_river(north2_cell.gx) {
        // Estimate: the gy+2 crossing is roughly as far above entry
        // as entry is above exit
        let dy = entry.y - exit.y;
        Point::new(
            entry.x + (entry.x - exit.x) * 0.5,
            entry.y + dy,
        )
    } else {
        // Fallback: extend linearly from exit through entry
        Point::new(
            2.0 * entry.x - exit.x,
            2.0 * entry.y - exit.y,
        )
    };

    let pt_ss = if is_river(south2_cell.gx) {
        let dy = exit.y - entry.y;
        Point::new(
            exit.x + (exit.x - entry.x) * 0.5,
            exit.y + dy,
        )
    } else {
        Point::new(
            2.0 * exit.x - entry.x,
            2.0 * exit.y - entry.y,
        )
    };

    // Catmull-Rom tangents
    let tangent_entry = Point::new(
        (exit.x - pt_nn.x) / 2.0,
        (exit.y - pt_nn.y) / 2.0,
    );
    let tangent_exit = Point::new(
        (pt_ss.x - entry.x) / 2.0,
        (pt_ss.y - entry.y) / 2.0,
    );

    let cp1 = Point::new(
        entry.x + tangent_entry.x / 3.0,
        entry.y + tangent_entry.y / 3.0,
    );
    let cp2 = Point::new(
        exit.x - tangent_exit.x / 3.0,
        exit.y - tangent_exit.y / 3.0,
    );

    vec![RiverSegment {
        river_gx: gx,
        entry,
        exit,
        cp1,
        cp2,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn river_density() {
        // At 8%, roughly 20 out of 256 gx values should host rivers
        let count = (0..256).filter(|&gx| is_river(gx)).count();
        // Allow generous range: 5-50
        assert!(
            count >= 5 && count <= 50,
            "River count {} is outside expected range",
            count
        );
    }

    #[test]
    fn river_identity_deterministic() {
        for gx in 0..256 {
            let r1 = is_river(gx);
            let r2 = is_river(gx);
            assert_eq!(r1, r2, "River identity not deterministic for gx={}", gx);
        }
    }

    #[test]
    fn river_crossing_symmetric() {
        // Both cells should compute the same crossing point
        let a = Point::new(0.0, 0.0);
        let b = Point::new(100.0, 0.0);
        let (ca, cb) = canonicalize_edge(a, b);

        let p1 = river_crossing(0x010000, 0x010100, 42, ca, cb);
        let p2 = river_crossing(0x010100, 0x010000, 42, ca, cb);
        assert!((p1.x - p2.x).abs() < 1e-10);
        assert!((p1.y - p2.y).abs() < 1e-10);
    }

    #[test]
    fn bezier_endpoints() {
        let seg = RiverSegment {
            river_gx: 0,
            entry: Point::new(0.0, 100.0),
            exit: Point::new(10.0, 0.0),
            cp1: Point::new(3.0, 70.0),
            cp2: Point::new(7.0, 30.0),
        };
        let start = seg.at(0.0);
        let end = seg.at(1.0);
        assert!((start.x - seg.entry.x).abs() < 1e-10);
        assert!((start.y - seg.entry.y).abs() < 1e-10);
        assert!((end.x - seg.exit.x).abs() < 1e-10);
        assert!((end.y - seg.exit.y).abs() < 1e-10);
    }
}

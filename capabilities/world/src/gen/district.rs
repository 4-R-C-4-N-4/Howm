//! District geometry: seed point placement, Voronoi polygon extraction,
//! and neighbor management.
//!
//! A district is the spatial expression of one /24 cell. Its polygon boundary
//! is determined by the Voronoi diagram of seed points from a 5×5 neighborhood.

use serde::{Deserialize, Serialize};

use super::cell::Cell;
use super::config::config;
use super::hash::{ha, hb, hash_to_f64};
use super::voronoi::{voronoi_cells, VoronoiCell};
use crate::types::{Point, Polygon};

/// A seed point in world space, associated with a cell.
#[derive(Debug, Clone)]
pub struct SeedPoint {
    pub cell: Cell,
    pub position: Point,
    /// Offset from the query cell in grid coords.
    pub dx: i32,
    pub dy: i32,
}

/// The result of generating a district's geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistrictGeometry {
    /// The query cell this district is centered on.
    pub cell: Cell,
    /// The district polygon (Voronoi cell boundary).
    pub polygon: Polygon,
    /// Shared edges with neighbors: (neighbor_key, edge_start, edge_end).
    pub shared_edges: Vec<SharedEdge>,
    /// Seed point position in world space.
    pub seed_position: Point,
}

/// A shared edge between two adjacent districts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedEdge {
    /// Cell key of the neighbor across this edge.
    pub neighbor_key: u32,
    /// Start point of the shared edge.
    pub start: Point,
    /// End point of the shared edge.
    pub end: Point,
    /// Index of this edge in the district polygon.
    pub edge_idx: usize,
    /// Length of this edge in world units.
    pub length: f64,
}

/// Compute the world-space seed point position for a cell.
///
/// Position = grid_pos * SCALE + deterministic jitter from cell key.
/// Jitter is always derived from the cell key alone — never from view state.
pub fn seed_position(cell: &Cell) -> Point {
    let cfg = config();
    let jx = (hash_to_f64(ha(cell.key)) - 0.5) * cfg.scale * cfg.jitter_default;
    let jy = (hash_to_f64(hb(cell.key)) - 0.5) * cfg.scale * cfg.jitter_default;
    Point::new(
        cell.gx as f64 * cfg.scale + jx,
        cell.gy as f64 * cfg.scale + jy,
    )
}

/// Generate the 5×5 neighborhood of seed points centered on a cell.
///
/// Returns 25 seed points: the query cell at index 0, then 24 neighbors
/// in row-major order.
fn generate_neighborhood(center: &Cell) -> Vec<SeedPoint> {
    let mut points = Vec::with_capacity(25);

    // Center cell first
    points.push(SeedPoint {
        cell: center.clone(),
        position: seed_position(center),
        dx: 0,
        dy: 0,
    });

    // Radius-2 neighborhood (24 surrounding cells)
    for dy in -2..=2_i32 {
        for dx in -2..=2_i32 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nkey = center.neighbor_key(dx, dy);
            let ncell = Cell::from_key(nkey);
            let pos = seed_position(&ncell);
            points.push(SeedPoint {
                cell: ncell,
                position: pos,
                dx,
                dy,
            });
        }
    }

    points
}

/// Generate the full district geometry for a cell.
///
/// Computes Voronoi diagram over the 5×5 neighborhood and extracts the
/// center cell's polygon with shared edge information.
pub fn generate_district(center: &Cell) -> DistrictGeometry {
    let neighborhood = generate_neighborhood(center);
    let seed_pts: Vec<Point> = neighborhood.iter().map(|s| s.position).collect();

    // Compute Voronoi cells for all 25 points
    let vcells = voronoi_cells(&seed_pts);

    // The center cell is at index 0
    let center_vcell = &vcells[0];
    let polygon = Polygon::new(center_vcell.vertices.clone());
    let center_pos = seed_pts[0];

    // Identify shared edges with ring-1 neighbors
    let shared_edges = identify_shared_edges(
        &center_vcell.vertices,
        &neighborhood,
        &vcells,
    );

    DistrictGeometry {
        cell: center.clone(),
        polygon,
        shared_edges,
        seed_position: center_pos,
    }
}

/// Identify which edges of the center cell's polygon are shared with neighbors.
///
/// Two adjacent Voronoi cells share an edge that is the perpendicular bisector
/// of the line between their seed points. We match polygon edges to neighbors
/// by checking which neighbor's Voronoi cell shares the same edge vertices.
fn identify_shared_edges(
    center_verts: &[Point],
    neighborhood: &[SeedPoint],
    vcells: &[VoronoiCell],
) -> Vec<SharedEdge> {
    let n = center_verts.len();
    if n < 3 {
        return vec![];
    }

    let mut shared = Vec::new();

    for edge_idx in 0..n {
        let start = center_verts[edge_idx];
        let end = center_verts[(edge_idx + 1) % n];

        // Check each ring-1 neighbor to see if they share this edge
        for (ni, seed_pt) in neighborhood.iter().enumerate().skip(1) {
            // Only check ring-1 neighbors (immediately adjacent)
            if seed_pt.dx.abs() > 1 || seed_pt.dy.abs() > 1 {
                continue;
            }

            let ncell = &vcells[ni];
            if shares_edge(&start, &end, &ncell.vertices) {
                shared.push(SharedEdge {
                    neighbor_key: seed_pt.cell.key,
                    start,
                    end,
                    edge_idx,
                    length: start.distance_to(end),
                });
                break;
            }
        }
    }

    shared
}

/// Check if a Voronoi cell's polygon shares an edge (approximately) with
/// the given start/end points.
fn shares_edge(start: &Point, end: &Point, other_verts: &[Point]) -> bool {
    let tol = 1e-6;
    let n = other_verts.len();
    for i in 0..n {
        let os = other_verts[i];
        let oe = other_verts[(i + 1) % n];

        // Check both orientations (edges may be reversed)
        let fwd = start.distance_to(os) < tol && end.distance_to(oe) < tol;
        let rev = start.distance_to(oe) < tol && end.distance_to(os) < tol;
        if fwd || rev {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_position_deterministic() {
        let c = Cell::from_octets(93, 184, 216);
        let p1 = seed_position(&c);
        let p2 = seed_position(&c);
        assert_eq!(p1.x, p2.x);
        assert_eq!(p1.y, p2.y);
    }

    #[test]
    fn seed_position_varies_with_key() {
        let c1 = Cell::from_octets(1, 0, 0);
        let c2 = Cell::from_octets(2, 0, 0);
        let p1 = seed_position(&c1);
        let p2 = seed_position(&c2);
        // Different cells should have different seed positions
        assert!((p1.x - p2.x).abs() > 0.01 || (p1.y - p2.y).abs() > 0.01);
    }

    #[test]
    fn neighborhood_has_25_points() {
        let c = Cell::from_octets(93, 184, 216);
        let hood = generate_neighborhood(&c);
        assert_eq!(hood.len(), 25);
        assert_eq!(hood[0].dx, 0);
        assert_eq!(hood[0].dy, 0);
    }

    #[test]
    fn district_has_polygon() {
        let c = Cell::from_octets(93, 184, 216);
        let d = generate_district(&c);
        // Center cell should have a polygon with at least 3 vertices
        assert!(
            d.polygon.vertices.len() >= 3,
            "District polygon has only {} vertices",
            d.polygon.vertices.len()
        );
    }

    #[test]
    fn district_polygon_contains_seed() {
        let c = Cell::from_octets(93, 184, 216);
        let d = generate_district(&c);
        // The seed point should be inside the polygon
        assert!(
            d.polygon.contains(d.seed_position),
            "Seed point {:?} not inside polygon",
            d.seed_position
        );
    }

    #[test]
    fn district_has_shared_edges() {
        let c = Cell::from_octets(93, 184, 216);
        let d = generate_district(&c);
        // Should have at least a few shared edges with neighbors
        assert!(
            !d.shared_edges.is_empty(),
            "District has no shared edges"
        );
    }

    #[test]
    fn district_deterministic() {
        let c = Cell::from_octets(93, 184, 216);
        let d1 = generate_district(&c);
        let d2 = generate_district(&c);
        assert_eq!(d1.polygon.vertices.len(), d2.polygon.vertices.len());
        for (v1, v2) in d1.polygon.vertices.iter().zip(d2.polygon.vertices.iter()) {
            assert!((v1.x - v2.x).abs() < 1e-10);
            assert!((v1.y - v2.y).abs() < 1e-10);
        }
    }
}

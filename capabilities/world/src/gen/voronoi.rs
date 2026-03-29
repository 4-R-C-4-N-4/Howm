//! Voronoi diagram computation via Bowyer-Watson Delaunay triangulation.
//!
//! We only ever compute Voronoi for 25 points (5×5 neighborhood), so
//! algorithmic complexity is irrelevant. The focus is correctness and
//! determinism.

use crate::types::Point;

/// A Delaunay triangle referencing point indices.
#[derive(Debug, Clone, Copy)]
struct Triangle {
    a: usize,
    b: usize,
    c: usize,
}

/// Circumcenter of a triangle defined by three points.
fn circumcenter(pa: Point, pb: Point, pc: Point) -> Point {
    let d = 2.0
        * (pa.x * (pb.y - pc.y)
            + pb.x * (pc.y - pa.y)
            + pc.x * (pa.y - pb.y));

    if d.abs() < 1e-10 {
        // Degenerate — return centroid as fallback
        return Point::new(
            (pa.x + pb.x + pc.x) / 3.0,
            (pa.y + pb.y + pc.y) / 3.0,
        );
    }

    let ax2 = pa.x * pa.x + pa.y * pa.y;
    let bx2 = pb.x * pb.x + pb.y * pb.y;
    let cx2 = pc.x * pc.x + pc.y * pc.y;

    Point::new(
        (ax2 * (pb.y - pc.y) + bx2 * (pc.y - pa.y) + cx2 * (pa.y - pb.y)) / d,
        (ax2 * (pc.x - pb.x) + bx2 * (pa.x - pc.x) + cx2 * (pb.x - pa.x)) / d,
    )
}

/// Squared distance from point to circumcenter, minus squared circumradius.
/// Negative = inside circumcircle.
fn in_circumcircle(p: Point, pa: Point, pb: Point, pc: Point) -> bool {
    let cc = circumcenter(pa, pb, pc);
    let r2 = pa.distance_sq(cc);
    p.distance_sq(cc) <= r2 + 1e-10
}

/// Bowyer-Watson Delaunay triangulation.
///
/// Returns a list of triangles (as index triples into `pts`) and their
/// circumcenters.
fn triangulate(pts: &[Point]) -> (Vec<Triangle>, Vec<Point>) {
    let n = pts.len();
    assert!(n >= 3, "Need at least 3 points for triangulation");

    // Compute bounding box
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in pts {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let d = (max_x - min_x).max(max_y - min_y) * 10.0;

    // Super-triangle vertices (appended after input points)
    let st = [
        Point::new(min_x - d, min_y - d * 3.0),
        Point::new(min_x - d, max_y + d),
        Point::new(max_x + d * 3.0, min_y - d),
    ];
    let all: Vec<Point> = pts.iter().copied().chain(st.iter().copied()).collect();

    // Start with the super-triangle
    let mut tris: Vec<Triangle> = vec![Triangle {
        a: n,
        b: n + 1,
        c: n + 2,
    }];

    // Insert each point
    for i in 0..n {
        let p = pts[i];

        // Find all triangles whose circumcircle contains p
        let mut bad_indices = Vec::new();
        let mut good = Vec::new();
        for (ti, tri) in tris.iter().enumerate() {
            if in_circumcircle(p, all[tri.a], all[tri.b], all[tri.c]) {
                bad_indices.push(ti);
            } else {
                good.push(*tri);
            }
        }

        // Collect boundary edges of the hole.
        // An edge is on the boundary if it appears exactly once among bad triangles.
        let bad_tris: Vec<Triangle> = bad_indices.iter().map(|&ti| tris[ti]).collect();
        let mut all_edges: Vec<(usize, usize)> = Vec::new();
        for tri in &bad_tris {
            all_edges.push((tri.a, tri.b));
            all_edges.push((tri.b, tri.c));
            all_edges.push((tri.c, tri.a));
        }

        let mut unique_edges: Vec<(usize, usize)> = Vec::new();
        for edge in &all_edges {
            // Count how many times this edge appears (in either direction)
            let count = all_edges
                .iter()
                .filter(|e| {
                    (e.0 == edge.0 && e.1 == edge.1) || (e.0 == edge.1 && e.1 == edge.0)
                })
                .count();
            if count == 1 {
                unique_edges.push(*edge);
            }
        }

        // Form new triangles from boundary edges to the new point
        for (a, b) in &unique_edges {
            good.push(Triangle { a: i, b: *a, c: *b });
        }

        tris = good;
    }

    // Remove triangles that reference super-triangle vertices
    tris.retain(|t| t.a < n && t.b < n && t.c < n);

    let ccs: Vec<Point> = tris
        .iter()
        .map(|t| circumcenter(all[t.a], all[t.b], all[t.c]))
        .collect();

    (tris, ccs)
}

/// A Voronoi cell: the polygon around a seed point.
#[derive(Debug, Clone)]
pub struct VoronoiCell {
    /// Index of the seed point in the input array.
    pub site: usize,
    /// Vertices of the cell polygon, ordered by angle around the seed point.
    pub vertices: Vec<Point>,
}

/// Compute Voronoi cells from seed points.
///
/// Returns one cell per input point. Cells on the convex hull boundary may
/// be open (extending to infinity). For district generation, we always have
/// a radius-2 neighborhood, so the center cell and ring-1 cells are fully
/// enclosed.
pub fn voronoi_cells(pts: &[Point]) -> Vec<VoronoiCell> {
    if pts.len() < 3 {
        return pts
            .iter()
            .enumerate()
            .map(|(i, _)| VoronoiCell {
                site: i,
                vertices: vec![],
            })
            .collect();
    }

    let (tris, ccs) = triangulate(pts);

    // For each point, collect the circumcenters of all triangles it belongs to,
    // then sort by angle around the point to form the cell polygon.
    pts.iter()
        .enumerate()
        .map(|(i, p)| {
            let mut cell_verts: Vec<Point> = tris
                .iter()
                .enumerate()
                .filter(|(_, t)| t.a == i || t.b == i || t.c == i)
                .map(|(ti, _)| ccs[ti])
                .collect();

            // Sort by angle around the seed point
            cell_verts.sort_by(|a, b| {
                let aa = (a.y - p.y).atan2(a.x - p.x);
                let ab = (b.y - p.y).atan2(b.x - p.x);
                aa.partial_cmp(&ab).unwrap()
            });

            VoronoiCell {
                site: i,
                vertices: cell_verts,
            }
        })
        .collect()
}

/// Clip a polygon to a rectangle [x0, y0] → [x1, y1] using Sutherland-Hodgman.
pub fn clip_polygon(poly: &[Point], x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    if poly.is_empty() {
        return vec![];
    }

    let mut output = poly.to_vec();

    // Four clip edges: left, right, bottom, top
    let clips: [(fn(Point, f64) -> bool, fn(Point, Point, f64) -> Point, f64); 4] = [
        (
            |p, v| p.x >= v,
            |a, b, v| {
                let t = (v - a.x) / (b.x - a.x);
                Point::new(v, a.y + t * (b.y - a.y))
            },
            x0,
        ),
        (
            |p, v| p.x <= v,
            |a, b, v| {
                let t = (v - a.x) / (b.x - a.x);
                Point::new(v, a.y + t * (b.y - a.y))
            },
            x1,
        ),
        (
            |p, v| p.y >= v,
            |a, b, v| {
                let t = (v - a.y) / (b.y - a.y);
                Point::new(a.x + t * (b.x - a.x), v)
            },
            y0,
        ),
        (
            |p, v| p.y <= v,
            |a, b, v| {
                let t = (v - a.y) / (b.y - a.y);
                Point::new(a.x + t * (b.x - a.x), v)
            },
            y1,
        ),
    ];

    for (inside, intersect, val) in &clips {
        if output.is_empty() {
            return vec![];
        }
        let input = output;
        output = Vec::new();
        let n = input.len();
        for i in 0..n {
            let cur = input[i];
            let prev = input[(i + n - 1) % n];
            let cur_in = inside(cur, *val);
            let prev_in = inside(prev, *val);
            if cur_in {
                if !prev_in {
                    output.push(intersect(prev, cur, *val));
                }
                output.push(cur);
            } else if prev_in {
                output.push(intersect(prev, cur, *val));
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circumcenter_equilateral() {
        let cc = circumcenter(
            Point::new(0.0, 0.0),
            Point::new(2.0, 0.0),
            Point::new(1.0, 3.0_f64.sqrt()),
        );
        assert!((cc.x - 1.0).abs() < 1e-8);
        assert!((cc.y - 3.0_f64.sqrt() / 3.0).abs() < 1e-8);
    }

    #[test]
    fn simple_voronoi() {
        // 9 points in a 3×3 grid — center cell should be fully enclosed
        let mut pts = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                pts.push(Point::new(x as f64 * 10.0, y as f64 * 10.0));
            }
        }
        let cells = voronoi_cells(&pts);
        assert_eq!(cells.len(), 9);
        // Center cell (index 4) should have at least 4 vertices
        let center = &cells[4];
        assert!(
            center.vertices.len() >= 4,
            "Center cell has only {} vertices",
            center.vertices.len()
        );
    }

    #[test]
    fn voronoi_25_points() {
        // Simulate a 5×5 grid with jitter — this is our real use case
        let mut pts = Vec::new();
        for gy in 0..5 {
            for gx in 0..5 {
                let jx = ((gx * 37 + gy * 13) % 20) as f64 - 10.0;
                let jy = ((gx * 17 + gy * 41) % 20) as f64 - 10.0;
                pts.push(Point::new(gx as f64 * 100.0 + jx, gy as f64 * 100.0 + jy));
            }
        }
        let cells = voronoi_cells(&pts);
        assert_eq!(cells.len(), 25);
        // Center cell (index 12) should be fully enclosed with >= 4 vertices
        let center = &cells[12];
        assert!(
            center.vertices.len() >= 4,
            "Center cell has only {} vertices",
            center.vertices.len()
        );
    }

    #[test]
    fn clip_polygon_basic() {
        // Triangle that extends outside a clipping rect
        let tri = vec![
            Point::new(-5.0, 5.0),
            Point::new(15.0, 5.0),
            Point::new(5.0, 15.0),
        ];
        let clipped = clip_polygon(&tri, 0.0, 0.0, 10.0, 10.0);
        assert!(!clipped.is_empty());
        // All clipped vertices should be inside [0,10]
        for v in &clipped {
            assert!(v.x >= -1e-10 && v.x <= 10.0 + 1e-10);
            assert!(v.y >= -1e-10 && v.y <= 10.0 + 1e-10);
        }
    }
}

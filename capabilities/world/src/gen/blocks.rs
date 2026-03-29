//! Block system: PSLG construction and face extraction.
//!
//! Once roads and rivers are placed, the remaining interior space is
//! subdivided into blocks — regions bounded by roads, rivers, and the
//! cell boundary. Each block is typed (building, park, water, plaza,
//! riverbank).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::cell::Cell;
use super::config::config;
use super::hash::ha;
use super::rivers::RiverSegment;
use super::roads::{RoadNetwork, RoadFate};
use crate::types::{Point, Polygon};

/// Block type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockType {
    Building,
    Park,
    Water,
    Riverbank,
    Plaza,
}

/// A block within a district.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Sequential index (stable for a given cell key).
    pub idx: usize,
    /// Block polygon.
    pub polygon: Polygon,
    /// Block type.
    pub block_type: BlockType,
    /// Area in world units².
    pub area: f64,
    /// Centroid position.
    pub centroid: Point,
    /// Whether this block touches a river.
    pub river_adjacent: bool,
}

/// A directed half-edge in the PSLG.
#[derive(Debug, Clone)]
struct HalfEdge {
    /// Start vertex index.
    from: usize,
    /// End vertex index.
    to: usize,
    /// Index of the next half-edge in the face traversal.
    next: Option<usize>,
    /// Whether this half-edge has been used in a face.
    used: bool,
}

/// Snap a point to the block snap grid.
fn snap(p: Point) -> Point {
    let r = config().block_snap;
    Point::new(
        (p.x / r).round() * r,
        (p.y / r).round() * r,
    )
}

/// Find or insert a vertex in the vertex list (with snap tolerance).
fn find_or_insert_vertex(vertices: &mut Vec<Point>, p: Point) -> usize {
    let snapped = snap(p);
    let tol = config().block_snap * 0.99;
    for (i, v) in vertices.iter().enumerate() {
        if (v.x - snapped.x).abs() < tol && (v.y - snapped.y).abs() < tol {
            return i;
        }
    }
    vertices.push(snapped);
    vertices.len() - 1
}

/// Segment in the PSLG before half-edge construction.
#[derive(Debug)]
struct PslgSegment {
    a: usize,
    b: usize,
}

/// Build the PSLG from cell boundary, roads, and rivers.
/// Returns the extracted block faces.
pub fn extract_blocks(
    cell: &Cell,
    boundary: &Polygon,
    roads: &RoadNetwork,
    rivers: &[RiverSegment],
) -> Vec<Block> {
    let cfg = config();

    // ── Step 1: Collect all PSLG vertices and segments ──
    let mut vertices: Vec<Point> = Vec::new();
    let mut segments: Vec<PslgSegment> = Vec::new();

    // Cell boundary edges
    let n = boundary.vertices.len();
    for i in 0..n {
        let a = find_or_insert_vertex(&mut vertices, boundary.vertices[i]);
        let b = find_or_insert_vertex(
            &mut vertices,
            boundary.vertices[(i + 1) % n],
        );
        if a != b {
            segments.push(PslgSegment { a, b });
        }
    }

    // Road segments (exclude dead-end stubs — they don't bound faces)
    for seg in &roads.segments {
        if seg.fate == RoadFate::DeadEnd {
            continue;
        }
        let a = find_or_insert_vertex(&mut vertices, seg.a);
        let b = find_or_insert_vertex(&mut vertices, seg.b);
        if a != b {
            segments.push(PslgSegment { a, b });
        }
    }

    // River segments (approximated as polylines)
    for river in rivers {
        let polyline = river.to_polyline(8);
        for i in 0..polyline.len() - 1 {
            let a = find_or_insert_vertex(&mut vertices, polyline[i]);
            let b = find_or_insert_vertex(&mut vertices, polyline[i + 1]);
            if a != b {
                segments.push(PslgSegment { a, b });
            }
        }
    }

    // ── Step 2: Find all intersections and split segments ──
    let split_segments = split_at_intersections(&mut vertices, &segments, cfg.intersect_margin);

    // ── Step 3: Build half-edge structure ──
    let faces = extract_faces(&vertices, &split_segments, cfg.block_face_iter_limit);

    // ── Step 4: Filter and classify blocks ──
    let mut blocks: Vec<Block> = Vec::new();
    let mut areas: Vec<f64> = Vec::new();

    for face in &faces {
        let poly = Polygon::new(face.clone());
        let area = poly.area();
        if area < cfg.block_min_area {
            continue;
        }
        // Skip the exterior face (largest negative signed area or wraps the whole cell)
        let signed = poly.signed_area();
        if signed > 0.0 {
            // Positive signed area = exterior face (CCW winding for exterior)
            continue;
        }
        areas.push(area);
    }

    if areas.is_empty() {
        return blocks;
    }

    // Compute median area for block typing
    let mut sorted_areas = areas.clone();
    sorted_areas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_area = sorted_areas[sorted_areas.len() / 2];

    // Build block list
    let mut block_candidates: Vec<(Point, Polygon, f64, bool)> = Vec::new();
    for face in &faces {
        let poly = Polygon::new(face.clone());
        let area = poly.area();
        if area < cfg.block_min_area {
            continue;
        }
        let signed = poly.signed_area();
        if signed > 0.0 {
            continue;
        }
        let centroid = poly.centroid();

        // Check river adjacency
        let river_adj = !rivers.is_empty()
            && rivers.iter().any(|r| {
                let polyline = r.to_polyline(8);
                polyline.iter().any(|p| poly.contains(*p))
            });

        block_candidates.push((centroid, poly, area, river_adj));
    }

    // Sort by centroid position for stable indexing
    block_candidates.sort_by(|a, b| {
        let ka = a.0.x + a.0.y * 10000.0;
        let kb = b.0.x + b.0.y * 10000.0;
        ka.partial_cmp(&kb).unwrap()
    });

    for (idx, (centroid, poly, area, river_adj)) in
        block_candidates.into_iter().enumerate()
    {
        let norm_area = area / median_area;
        let block_type = classify_block(
            cell,
            idx,
            norm_area,
            river_adj,
        );

        blocks.push(Block {
            idx,
            polygon: poly,
            block_type,
            area,
            centroid,
            river_adjacent: river_adj,
        });
    }

    blocks
}

/// Classify a block based on normalised area and cell properties.
fn classify_block(
    cell: &Cell,
    block_idx: usize,
    norm_area: f64,
    river_adjacent: bool,
) -> BlockType {
    let cfg = config();
    let pr = cell.popcount_ratio;

    // Large blocks
    if norm_area > cfg.block_large_threshold {
        if pr < cfg.block_entropy_water {
            return if river_adjacent {
                BlockType::Riverbank
            } else {
                BlockType::Water
            };
        }
        return BlockType::Park;
    }

    // Medium blocks
    if norm_area > cfg.block_medium_threshold {
        if pr < cfg.block_entropy_plaza {
            return BlockType::Plaza;
        }
        return BlockType::Park;
    }

    // Small blocks
    if norm_area < cfg.block_medium_threshold * 0.77 {
        return BlockType::Building;
    }

    // Default small blocks — rare plaza check
    if pr < cfg.block_entropy_sparse_plaza
        && (ha(cell.key ^ (block_idx as u32).wrapping_mul(0x6c62272e)) & 0xF == 0)
    {
        return BlockType::Plaza;
    }

    BlockType::Building
}

/// Split segments at their mutual intersection points.
fn split_at_intersections(
    vertices: &mut Vec<Point>,
    segments: &[PslgSegment],
    margin: f64,
) -> Vec<PslgSegment> {
    // For each segment, collect the t-values where other segments cross it
    let n = segments.len();
    let mut splits: Vec<Vec<f64>> = vec![vec![]; n];

    for i in 0..n {
        for j in (i + 1)..n {
            let a1 = vertices[segments[i].a];
            let b1 = vertices[segments[i].b];
            let a2 = vertices[segments[j].a];
            let b2 = vertices[segments[j].b];

            if let Some((t, u)) = segment_intersect(a1, b1, a2, b2) {
                if t > margin && t < 1.0 - margin && u > margin && u < 1.0 - margin {
                    let pt = Point::new(
                        a1.x + t * (b1.x - a1.x),
                        a1.y + t * (b1.y - a1.y),
                    );
                    let vi = find_or_insert_vertex(vertices, pt);

                    splits[i].push(t);
                    splits[j].push(u);
                    let _ = vi; // vertex already inserted
                }
            }
        }
    }

    // Rebuild segments, splitting at intersection points
    let mut result = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if splits[i].is_empty() {
            result.push(PslgSegment { a: seg.a, b: seg.b });
            continue;
        }

        let mut ts = splits[i].clone();
        ts.push(0.0);
        ts.push(1.0);
        ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-8);

        let a_pt = vertices[seg.a];
        let b_pt = vertices[seg.b];
        for w in ts.windows(2) {
            let p1 = Point::new(
                a_pt.x + w[0] * (b_pt.x - a_pt.x),
                a_pt.y + w[0] * (b_pt.y - a_pt.y),
            );
            let p2 = Point::new(
                a_pt.x + w[1] * (b_pt.x - a_pt.x),
                a_pt.y + w[1] * (b_pt.y - a_pt.y),
            );
            let vi_a = find_or_insert_vertex(vertices, p1);
            let vi_b = find_or_insert_vertex(vertices, p2);
            if vi_a != vi_b {
                result.push(PslgSegment { a: vi_a, b: vi_b });
            }
        }
    }

    result
}

/// Line segment intersection. Returns (t, u) if segments intersect.
fn segment_intersect(a1: Point, b1: Point, a2: Point, b2: Point) -> Option<(f64, f64)> {
    let d1x = b1.x - a1.x;
    let d1y = b1.y - a1.y;
    let d2x = b2.x - a2.x;
    let d2y = b2.y - a2.y;
    let cross = d1x * d2y - d1y * d2x;
    if cross.abs() < 1e-12 {
        return None;
    }
    let dx = a2.x - a1.x;
    let dy = a2.y - a1.y;
    let t = (dx * d2y - dy * d2x) / cross;
    let u = (dx * d1y - dy * d1x) / cross;
    if t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0 {
        Some((t, u))
    } else {
        None
    }
}

/// Extract faces from a PSLG using half-edge traversal.
fn extract_faces(
    vertices: &[Point],
    segments: &[PslgSegment],
    iter_limit: u32,
) -> Vec<Vec<Point>> {
    if segments.is_empty() || vertices.is_empty() {
        return vec![];
    }

    // Build half-edges
    let mut half_edges: Vec<HalfEdge> = Vec::new();
    for seg in segments {
        half_edges.push(HalfEdge {
            from: seg.a,
            to: seg.b,
            next: None,
            used: false,
        });
        half_edges.push(HalfEdge {
            from: seg.b,
            to: seg.a,
            next: None,
            used: false,
        });
    }

    // Build adjacency: for each vertex, collect outgoing half-edges sorted by angle
    let mut outgoing: HashMap<usize, Vec<usize>> = HashMap::new();
    for (he_idx, he) in half_edges.iter().enumerate() {
        outgoing.entry(he.from).or_default().push(he_idx);
    }

    // Sort outgoing edges at each vertex by angle
    for (vi, edges) in outgoing.iter_mut() {
        let v = vertices[*vi];
        edges.sort_by(|&a, &b| {
            let ha_to = vertices[half_edges[a].to];
            let hb_to = vertices[half_edges[b].to];
            let angle_a = (ha_to.y - v.y).atan2(ha_to.x - v.x);
            let angle_b = (hb_to.y - v.y).atan2(hb_to.x - v.x);
            angle_a.partial_cmp(&angle_b).unwrap()
        });
    }

    // Set next pointers: for half-edge A→B, find the twin B→A, then rotate
    // CW around B to find the next outgoing edge.
    for he_idx in 0..half_edges.len() {
        let to_vertex = half_edges[he_idx].to;
        let from_vertex = half_edges[he_idx].from;

        if let Some(out_edges) = outgoing.get(&to_vertex) {
            // Find the twin (B→A) in the outgoing list of B
            let reverse_angle = (vertices[from_vertex].y - vertices[to_vertex].y)
                .atan2(vertices[from_vertex].x - vertices[to_vertex].x);

            // Find the edge coming right after the reverse direction (CW rotation)
            let mut best_idx = None;
            let mut best_angle_diff = f64::NEG_INFINITY;

            for &out_he in out_edges {
                if half_edges[out_he].to == from_vertex {
                    continue; // Skip the twin itself
                }
                let out_to = vertices[half_edges[out_he].to];
                let out_angle =
                    (out_to.y - vertices[to_vertex].y)
                        .atan2(out_to.x - vertices[to_vertex].x);

                // We want the first edge CCW from the reverse direction
                let mut diff = out_angle - reverse_angle;
                if diff <= 0.0 {
                    diff += 2.0 * std::f64::consts::PI;
                }
                // Smallest positive diff = next CCW edge
                if best_idx.is_none() || diff < best_angle_diff {
                    best_angle_diff = diff;
                    best_idx = Some(out_he);
                }
            }

            half_edges[he_idx].next = best_idx;
        }
    }

    // Trace faces
    let mut faces: Vec<Vec<Point>> = Vec::new();
    for start in 0..half_edges.len() {
        if half_edges[start].used {
            continue;
        }

        let mut face = Vec::new();
        let mut current = start;
        let mut steps = 0;

        loop {
            if half_edges[current].used {
                break;
            }
            half_edges[current].used = true;
            face.push(vertices[half_edges[current].from]);

            match half_edges[current].next {
                Some(next) => {
                    current = next;
                    steps += 1;
                    if steps > iter_limit || current == start {
                        break;
                    }
                }
                None => break,
            }
        }

        if face.len() >= 3 && current == start {
            faces.push(face);
        }
    }

    faces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::cell::Cell;
    use crate::gen::district::generate_district;
    use crate::gen::roads::generate_roads;
    use crate::gen::rivers::generate_rivers;

    #[test]
    fn blocks_from_district() {
        let cell = Cell::from_octets(93, 184, 216);
        let district = generate_district(&cell);
        let roads = generate_roads(&district);
        let rivers =
            generate_rivers(&cell, &district.polygon.vertices);
        let blocks =
            extract_blocks(&cell, &district.polygon, &roads, &rivers);

        // Should produce at least one block
        assert!(
            !blocks.is_empty(),
            "No blocks extracted for 93.184.216.0"
        );

        // Each block should have a polygon with at least 3 vertices
        for block in &blocks {
            assert!(
                block.polygon.vertices.len() >= 3,
                "Block {} has {} vertices",
                block.idx,
                block.polygon.vertices.len()
            );
            assert!(block.area > 0.0);
        }
    }

    #[test]
    fn block_types_present() {
        // Test a variety of cells to see we get different block types
        let cells = [
            Cell::from_octets(1, 0, 0),
            Cell::from_octets(93, 184, 216),
            Cell::from_octets(255, 170, 85),
        ];

        for cell in &cells {
            let district = generate_district(cell);
            let roads = generate_roads(&district);
            let rivers =
                generate_rivers(cell, &district.polygon.vertices);
            let blocks =
                extract_blocks(cell, &district.polygon, &roads, &rivers);

            // Blocks should be non-empty for most cells
            // (some edge cases at boundaries might produce none)
            if !blocks.is_empty() {
                // At least one should be a building
                let has_building = blocks
                    .iter()
                    .any(|b| b.block_type == BlockType::Building);
                assert!(
                    has_building || blocks.len() < 2,
                    "No building blocks for {:?}",
                    cell.ip_prefix()
                );
            }
        }
    }

    #[test]
    fn blocks_deterministic() {
        let cell = Cell::from_octets(93, 184, 216);
        let district = generate_district(&cell);
        let roads = generate_roads(&district);
        let rivers =
            generate_rivers(&cell, &district.polygon.vertices);
        let b1 =
            extract_blocks(&cell, &district.polygon, &roads, &rivers);
        let b2 =
            extract_blocks(&cell, &district.polygon, &roads, &rivers);
        assert_eq!(b1.len(), b2.len());
    }

    #[test]
    fn classify_block_low_popcount() {
        let cell = Cell::from_octets(1, 0, 0);
        // Low popcount, large normalised area → water
        let bt = classify_block(&cell, 0, 3.0, false);
        assert_eq!(bt, BlockType::Water);

        // Low popcount, large, river adjacent → riverbank
        let bt2 = classify_block(&cell, 0, 3.0, true);
        assert_eq!(bt2, BlockType::Riverbank);
    }
}

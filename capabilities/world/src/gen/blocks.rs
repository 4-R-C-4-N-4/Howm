//! Block system: PSLG construction and face extraction.
//!
//! Once roads and rivers are placed, the remaining interior space is
//! subdivided into blocks — regions bounded by roads, rivers, and the
//! cell boundary. Each block is a polygon with a deterministic type and index.

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

// ═══════════════════════════════════════════════════════════════════════════
// PSLG types
// ═══════════════════════════════════════════════════════════════════════════

/// A directed half-edge in the PSLG.
#[derive(Debug, Clone)]
struct HalfEdge {
    from: usize,
    to: usize,
    next: Option<usize>,
    used: bool,
}

/// Snap a point to the block snap grid.
fn snap(p: Point) -> Point {
    let r = config().block_snap;
    Point::new((p.x / r).round() * r, (p.y / r).round() * r)
}

/// Find or insert a vertex in the vertex list (with snap tolerance).
fn find_or_insert(vertices: &mut Vec<Point>, p: Point) -> usize {
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

/// An undirected segment between two vertex indices.
#[derive(Debug, Clone)]
struct Seg {
    a: usize,
    b: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════

/// Build the PSLG from cell boundary, roads, and rivers.
/// Returns the extracted block faces.
pub fn extract_blocks(
    cell: &Cell,
    boundary: &Polygon,
    roads: &RoadNetwork,
    rivers: &[RiverSegment],
) -> Vec<Block> {
    let cfg = config();

    // ── Step 1: Collect ALL vertices first ──────────────────────────────
    //
    // We insert boundary vertices, road endpoints, and river polyline
    // vertices into a shared vertex list. Snap ensures near-coincident
    // points merge. This is critical: a road terminal that sits ON a
    // boundary edge must snap to the same grid as boundary vertices.
    let mut verts: Vec<Point> = Vec::new();

    // Boundary vertices
    let bv: Vec<usize> = boundary
        .vertices
        .iter()
        .map(|p| find_or_insert(&mut verts, *p))
        .collect();

    // Road endpoints (non-dead-end only)
    let mut road_endpoints: Vec<(usize, usize)> = Vec::new();
    for seg in &roads.segments {
        if seg.fate == RoadFate::DeadEnd {
            continue;
        }
        let a = find_or_insert(&mut verts, seg.a);
        let b = find_or_insert(&mut verts, seg.b);
        if a != b {
            road_endpoints.push((a, b));
        }
    }

    // River polyline vertices
    let mut river_segs: Vec<(usize, usize)> = Vec::new();
    for river in rivers {
        let polyline = river.to_polyline(8);
        for i in 0..polyline.len() - 1 {
            let a = find_or_insert(&mut verts, polyline[i]);
            let b = find_or_insert(&mut verts, polyline[i + 1]);
            if a != b {
                river_segs.push((a, b));
            }
        }
    }

    // ── Step 2: Build boundary chain, splitting at road/river vertices ──
    //
    // A road terminal sits ON a boundary edge. We need to split that
    // boundary edge into sub-segments at the terminal point. We do this
    // by walking each boundary edge and inserting any PSLG vertices that
    // lie on it (within snap tolerance).
    let mut segments: Vec<Seg> = Vec::new();

    for i in 0..bv.len() {
        let va = bv[i];
        let vb = bv[(i + 1) % bv.len()];
        let pa = verts[va];
        let pb = verts[vb];

        // Find all vertices that lie on this boundary edge
        let dx = pb.x - pa.x;
        let dy = pb.y - pa.y;
        let edge_len_sq = dx * dx + dy * dy;
        if edge_len_sq < 1e-20 {
            continue;
        }
        let edge_len = edge_len_sq.sqrt();
        // Normal for distance check
        let nx = -dy / edge_len;
        let ny = dx / edge_len;

        let mut on_edge: Vec<(f64, usize)> = Vec::new(); // (t, vertex_idx)
        on_edge.push((0.0, va));
        on_edge.push((1.0, vb));

        for (vi, v) in verts.iter().enumerate() {
            if vi == va || vi == vb {
                continue;
            }
            // Project onto edge
            let t = ((v.x - pa.x) * dx + (v.y - pa.y) * dy) / edge_len_sq;
            if t < 0.005 || t > 0.995 {
                continue; // Not interior to this edge
            }
            // Distance from edge line
            let dist = ((v.x - pa.x) * nx + (v.y - pa.y) * ny).abs();
            if dist < cfg.block_snap * 2.0 {
                on_edge.push((t, vi));
            }
        }

        // Sort by parameter and create sub-segments
        on_edge.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        on_edge.dedup_by(|a, b| a.1 == b.1);
        for w in on_edge.windows(2) {
            if w[0].1 != w[1].1 {
                segments.push(Seg {
                    a: w[0].1,
                    b: w[1].1,
                });
            }
        }
    }

    // Add road segments
    for (a, b) in &road_endpoints {
        segments.push(Seg { a: *a, b: *b });
    }

    // Add river segments
    for (a, b) in &river_segs {
        segments.push(Seg { a: *a, b: *b });
    }

    // ── Step 3: Find all segment-segment intersections and split ────────
    let segments = split_at_intersections(&mut verts, &segments, cfg.intersect_margin);

    // ── Step 4: Half-edge face extraction ───────────────────────────────
    let faces = extract_faces(&verts, &segments, cfg.block_face_iter_limit);

    // ── Step 5: Filter and classify blocks ──────────────────────────────
    let mut block_candidates: Vec<(Point, Polygon, f64, bool)> = Vec::new();

    for face in &faces {
        let poly = Polygon::new(face.clone());
        let area = poly.area();
        if area < cfg.block_min_area {
            continue;
        }
        let signed = poly.signed_area();
        if signed < 0.0 {
            // Negative signed area = exterior face (CW in our coord system), skip.
            // Interior block faces have positive signed area.
            continue;
        }

        let centroid = poly.centroid();

        // River adjacency
        let river_adj = !rivers.is_empty()
            && rivers.iter().any(|r| {
                let polyline = r.to_polyline(8);
                polyline.iter().any(|p| poly.contains(*p))
            });

        block_candidates.push((centroid, poly, area, river_adj));
    }

    if block_candidates.is_empty() {
        // Fallback: if face extraction finds nothing, use the whole district
        // as a single block. Better than returning empty.
        let area = boundary.area();
        let centroid = boundary.centroid();
        return vec![Block {
            idx: 0,
            polygon: boundary.clone(),
            block_type: classify_block(cell, 0, 1.0, false),
            area,
            centroid,
            river_adjacent: false,
        }];
    }

    // Compute median area for block typing
    let mut areas: Vec<f64> = block_candidates.iter().map(|c| c.2).collect();
    areas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_area = areas[areas.len() / 2];

    // Sort by centroid for stable indexing
    block_candidates.sort_by(|a, b| {
        let ka = a.0.x + a.0.y * 10000.0;
        let kb = b.0.x + b.0.y * 10000.0;
        ka.partial_cmp(&kb).unwrap()
    });

    let mut blocks: Vec<Block> = Vec::new();
    for (idx, (centroid, poly, area, river_adj)) in
        block_candidates.into_iter().enumerate()
    {
        let norm_area = area / median_area;
        let block_type = classify_block(cell, idx, norm_area, river_adj);

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

// ═══════════════════════════════════════════════════════════════════════════
// Block type classification
// ═══════════════════════════════════════════════════════════════════════════

fn classify_block(
    cell: &Cell,
    block_idx: usize,
    norm_area: f64,
    river_adjacent: bool,
) -> BlockType {
    let cfg = config();
    let pr = cell.popcount_ratio;

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

    if norm_area > cfg.block_medium_threshold {
        if pr < cfg.block_entropy_plaza {
            return BlockType::Plaza;
        }
        return BlockType::Park;
    }

    if norm_area < cfg.block_medium_threshold * 0.77 {
        return BlockType::Building;
    }

    if pr < cfg.block_entropy_sparse_plaza
        && (ha(cell.key ^ (block_idx as u32).wrapping_mul(0x6c62272e)) & 0xF == 0)
    {
        return BlockType::Plaza;
    }

    BlockType::Building
}

// ═══════════════════════════════════════════════════════════════════════════
// Segment intersection and splitting
// ═══════════════════════════════════════════════════════════════════════════

/// Line segment intersection. Returns (t, u) if segments intersect strictly.
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
    // Use a small epsilon to avoid exact endpoint hits which are
    // handled by vertex merging instead
    let eps = 1e-6;
    if t > eps && t < 1.0 - eps && u > eps && u < 1.0 - eps {
        Some((t, u))
    } else {
        None
    }
}

/// Split segments at their mutual intersection points.
fn split_at_intersections(
    vertices: &mut Vec<Point>,
    segments: &[Seg],
    margin: f64,
) -> Vec<Seg> {
    let n = segments.len();
    let mut splits: Vec<Vec<(f64, usize)>> = vec![vec![]; n];

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
                    let vi = find_or_insert(vertices, pt);
                    splits[i].push((t, vi));
                    splits[j].push((u, vi));
                }
            }
        }
    }

    let mut result = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if splits[i].is_empty() {
            result.push(Seg { a: seg.a, b: seg.b });
            continue;
        }

        let mut chain: Vec<(f64, usize)> = Vec::new();
        chain.push((0.0, seg.a));
        chain.extend_from_slice(&splits[i]);
        chain.push((1.0, seg.b));
        chain.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        chain.dedup_by(|a, b| a.1 == b.1);

        for w in chain.windows(2) {
            if w[0].1 != w[1].1 {
                result.push(Seg {
                    a: w[0].1,
                    b: w[1].1,
                });
            }
        }
    }

    result
}

// ═══════════════════════════════════════════════════════════════════════════
// Half-edge face extraction
// ═══════════════════════════════════════════════════════════════════════════

/// Extract faces from a PSLG using half-edge traversal (§9.3).
///
/// For each undirected edge, create two half-edges. At each vertex, sort
/// outgoing edges by angle. For half-edge A→B, the next edge is found by
/// looking at vertex B, finding the direction B→A (the reverse), then
/// taking the FIRST outgoing edge counter-clockwise from that reverse
/// direction. Following next pointers traces closed face polygons.
fn extract_faces(
    vertices: &[Point],
    segments: &[Seg],
    iter_limit: u32,
) -> Vec<Vec<Point>> {
    if segments.is_empty() || vertices.is_empty() {
        return vec![];
    }

    // Build half-edges (two per undirected segment)
    let mut half_edges: Vec<HalfEdge> = Vec::new();
    for seg in segments {
        // Forward
        half_edges.push(HalfEdge {
            from: seg.a,
            to: seg.b,
            next: None,
            used: false,
        });
        // Reverse
        half_edges.push(HalfEdge {
            from: seg.b,
            to: seg.a,
            next: None,
            used: false,
        });
    }

    // Build adjacency: for each vertex, collect outgoing half-edge indices
    let mut outgoing: HashMap<usize, Vec<usize>> = HashMap::new();
    for (he_idx, he) in half_edges.iter().enumerate() {
        outgoing.entry(he.from).or_default().push(he_idx);
    }

    // Sort outgoing edges at each vertex by angle (ascending)
    for (vi, edges) in outgoing.iter_mut() {
        let v = vertices[*vi];
        edges.sort_by(|&a, &b| {
            let ta = vertices[half_edges[a].to];
            let tb = vertices[half_edges[b].to];
            let aa = (ta.y - v.y).atan2(ta.x - v.x);
            let ab = (tb.y - v.y).atan2(tb.x - v.x);
            aa.partial_cmp(&ab).unwrap()
        });
    }

    // Set next pointers.
    //
    // For half-edge A→B, the NEXT half-edge in face traversal is:
    //   1. Look at vertex B's outgoing edges (sorted by angle).
    //   2. Find the index of the TWIN edge (B→A) in that sorted list.
    //   3. The next edge is the one BEFORE it in the sorted list (wrapping).
    //      This is the first edge clockwise from B→A, which traces the
    //      face to the LEFT of A→B.
    //
    // Why "before" (CW)? Because the sorted list is CCW. The edge just
    // before B→A in CCW order is the first edge CW from B→A. Following
    // the CW-next from each half-edge traces the interior (left) face.
    for he_idx in 0..half_edges.len() {
        let to_v = half_edges[he_idx].to;
        let from_v = half_edges[he_idx].from;

        if let Some(out_edges) = outgoing.get(&to_v) {
            // Find the twin (B→A) in B's outgoing list
            let twin_pos = out_edges
                .iter()
                .position(|&oe| half_edges[oe].to == from_v);

            if let Some(pos) = twin_pos {
                // Next = the edge BEFORE the twin in the sorted list (CW neighbor)
                let prev_pos = if pos == 0 {
                    out_edges.len() - 1
                } else {
                    pos - 1
                };
                let next_he = out_edges[prev_pos];
                // Don't set next to the twin itself (only happens if degree=1)
                if half_edges[next_he].to != from_v || out_edges.len() == 1 {
                    half_edges[he_idx].next = Some(next_he);
                }
            }
        }
    }

    // Trace faces by following next pointers
    let mut faces: Vec<Vec<Point>> = Vec::new();
    for start in 0..half_edges.len() {
        if half_edges[start].used {
            continue;
        }

        let mut face = Vec::new();
        let mut current = start;
        let mut steps = 0u32;

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

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

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
        let rivers = generate_rivers(&cell, &district.polygon.vertices);
        let blocks = extract_blocks(&cell, &district.polygon, &roads, &rivers);

        assert!(
            !blocks.is_empty(),
            "No blocks extracted for 93.184.216.0"
        );

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
    fn roads_create_multiple_blocks() {
        // Districts with multiple through-roads MUST produce multiple blocks
        let test_ips: &[(u8, u8, u8)] = &[
            (93, 184, 216),
            (255, 170, 85),
            (8, 8, 8),
            (128, 128, 128),
        ];

        for &(o1, o2, o3) in test_ips {
            let cell = Cell::from_octets(o1, o2, o3);
            let district = generate_district(&cell);
            let roads = generate_roads(&district);
            let rivers = generate_rivers(&cell, &district.polygon.vertices);

            let through_count = roads
                .segments
                .iter()
                .filter(|s| s.fate == RoadFate::Through)
                .count();

            let blocks = extract_blocks(&cell, &district.polygon, &roads, &rivers);

            if through_count >= 2 {
                assert!(
                    blocks.len() >= 2,
                    "{}.{}.{}: {} through-roads but only {} blocks",
                    o1, o2, o3, through_count, blocks.len()
                );
            }
        }
    }

    #[test]
    fn block_types_present() {
        let cells = [
            Cell::from_octets(1, 0, 0),
            Cell::from_octets(93, 184, 216),
            Cell::from_octets(255, 170, 85),
        ];

        for cell in &cells {
            let district = generate_district(cell);
            let roads = generate_roads(&district);
            let rivers = generate_rivers(cell, &district.polygon.vertices);
            let blocks = extract_blocks(cell, &district.polygon, &roads, &rivers);

            if !blocks.is_empty() {
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
        let rivers = generate_rivers(&cell, &district.polygon.vertices);
        let b1 = extract_blocks(&cell, &district.polygon, &roads, &rivers);
        let b2 = extract_blocks(&cell, &district.polygon, &roads, &rivers);
        assert_eq!(b1.len(), b2.len());
    }

    #[test]
    fn classify_block_low_popcount() {
        let cell = Cell::from_octets(1, 0, 0);
        let bt = classify_block(&cell, 0, 3.0, false);
        assert_eq!(bt, BlockType::Water);

        let bt2 = classify_block(&cell, 0, 3.0, true);
        assert_eq!(bt2, BlockType::Riverbank);
    }

    /// Simple test: a square boundary with one through-road should produce 2 blocks.
    #[test]
    fn simple_split_produces_two_faces() {
        let mut verts = Vec::new();

        // Square boundary: (0,0) → (100,0) → (100,100) → (0,100)
        let v0 = find_or_insert(&mut verts, Point::new(0.0, 0.0));
        let v1 = find_or_insert(&mut verts, Point::new(100.0, 0.0));
        let v2 = find_or_insert(&mut verts, Point::new(100.0, 100.0));
        let v3 = find_or_insert(&mut verts, Point::new(0.0, 100.0));

        // Road from midpoint of bottom edge to midpoint of top edge
        let v4 = find_or_insert(&mut verts, Point::new(50.0, 0.0));
        let v5 = find_or_insert(&mut verts, Point::new(50.0, 100.0));

        // Boundary segments (split at road terminals)
        let mut segs = vec![
            Seg { a: v0, b: v4 }, // bottom-left to road
            Seg { a: v4, b: v1 }, // road to bottom-right
            Seg { a: v1, b: v2 }, // right side
            Seg { a: v2, b: v5 }, // top-right to road
            Seg { a: v5, b: v3 }, // road to top-left
            Seg { a: v3, b: v0 }, // left side
            // Road segment
            Seg { a: v4, b: v5 },
        ];

        let faces = extract_faces(&verts, &segs, 100);

        // Filter out exterior face
        let interior: Vec<_> = faces
            .iter()
            .filter(|f| {
                let p = Polygon::new(f.to_vec());
                p.signed_area() > 0.0 && p.area() > 10.0
            })
            .collect();

        assert_eq!(
            interior.len(),
            2,
            "One road splitting a square should produce exactly 2 interior faces, got {}",
            interior.len()
        );
    }
}

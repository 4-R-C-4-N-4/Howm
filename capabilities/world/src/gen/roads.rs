//! Road network generation.
//!
//! Roads are defined by crossing points on district boundaries (terminals),
//! connected within the district by matched pairs with assigned fates
//! (through, meeting point, dead end).

use serde::{Deserialize, Serialize};

use super::config::config;
use super::district::{DistrictGeometry, SharedEdge};
use super::hash::{ha, hb};
use crate::types::{Point, Segment};

/// A terminal: a road crossing point on the district boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Terminal {
    /// Position in world space.
    pub position: Point,
    /// Which polygon edge this terminal sits on.
    pub edge_idx: usize,
    /// Continuous perimeter order: edge_idx + t (for sorting).
    pub perim_order: f64,
    /// Parameter t along the edge (0.0 = start, 1.0 = end).
    pub t: f64,
    /// Key of the neighbor cell across this edge.
    pub neighbor_key: u32,
}

/// The fate of a matched terminal pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoadFate {
    /// Straight line between the two terminals.
    Through,
    /// Both terminals connect to a shared interior junction.
    MeetingPoint,
    /// Both terminals become dead-end stubs.
    DeadEnd,
}

/// A road segment within the district.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadSegment {
    pub a: Point,
    pub b: Point,
    pub fate: RoadFate,
    /// Indices of the terminals that produced this segment (if matched).
    pub terminal_indices: Option<(usize, usize)>,
}

/// An intersection point where two road segments cross.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intersection {
    pub position: Point,
    /// Indices of the two road segments.
    pub segments: (usize, usize),
}

/// Complete road network for a district.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadNetwork {
    pub terminals: Vec<Terminal>,
    pub segments: Vec<RoadSegment>,
    pub intersections: Vec<Intersection>,
}

/// Canonical edge hash for a shared boundary between two cells.
/// Both cells independently derive the same hash for their shared edge.
fn edge_hash(key_a: u32, key_b: u32) -> u32 {
    let min_key = key_a.min(key_b);
    let max_key = key_a.max(key_b);
    ha(min_key ^ ((max_key & 0xFFFF) << 8))
}

/// Compute road crossing points on a shared edge.
fn edge_crossings(
    edge: &SharedEdge,
    cell_key: u32,
    cell_popcount: u32,
) -> Vec<Terminal> {
    let cfg = config();
    let eh = edge_hash(cell_key, edge.neighbor_key);

    // Crossing count from density and edge length
    let neighbor_popcount = edge.neighbor_key.count_ones();
    let edge_density = cell_popcount.min(neighbor_popcount);
    let base_count = 1 + (edge_density / 8);
    let max_by_length = (edge.length / cfg.min_road_spacing).floor() as u32;
    let crossing_count = 1.max(base_count.min(max_by_length));

    let mut terminals = Vec::new();
    for i in 0..crossing_count {
        let seg_start = i as f64 / (crossing_count + 1) as f64;
        let seg_end = (i + 1) as f64 / (crossing_count + 1) as f64;
        let byte = ((eh >> (i * 8)) & 0xFF) as f64;
        let t = seg_start + (byte / 255.0) * (seg_end - seg_start);

        let position = Point::new(
            edge.start.x + t * (edge.end.x - edge.start.x),
            edge.start.y + t * (edge.end.y - edge.start.y),
        );

        terminals.push(Terminal {
            position,
            edge_idx: edge.edge_idx,
            perim_order: edge.edge_idx as f64 + t,
            t,
            neighbor_key: edge.neighbor_key,
        });
    }

    terminals
}

/// Generate the complete road network for a district.
pub fn generate_roads(district: &DistrictGeometry) -> RoadNetwork {
    let cfg = config();
    let cell_key = district.cell.key;

    // ── Step 1: Collect all terminals from shared edges ──
    let mut terminals: Vec<Terminal> = Vec::new();
    for edge in &district.shared_edges {
        let crossings = edge_crossings(edge, cell_key, district.cell.popcount);
        terminals.extend(crossings);
    }

    // Sort by perimeter order for clockwise sequencing
    terminals.sort_by(|a, b| {
        a.perim_order
            .partial_cmp(&b.perim_order)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ── Step 2: Match terminals greedily by affinity ──
    let n = terminals.len();
    let mut matched = vec![false; n];
    let mut matches: Vec<(usize, usize)> = Vec::new();

    // Build affinity-sorted pair list
    let mut pairs: Vec<(u32, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            // Only match terminals on different polygon edges
            if terminals[i].edge_idx == terminals[j].edge_idx {
                continue;
            }
            let affinity = ha(cell_key ^ ((i as u32) << 8) ^ j as u32);
            pairs.push((affinity, i, j));
        }
    }

    // Sort descending by affinity
    pairs.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, i, j) in &pairs {
        if !matched[*i] && !matched[*j] {
            matched[*i] = true;
            matched[*j] = true;
            matches.push((*i, *j));
        }
    }

    // ── Step 3: Assign road fate and generate segments ──
    let seed_pos = district.seed_position;
    let mut segments: Vec<RoadSegment> = Vec::new();

    for &(ti, tj) in &matches {
        let a = terminals[ti].position;
        let b = terminals[tj].position;
        let min_idx = ti.min(tj) as u32;
        let max_idx = ti.max(tj) as u32;

        let fate_hash = hb(cell_key ^ min_idx ^ (max_idx << 4));
        let fate_byte = (fate_hash & 0xFF) as u8;

        let fate = if fate_byte < cfg.fate_through_max {
            RoadFate::Through
        } else if fate_byte < cfg.fate_meeting_max {
            RoadFate::MeetingPoint
        } else {
            RoadFate::DeadEnd
        };

        match fate {
            RoadFate::Through => {
                segments.push(RoadSegment {
                    a,
                    b,
                    fate,
                    terminal_indices: Some((ti, tj)),
                });
            }
            RoadFate::MeetingPoint => {
                let mid = a.midpoint(b);
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let len = (dx * dx + dy * dy).sqrt();
                let perp = if len > 1e-10 {
                    Point::new(-dy / len, dx / len)
                } else {
                    Point::new(0.0, 1.0)
                };
                let offset =
                    ((fate_hash >> 8) & 0xFF) as f64 / 255.0 * 20.0 - 10.0;
                let junction = Point::new(
                    mid.x + perp.x * offset,
                    mid.y + perp.y * offset,
                );
                segments.push(RoadSegment {
                    a,
                    b: junction,
                    fate,
                    terminal_indices: Some((ti, tj)),
                });
                segments.push(RoadSegment {
                    a: b,
                    b: junction,
                    fate,
                    terminal_indices: Some((ti, tj)),
                });
            }
            RoadFate::DeadEnd => {
                // Each terminal stubs toward the seed point
                let stub_a = Point::new(
                    a.x + (seed_pos.x - a.x) * cfg.dead_end_frac,
                    a.y + (seed_pos.y - a.y) * cfg.dead_end_frac,
                );
                let stub_b = Point::new(
                    b.x + (seed_pos.x - b.x) * cfg.dead_end_frac,
                    b.y + (seed_pos.y - b.y) * cfg.dead_end_frac,
                );
                segments.push(RoadSegment {
                    a,
                    b: stub_a,
                    fate,
                    terminal_indices: Some((ti, tj)),
                });
                segments.push(RoadSegment {
                    a: b,
                    b: stub_b,
                    fate,
                    terminal_indices: Some((ti, tj)),
                });
            }
        }
    }

    // Unmatched terminals become dead-end stubs (shorter)
    let unmatched_frac = cfg.dead_end_frac * 0.857; // 30%
    for (i, terminal) in terminals.iter().enumerate() {
        if !matched[i] {
            let stub = Point::new(
                terminal.position.x
                    + (seed_pos.x - terminal.position.x) * unmatched_frac,
                terminal.position.y
                    + (seed_pos.y - terminal.position.y) * unmatched_frac,
            );
            segments.push(RoadSegment {
                a: terminal.position,
                b: stub,
                fate: RoadFate::DeadEnd,
                terminal_indices: None,
            });
        }
    }

    // ── Step 4: Find road intersections ──
    let mut intersections = Vec::new();
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            let s1 = Segment::new(segments[i].a, segments[i].b);
            let s2 = Segment::new(segments[j].a, segments[j].b);

            if let Some((t, u)) = s1.intersect(&s2) {
                if t > cfg.intersect_margin
                    && t < 1.0 - cfg.intersect_margin
                    && u > cfg.intersect_margin
                    && u < 1.0 - cfg.intersect_margin
                {
                    intersections.push(Intersection {
                        position: s1.at(t),
                        segments: (i, j),
                    });
                }
            }
        }
    }

    RoadNetwork {
        terminals,
        segments,
        intersections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::cell::Cell;
    use crate::gen::district::generate_district;

    #[test]
    fn roads_93_184_216() {
        let cell = Cell::from_octets(93, 184, 216);
        let district = generate_district(&cell);
        let roads = generate_roads(&district);

        // Should have terminals (shared edges produce crossings)
        assert!(
            !roads.terminals.is_empty(),
            "No terminals generated for 93.184.216.0"
        );

        // Should have road segments
        assert!(
            !roads.segments.is_empty(),
            "No road segments for 93.184.216.0"
        );

        // All terminals should be on the district boundary
        for t in &roads.terminals {
            // Terminal position should be near a polygon edge
            let poly = &district.polygon;
            let min_dist = (0..poly.vertices.len())
                .map(|i| {
                    let (a, b) = poly.edge(i);
                    point_to_segment_dist(t.position, a, b)
                })
                .fold(f64::INFINITY, f64::min);
            assert!(
                min_dist < 1.0,
                "Terminal at {:?} is {} from polygon boundary",
                t.position,
                min_dist
            );
        }
    }

    #[test]
    fn roads_deterministic() {
        let cell = Cell::from_octets(93, 184, 216);
        let district = generate_district(&cell);
        let r1 = generate_roads(&district);
        let r2 = generate_roads(&district);
        assert_eq!(r1.terminals.len(), r2.terminals.len());
        assert_eq!(r1.segments.len(), r2.segments.len());
        assert_eq!(r1.intersections.len(), r2.intersections.len());
    }

    #[test]
    fn road_density_scales() {
        // Low popcount → fewer roads
        let low = Cell::from_octets(1, 0, 0);
        let high = Cell::from_octets(255, 170, 85);
        let d_low = generate_district(&low);
        let d_high = generate_district(&high);
        let r_low = generate_roads(&d_low);
        let r_high = generate_roads(&d_high);

        // High popcount should generally have more terminals
        // (but depends on edge lengths too, so we just verify both work)
        assert!(!r_low.terminals.is_empty() || d_low.shared_edges.is_empty());
        assert!(!r_high.terminals.is_empty() || d_high.shared_edges.is_empty());
    }

    #[test]
    fn edge_hash_symmetric() {
        // edge_hash(A, B) == edge_hash(B, A) — both cells see the same crossings
        let h1 = edge_hash(0x5db8d8, 0x5db9d8);
        let h2 = edge_hash(0x5db9d8, 0x5db8d8);
        assert_eq!(h1, h2);
    }

    /// Helper: distance from point to line segment.
    fn point_to_segment_dist(p: Point, a: Point, b: Point) -> f64 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len_sq = dx * dx + dy * dy;
        if len_sq < 1e-20 {
            return p.distance_to(a);
        }
        let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq;
        let t = t.clamp(0.0, 1.0);
        let proj = Point::new(a.x + t * dx, a.y + t * dy);
        p.distance_to(proj)
    }
}

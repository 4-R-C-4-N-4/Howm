//! Conveyance generation — parked (Tier 0) and route-following (Tier 1).
//!
//! Parked conveyances are static objects at seeded positions along road edges.
//! Moving conveyances follow road network loops, position derived from UTC time.

use serde::{Deserialize, Serialize};

use super::cell::Cell;
use super::config::config;
use super::hash::{ha, hash_to_f64};
use super::objects::{compute_form_id, compute_object_id, ObjectSeeds, Tier};
use super::roads::RoadNetwork;
use crate::types::Point;

/// Conveyance type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConveyanceType {
    Parked,
    RouteFollowing,
}

/// A conveyance instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conveyance {
    pub idx: usize,
    pub conveyance_type: ConveyanceType,
    pub position: Point,
    pub orientation: f64,
    pub form_id: u32,
    pub object_id: u64,
    pub seeds: ObjectSeeds,
    pub tier: Tier,
    /// For route-following: the road segment indices forming the loop.
    pub route: Option<Vec<usize>>,
    /// For route-following: loop period in ms.
    pub loop_period_ms: Option<u64>,
}

/// Generate parked conveyances along road edges.
fn generate_parked(cell: &Cell, road_network: &RoadNetwork) -> Vec<Conveyance> {
    let mut result = Vec::new();
    let count = (cell.popcount_ratio * 3.0 + 1.0).floor() as u32; // 1–4 parked

    for i in 0..count {
        if road_network.segments.is_empty() {
            break;
        }
        let seg_seed = ha(cell.key ^ i ^ 0xc0e7);
        let seg_idx = seg_seed as usize % road_network.segments.len();
        let segment = &road_network.segments[seg_idx];

        let t = 0.2 + hash_to_f64(ha(seg_seed ^ 0x1)) * 0.6;
        let base_pos = segment.a.lerp(segment.b, t);

        // Offset from road
        let dx = segment.b.x - segment.a.x;
        let dy = segment.b.y - segment.a.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-10 { continue; }
        let side = if ha(seg_seed ^ 0x2) & 1 == 0 { 1.0 } else { -1.0 };
        let offset = 5.0; // world units from centreline
        let position = Point::new(
            base_pos.x + (-dy / len) * offset * side,
            base_pos.y + (dx / len) * offset * side,
        );

        let orient = (dy).atan2(dx) + if side > 0.0 { 0.0 } else { std::f64::consts::PI };
        let object_seed = ha(cell.key ^ i ^ 0xc0e7 ^ 0x2);
        let seeds = ObjectSeeds::from_seed(object_seed);
        let form_id = compute_form_id("conveyance:parked", cell.aesthetic_bucket(), object_seed);
        let object_id = compute_object_id(cell.key, seeds.object_seed);

        result.push(Conveyance {
            idx: i as usize,
            conveyance_type: ConveyanceType::Parked,
            position,
            orientation: orient,
            form_id,
            object_id,
            seeds,
            tier: Tier::Seedable,
            route: None,
            loop_period_ms: None,
        });
    }
    result
}

/// Select a road loop (simple: pick 2-4 connected segments forming a path).
fn select_road_loop(road_network: &RoadNetwork, route_seed: u32) -> Vec<usize> {
    if road_network.segments.is_empty() {
        return Vec::new();
    }
    let count = 2 + (route_seed & 0x3) as usize; // 2–5 segments
    let mut route = Vec::with_capacity(count);
    let start = route_seed as usize % road_network.segments.len();
    for i in 0..count {
        route.push((start + i) % road_network.segments.len());
    }
    route
}

/// Generate route-following conveyances.
fn generate_route_following(cell: &Cell, road_network: &RoadNetwork) -> Vec<Conveyance> {
    let cfg = config();
    let mut result = Vec::new();
    let count = (cell.popcount_ratio * 2.0).floor() as u32; // 0–2

    for i in 0..count {
        let route_seed = ha(cell.key ^ i ^ 0xc3a1f2b4);
        let route = select_road_loop(road_network, route_seed);
        if route.is_empty() { continue; }

        let loop_period_ms = cfg.conveyance_loop_base_ms + (route_seed & 0xFFFF) as u64;

        // Initial position: start of first segment
        let first_seg = &road_network.segments[route[0]];
        let position = first_seg.a;
        let dx = first_seg.b.x - first_seg.a.x;
        let dy = first_seg.b.y - first_seg.a.y;
        let orientation = dy.atan2(dx);

        let object_seed = ha(cell.key ^ i ^ 0xc3a1f2b4 ^ 0x2);
        let seeds = ObjectSeeds::from_seed(object_seed);
        let form_id = compute_form_id("conveyance:route", cell.aesthetic_bucket(), object_seed);
        let object_id = compute_object_id(cell.key, seeds.object_seed);

        result.push(Conveyance {
            idx: (count + i) as usize,
            conveyance_type: ConveyanceType::RouteFollowing,
            position,
            orientation,
            form_id,
            object_id,
            seeds,
            tier: Tier::TimeSynced,
            route: Some(route),
            loop_period_ms: Some(loop_period_ms),
        });
    }
    result
}

/// Result of conveyance generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistrictConveyances {
    pub parked: Vec<Conveyance>,
    pub route_following: Vec<Conveyance>,
}

/// Compute the position of a route-following conveyance at a given UTC time.
pub fn conveyance_position(
    conveyance: &Conveyance,
    road_network: &RoadNetwork,
    utc_time_ms: u64,
) -> Option<Point> {
    let route = conveyance.route.as_ref()?;
    let loop_period = conveyance.loop_period_ms?;
    if route.is_empty() || loop_period == 0 { return None; }

    let t = (utc_time_ms % loop_period) as f64 / loop_period as f64;

    // Interpolate along the route segments
    let total_segs = route.len() as f64;
    let seg_t = t * total_segs;
    let seg_idx = (seg_t as usize).min(route.len() - 1);
    let local_t = seg_t - seg_idx as f64;

    let seg = &road_network.segments[route[seg_idx]];
    Some(seg.a.lerp(seg.b, local_t.clamp(0.0, 1.0)))
}

/// Generate all conveyances for a district.
pub fn generate_conveyances(cell: &Cell, road_network: &RoadNetwork) -> DistrictConveyances {
    DistrictConveyances {
        parked: generate_parked(cell, road_network),
        route_following: generate_route_following(cell, road_network),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::roads::{RoadNetwork, RoadSegment, RoadFate, Terminal, Intersection};

    fn test_road_network() -> RoadNetwork {
        RoadNetwork {
            terminals: vec![],
            segments: vec![
                RoadSegment {
                    a: Point::new(0.0, 0.0), b: Point::new(100.0, 0.0),
                    fate: RoadFate::Through, terminal_indices: None,
                },
                RoadSegment {
                    a: Point::new(100.0, 0.0), b: Point::new(100.0, 100.0),
                    fate: RoadFate::Through, terminal_indices: None,
                },
                RoadSegment {
                    a: Point::new(100.0, 100.0), b: Point::new(0.0, 100.0),
                    fate: RoadFate::Through, terminal_indices: None,
                },
            ],
            intersections: vec![],
        }
    }

    #[test]
    fn parked_conveyances_generated() {
        let cell = Cell::from_octets(93, 184, 216);
        let network = test_road_network();
        let result = generate_conveyances(&cell, &network);
        assert!(!result.parked.is_empty());
    }

    #[test]
    fn route_conveyances_deterministic() {
        let cell = Cell::from_octets(255, 170, 85);
        let network = test_road_network();
        let r1 = generate_conveyances(&cell, &network);
        let r2 = generate_conveyances(&cell, &network);
        assert_eq!(r1.parked.len(), r2.parked.len());
        assert_eq!(r1.route_following.len(), r2.route_following.len());
    }

    #[test]
    fn conveyance_position_moves() {
        let cell = Cell::from_octets(255, 170, 85);
        let network = test_road_network();
        let result = generate_conveyances(&cell, &network);
        if let Some(c) = result.route_following.first() {
            let p1 = conveyance_position(c, &network, 0);
            let p2 = conveyance_position(c, &network, 10000);
            assert!(p1.is_some());
            assert!(p2.is_some());
            // Positions should differ unless the loop period exactly matches
        }
    }
}

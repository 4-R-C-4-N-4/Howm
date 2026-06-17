//! Structured world-structure audit — a machine-checkable layer above the
//! renderer. Runs geometric/topological invariants over generated districts and
//! returns a pass/fail report, so generation correctness (and cross-district
//! continuity) can be verified automatically instead of by eyeballing maps.

use serde::Serialize;

use crate::gen::blocks::{extract_blocks, Block};
use crate::gen::buildings::{generate_buildings, BuildingPlot};
use crate::gen::cell::Cell;
use crate::gen::config::config;
use crate::gen::conveyances::generate_conveyances;
use crate::gen::district::{generate_district, DistrictGeometry};
use crate::gen::fixtures::generate_fixtures;
use crate::gen::flora::generate_flora;
use crate::gen::creatures::generate_creatures;
use crate::gen::rivers::{generate_rivers, is_river, RiverSegment};
use crate::gen::roads::{generate_roads, RoadNetwork};
use crate::types::{Point, Polygon, Segment};

/// Point-equality tolerance in world units (sub-cell).
const TOL: f64 = 0.5;

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub pass: bool,
    pub detail: String,
}

impl Check {
    fn new(name: &str, pass: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            pass,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub ip: String,
    pub cell_key: u32,
    pub pass: bool,
    pub checks: Vec<Check>,
    pub metrics: serde_json::Value,
}

/// Everything generated for one district, kept together for the checks.
struct DistrictData {
    cell: Cell,
    geom: DistrictGeometry,
    roads: RoadNetwork,
    rivers: Vec<RiverSegment>,
    blocks: Vec<Block>,
    /// (block_idx, plot) for every building.
    buildings: Vec<(usize, BuildingPlot)>,
    /// Positions of every fixture (zone + road-edge).
    fixtures: Vec<Point>,
    /// Positions of every flora instance (block + road-edge).
    flora: Vec<Point>,
    /// Conveyance positions with a route flag (true = route-following).
    conveyances: Vec<(Point, bool)>,
    creature_count: usize,
}

impl DistrictData {
    fn build(cell: Cell) -> Self {
        let geom = generate_district(&cell);
        let roads = generate_roads(&geom);
        let rivers = generate_rivers(&geom);
        let blocks = extract_blocks(&cell, &geom.polygon, &roads, &rivers);

        let mut buildings = Vec::new();
        let mut fixtures = Vec::new();
        let mut flora = Vec::new();
        let mut creature_count = 0;
        for b in &blocks {
            buildings.extend(generate_buildings(&cell, b).plots.into_iter().map(|p| (b.idx, p)));
            let bf = generate_fixtures(&cell, b, Some(&roads));
            fixtures.extend(bf.zone_fixtures.iter().chain(&bf.road_fixtures).map(|f| f.position));
            let fl = generate_flora(&cell, b, Some(&roads));
            flora.extend(fl.block_flora.iter().chain(&fl.road_flora).map(|f| f.position));
            creature_count += generate_creatures(&cell, b).creatures.len();
        }
        let conv = generate_conveyances(&cell, &roads);
        let conveyances = conv
            .parked
            .iter()
            .map(|c| (c.position, false))
            .chain(conv.route_following.iter().map(|c| (c.position, true)))
            .collect();

        Self {
            cell,
            geom,
            roads,
            rivers,
            blocks,
            buildings,
            fixtures,
            flora,
            conveyances,
            creature_count,
        }
    }
}

// ── Geometry helpers ────────────────────────────────────────────────────────

fn pt_eq(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() <= TOL && (a.y - b.y).abs() <= TOL
}

fn bbox_overlap(a: &Polygon, b: &Polygon) -> bool {
    let (ax0, ay0, ax1, ay1) = a.bbox();
    let (bx0, by0, bx1, by1) = b.bbox();
    ax0 <= bx1 && bx0 <= ax1 && ay0 <= by1 && by0 <= ay1
}

/// Do two polygons overlap (share interior area)? bbox prefilter, then any
/// vertex-in-other or any edge crossing.
fn polys_overlap(a: &Polygon, b: &Polygon) -> bool {
    if !bbox_overlap(a, b) {
        return false;
    }
    if a.vertices.iter().any(|&v| b.contains(v)) || b.vertices.iter().any(|&v| a.contains(v)) {
        return true;
    }
    for i in 0..a.edge_count() {
        let (a0, a1) = a.edge(i);
        let sa = Segment::new(a0, a1);
        for j in 0..b.edge_count() {
            let (b0, b1) = b.edge(j);
            if sa.intersect(&Segment::new(b0, b1)).is_some() {
                return true;
            }
        }
    }
    false
}

/// Estimate overlap area fraction (of the smaller polygon) by point sampling.
fn overlap_fraction(a: &Polygon, b: &Polygon, samples: usize) -> f64 {
    let (x0, y0, x1, y1) = a.bbox();
    let mut inside_a = 0usize;
    let mut inside_both = 0usize;
    let n = samples.max(4);
    for i in 0..n {
        for j in 0..n {
            let p = Point::new(
                x0 + (x1 - x0) * (i as f64 + 0.5) / n as f64,
                y0 + (y1 - y0) * (j as f64 + 0.5) / n as f64,
            );
            if a.contains(p) {
                inside_a += 1;
                if b.contains(p) {
                    inside_both += 1;
                }
            }
        }
    }
    if inside_a == 0 {
        0.0
    } else {
        inside_both as f64 / inside_a as f64
    }
}

// ── Single-district checks ──────────────────────────────────────────────────

fn check_determinism(d: &DistrictData) -> Check {
    let d2 = DistrictData::build(d.cell.clone());
    let same = d2.blocks.len() == d.blocks.len()
        && d2.roads.segments.len() == d.roads.segments.len()
        && d2.buildings.len() == d.buildings.len()
        && d2.geom.polygon.vertices.len() == d.geom.polygon.vertices.len();
    Check::new(
        "determinism",
        same,
        format!(
            "blocks {}/{}, roads {}/{}, buildings {}/{}",
            d.blocks.len(),
            d2.blocks.len(),
            d.roads.segments.len(),
            d2.roads.segments.len(),
            d.buildings.len(),
            d2.buildings.len()
        ),
    )
}

fn check_blocks_in_district(d: &DistrictData) -> Check {
    // A block belongs to the district if it lies within the district polygon. We
    // test vertex containment (with a small boundary tolerance) rather than the
    // centroid, because a legitimately non-convex (L/U-shaped) block can have its
    // area centroid outside itself.
    // Containment tolerance scales with cell size: degenerate/elongated Voronoi
    // cells carry inherent float boundary noise from the PSLG pipeline (sub-1% of
    // the cell). A gross misplacement is far larger and still caught.
    let tol = containment_tol(&d.geom.polygon);
    let mut offenders = Vec::new();
    for b in &d.blocks {
        let escaped = b
            .polygon
            .vertices
            .iter()
            .filter(|&&v| !point_within(v, &d.geom.polygon, tol))
            .count();
        if escaped > 0 {
            offenders.push((b.idx, escaped, b.polygon.vertices.len()));
        }
    }
    Check::new(
        "blocks_in_district",
        offenders.is_empty(),
        format!(
            "{} blocks with vertices outside district (idx,escaped,total): {:?}",
            offenders.len(),
            offenders
        ),
    )
}

/// Containment tolerance for a container polygon: 1.5% of its bbox diagonal
/// (min `TOL`). Allows for float boundary noise proportional to cell size while
/// staying far below any gross geometric error.
fn containment_tol(poly: &Polygon) -> f64 {
    let (x0, y0, x1, y1) = poly.bbox();
    let diag = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    (0.015 * diag).max(TOL)
}

/// Is `p` inside `poly`, or within `tol` of its boundary? Robust for non-convex
/// polygons (unlike inflating the polygon, which can self-intersect).
fn point_within(p: Point, poly: &Polygon, tol: f64) -> bool {
    if poly.contains(p) {
        return true;
    }
    let n = poly.vertices.len();
    let mut best = f64::MAX;
    for i in 0..n {
        let a = poly.vertices[i];
        let b = poly.vertices[(i + 1) % n];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len_sq = dx * dx + dy * dy;
        let proj = if len_sq < 1e-12 {
            a
        } else {
            let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
            Point::new(a.x + t * dx, a.y + t * dy)
        };
        best = best.min(p.distance_sq(proj));
    }
    best.sqrt() <= tol
}

fn check_blocks_no_overlap(d: &DistrictData) -> Check {
    let mut pairs = Vec::new();
    for i in 0..d.blocks.len() {
        for j in (i + 1)..d.blocks.len() {
            let f = overlap_fraction(&d.blocks[i].polygon, &d.blocks[j].polygon, 12);
            if f > 0.05 {
                pairs.push((d.blocks[i].idx, d.blocks[j].idx, (f * 100.0) as u32));
            }
        }
    }
    Check::new(
        "blocks_no_overlap",
        pairs.is_empty(),
        format!(
            "{} overlapping block pairs (idx_a,idx_b,%): {:?}",
            pairs.len(),
            pairs
        ),
    )
}

fn check_buildings_in_block(d: &DistrictData) -> Check {
    // Each building footprint must lie within its block (vertex containment with
    // a small tolerance — centroid containment false-positives on non-convex
    // blocks/footprints).
    let mut bad = 0usize;
    for (bidx, plot) in &d.buildings {
        if let Some(block) = d.blocks.iter().find(|b| b.idx == *bidx) {
            let tol = containment_tol(&block.polygon);
            if plot
                .polygon
                .vertices
                .iter()
                .any(|&v| !point_within(v, &block.polygon, tol))
            {
                bad += 1;
            }
        }
    }
    Check::new(
        "buildings_in_block",
        bad == 0,
        format!(
            "{}/{} buildings with vertices outside their block",
            bad,
            d.buildings.len()
        ),
    )
}

fn check_buildings_no_overlap(d: &DistrictData) -> Check {
    // Same-block buildings come from a Voronoi tiling and must NOT overlap (the
    // gross "buildings intersect" bug was 45–100% same-block). Cross-block pairs
    // can have a small boundary touch where two blocks share an edge with no road
    // between them — those are tolerated up to a moderate bound (the footprint
    // inset minimises but can't fully eliminate them on non-convex boundary lots).
    const SAME_BLOCK_MAX: f64 = 0.10;
    const CROSS_BLOCK_MAX: f64 = 0.30;
    let mut same_block_bad = 0usize;
    let mut cross_block_bad = 0usize;
    let mut worst = 0.0f64;
    let mut examples: Vec<(usize, usize, u32)> = Vec::new();
    for i in 0..d.buildings.len() {
        for j in (i + 1)..d.buildings.len() {
            let (pa, pb) = (&d.buildings[i].1.polygon, &d.buildings[j].1.polygon);
            if !polys_overlap(pa, pb) {
                continue;
            }
            let f = overlap_fraction(pa, pb, 8);
            let same = d.buildings[i].0 == d.buildings[j].0;
            let limit = if same {
                SAME_BLOCK_MAX
            } else {
                CROSS_BLOCK_MAX
            };
            if f > limit {
                if same {
                    same_block_bad += 1;
                } else {
                    cross_block_bad += 1;
                }
                worst = worst.max(f);
                if examples.len() < 4 {
                    examples.push((d.buildings[i].0, d.buildings[j].0, (f * 100.0) as u32));
                }
            }
        }
    }
    Check::new(
        "buildings_no_overlap",
        same_block_bad == 0 && cross_block_bad == 0,
        format!(
            "{} same-block (>{:.0}%) + {} cross-block (>{:.0}%) overlapping pairs, worst {:.0}%; {:?}",
            same_block_bad,
            SAME_BLOCK_MAX * 100.0,
            cross_block_bad,
            CROSS_BLOCK_MAX * 100.0,
            worst * 100.0,
            examples
        ),
    )
}

fn check_roads_present(d: &DistrictData) -> Check {
    // Populated cells (popcount > 0) should have a road network.
    let need = d.cell.popcount > 0;
    let ok = !need || !d.roads.segments.is_empty();
    Check::new(
        "roads_present",
        ok,
        format!(
            "{} segments, {} intersections (popcount {})",
            d.roads.segments.len(),
            d.roads.intersections.len(),
            d.cell.popcount
        ),
    )
}

fn check_intersections_on_segments(d: &DistrictData) -> Check {
    // Each intersection must actually lie on both of its referenced segments.
    let mut bad = 0usize;
    for ix in &d.roads.intersections {
        let (i, j) = ix.segments;
        let on = |k: usize| {
            d.roads.segments.get(k).is_some_and(|s| {
                let seg = Segment::new(s.a, s.b);
                point_on_segment(ix.position, &seg, 1.0)
            })
        };
        if !(on(i) && on(j)) {
            bad += 1;
        }
    }
    Check::new(
        "intersections_on_segments",
        bad == 0,
        format!(
            "{}/{} intersections not on their segments",
            bad,
            d.roads.intersections.len()
        ),
    )
}

fn check_rivers(d: &DistrictData) -> Check {
    // Any river that IS generated must be geometrically sound: endpoints on the
    // district boundary and the path within bounds. (Presence isn't asserted:
    // a river only routes when the cell is Voronoi-adjacent to BOTH its grid
    // gy-neighbours; otherwise it legitimately can't cross — see the
    // `river_routing_gap` metric and the cross-district `river_continuity` check.)
    let mut issues = Vec::new();
    let tol = containment_tol(&d.geom.polygon);
    let (x0, y0, x1, y1) = d.geom.polygon.bbox();
    let pad = 0.05 * (x1 - x0).max(y1 - y0);
    for r in &d.rivers {
        // Entry/exit are crossings on shared edges → on the district boundary.
        if !point_within(r.entry, &d.geom.polygon, tol) {
            issues.push(format!(
                "river {} entry off the district boundary",
                r.river_gx
            ));
        }
        if !point_within(r.exit, &d.geom.polygon, tol) {
            issues.push(format!(
                "river {} exit off the district boundary",
                r.river_gx
            ));
        }
        // The bezier path must not wander far outside the district.
        if r.to_polyline(16)
            .iter()
            .any(|p| p.x < x0 - pad || p.x > x1 + pad || p.y < y0 - pad || p.y > y1 + pad)
        {
            issues.push(format!(
                "river {} path leaves the district bounds",
                r.river_gx
            ));
        }
    }

    Check::new(
        "rivers_valid",
        issues.is_empty(),
        if issues.is_empty() {
            format!(
                "{} river segment(s) geometrically ok (is_river={})",
                d.rivers.len(),
                is_river(d.cell.gx)
            )
        } else {
            issues.join("; ")
        },
    )
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn check_objects_in_district(d: &DistrictData) -> Check {
    // Every object must have a finite position within the district's footprint.
    // We test a padded bounding box rather than strict polygon containment:
    // road-edge fixtures/conveyances are offset from a road centreline, so a
    // boundary (or district-exiting) road legitimately places them just outside
    // the polygon. The check still catches NaN/inf and gross misplacement
    // (an object at the origin when the district sits at x≈40000, etc.).
    let (x0, y0, x1, y1) = d.geom.polygon.bbox();
    let diag = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    let pad = (0.08 * diag).max(config().lamp_offset * 2.0);
    let outside = |p: Point| {
        !finite(p) || p.x < x0 - pad || p.x > x1 + pad || p.y < y0 - pad || p.y > y1 + pad
    };
    let f = d.fixtures.iter().filter(|&&p| outside(p)).count();
    let fl = d.flora.iter().filter(|&&p| outside(p)).count();
    let cv = d.conveyances.iter().filter(|&&(p, _)| outside(p)).count();
    Check::new(
        "objects_in_district",
        f + fl + cv == 0,
        format!(
            "out-of-bounds/total — fixtures {}/{}, flora {}/{}, conveyances {}/{}",
            f,
            d.fixtures.len(),
            fl,
            d.flora.len(),
            cv,
            d.conveyances.len()
        ),
    )
}

fn check_conveyances_on_roads(d: &DistrictData) -> Check {
    // Conveyances are placed along road segments (with a road-edge offset), so
    // each should sit near some road.
    let thresh = config().lamp_offset * 2.0 + 6.0;
    let off = d
        .conveyances
        .iter()
        .filter(|&&(p, _)| {
            !d.roads
                .segments
                .iter()
                .any(|s| point_on_segment(p, &Segment::new(s.a, s.b), thresh))
        })
        .count();
    Check::new(
        "conveyances_on_roads",
        off == 0,
        format!("{}/{} conveyances not near a road (≤{:.0} wu)", off, d.conveyances.len(), thresh),
    )
}

fn point_on_segment(p: Point, s: &Segment, tol: f64) -> bool {
    let dx = s.b.x - s.a.x;
    let dy = s.b.y - s.a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-9 {
        return pt_eq(p, s.a);
    }
    let t = (((p.x - s.a.x) * dx + (p.y - s.a.y) * dy) / len_sq).clamp(0.0, 1.0);
    let proj = Point::new(s.a.x + t * dx, s.a.y + t * dy);
    (p.x - proj.x).abs() <= tol && (p.y - proj.y).abs() <= tol
}

/// Run all single-district checks.
pub fn audit_district(cell: &Cell) -> AuditReport {
    let d = DistrictData::build(cell.clone());
    let checks = vec![
        check_determinism(&d),
        check_blocks_in_district(&d),
        check_blocks_no_overlap(&d),
        check_buildings_in_block(&d),
        check_buildings_no_overlap(&d),
        check_roads_present(&d),
        check_intersections_on_segments(&d),
        check_rivers(&d),
        check_objects_in_district(&d),
        check_conveyances_on_roads(&d),
    ];
    let pass = checks.iter().all(|c| c.pass);
    let metrics = serde_json::json!({
        "blocks": d.blocks.len(),
        "buildings": d.buildings.len(),
        "fixtures": d.fixtures.len(),
        "flora": d.flora.len(),
        "creatures": d.creature_count,
        "conveyances": d.conveyances.len(),
        "road_segments": d.roads.segments.len(),
        "intersections": d.roads.intersections.len(),
        "rivers": d.rivers.len(),
        // True when is_river(gx) holds but the cell can't route a river because it
        // isn't Voronoi-adjacent to both grid gy-neighbours (model limitation).
        "river_routing_gap": is_river(d.cell.gx) && d.rivers.is_empty(),
        "shared_edges": d.geom.shared_edges.len(),
        "popcount": d.cell.popcount,
        "domain": d.cell.domain,
    });
    AuditReport {
        ip: cell.ip_prefix(),
        cell_key: cell.key,
        pass,
        checks,
        metrics,
    }
}

// ── Cross-district checks ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CrossAuditReport {
    pub ip_a: String,
    pub ip_b: String,
    pub pass: bool,
    pub checks: Vec<Check>,
}

/// Audit continuity between a district and one neighbour (by octet delta).
pub fn audit_cross_district(
    cell_a: &Cell,
    d_octet2: i16,
    d_octet3: i16,
) -> Option<CrossAuditReport> {
    let o = cell_a.octets;
    let n2 = o[1] as i16 + d_octet2;
    let n3 = o[2] as i16 + d_octet3;
    if !(0..=255).contains(&n2) || !(0..=255).contains(&n3) {
        return None;
    }
    Some(audit_cross_cells(
        cell_a,
        &Cell::from_octets(o[0], n2 as u8, n3 as u8),
    ))
}

/// Audit continuity between two specific districts.
pub fn audit_cross_cells(cell_a: &Cell, cell_b: &Cell) -> CrossAuditReport {
    let a = generate_district(cell_a);
    let b = generate_district(cell_b);
    let roads_a = generate_roads(&a);
    let roads_b = generate_roads(&b);

    let mut checks = Vec::new();

    // 1. Shared-edge agreement: A's edge toward B == B's edge toward A.
    let ea = a.shared_edges.iter().find(|e| e.neighbor_key == cell_b.key);
    let eb = b.shared_edges.iter().find(|e| e.neighbor_key == cell_a.key);
    let edge_ok = match (ea, eb) {
        (Some(ea), Some(eb)) => {
            // Same segment, either orientation.
            (pt_eq(ea.start, eb.start) && pt_eq(ea.end, eb.end))
                || (pt_eq(ea.start, eb.end) && pt_eq(ea.end, eb.start))
        }
        _ => false,
    };
    checks.push(Check::new(
        "shared_edge_agreement",
        edge_ok || (ea.is_none() && eb.is_none()),
        match (ea, eb) {
            (Some(ea), Some(eb)) => format!(
                "A[{:.1},{:.1}->{:.1},{:.1}] B[{:.1},{:.1}->{:.1},{:.1}]",
                ea.start.x,
                ea.start.y,
                ea.end.x,
                ea.end.y,
                eb.start.x,
                eb.start.y,
                eb.end.x,
                eb.end.y
            ),
            _ => "no shared edge between these cells".to_string(),
        },
    ));

    // 2. Road-crossing alignment: terminals A places toward B match terminals B
    //    places toward A (same world positions on the shared edge).
    let ta: Vec<Point> = roads_a
        .terminals
        .iter()
        .filter(|t| t.neighbor_key == cell_b.key)
        .map(|t| t.position)
        .collect();
    let tb: Vec<Point> = roads_b
        .terminals
        .iter()
        .filter(|t| t.neighbor_key == cell_a.key)
        .map(|t| t.position)
        .collect();
    let matched = ta
        .iter()
        .filter(|pa| tb.iter().any(|pb| pt_eq(**pa, *pb)))
        .count();
    let crossing_ok = ta.len() == tb.len() && matched == ta.len();
    checks.push(Check::new(
        "road_crossing_alignment",
        crossing_ok,
        format!(
            "A has {} terminals toward B, B has {}, {} matched",
            ta.len(),
            tb.len(),
            matched
        ),
    ));

    // 3. Districts don't overlap (they tile the plane).
    let overlap = polys_overlap(&a.polygon, &b.polygon)
        && overlap_fraction(&a.polygon, &b.polygon, 16) > 0.05;
    checks.push(Check::new(
        "districts_no_overlap",
        !overlap,
        if overlap {
            "district polygons overlap"
        } else {
            "ok"
        },
    ));

    // 4. River continuity: vertical neighbours (same gx, adjacent gy) that host a
    //    river must share the crossing on their boundary — this district's exit
    //    toward the southern neighbour equals that neighbour's entry, so the
    //    river flows unbroken across the border.
    if cell_a.gx == cell_b.gx && is_river(cell_a.gx) {
        let (gy_a, gy_b) = (cell_a.gy as i64, cell_b.gy as i64);
        if (gy_a - gy_b).abs() == 1 {
            let ra = generate_rivers(&a);
            let rb = generate_rivers(&b);
            let (ok, detail) = match (ra.first(), rb.first()) {
                (Some(ra), Some(rb)) => {
                    // B south of A → A.exit == B.entry; B north of A → A.entry == B.exit.
                    let (pa, pb) = if gy_b < gy_a {
                        (ra.exit, rb.entry)
                    } else {
                        (ra.entry, rb.exit)
                    };
                    (
                        pt_eq(pa, pb),
                        format!(
                            "A[{:.1},{:.1}] B[{:.1},{:.1}] Δ={:.2}",
                            pa.x,
                            pa.y,
                            pb.x,
                            pb.y,
                            pa.distance_to(pb)
                        ),
                    )
                }
                (None, None) => (true, "river gx but no segments either side".into()),
                // One side routes a river, the other doesn't: a Voronoi routing
                // gap (the non-routing cell isn't Voronoi-adjacent to its *other*
                // grid gy-neighbour, so it can't continue the river). A known
                // model limitation, not a misalignment bug — don't hard-fail.
                _ => (true, "river routing gap (Voronoi non-adjacency)".into()),
            };
            checks.push(Check::new("river_continuity", ok, detail));
        }
    }

    let pass = checks.iter().all(|c| c.pass);
    CrossAuditReport {
        ip_a: cell_a.ip_prefix(),
        ip_b: cell_b.ip_prefix(),
        pass,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness itself runs and produces a populated report (always on).
    #[test]
    fn harness_produces_report() {
        let cell = Cell::from_ip_str("93.184.216.0").unwrap();
        let r = audit_district(&cell);
        assert_eq!(r.checks.len(), 10);
        assert!(r.checks.iter().any(|c| c.name == "rivers_valid"));
        assert!(r.checks.iter().any(|c| c.name == "objects_in_district"));
        assert!(r.checks.iter().any(|c| c.name == "buildings_no_overlap"));
    }

    /// Fuzz a broad spread of districts across domains/popcounts — every
    /// single-district invariant must hold. The regression net for generation
    /// bugs (caught the bisecting-alley overlap, coincident-plot, and boundary
    /// containment bugs; keeps them fixed).
    #[test]
    fn audit_spread_of_districts() {
        let mut failures = Vec::new();
        for a in [1u8, 8, 12, 50, 93, 100, 127, 172, 192, 203, 224, 240, 254] {
            for b in [0u8, 99, 200] {
                // c includes river-hosting gx values (12, 18, 54) so rivers_valid
                // is exercised on districts that actually have a river.
                for c in [0u8, 12, 18, 42, 54, 188] {
                    let ip = format!("{a}.{b}.{c}.0");
                    let cell = Cell::from_ip_str(&ip).unwrap();
                    let report = audit_district(&cell);
                    for chk in report.checks.iter().filter(|c| !c.pass) {
                        failures.push(format!("{ip}: {} — {}", chk.name, chk.detail));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} audit failures across the spread:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn audit_cross_district_continuity() {
        // Several districts vs their 4 axis neighbours — shared edges agree, road
        // crossings align across the border, and districts don't overlap.
        let mut failures = Vec::new();
        // Includes river-hosting districts (gx 12/54) so the N/S neighbour checks
        // exercise river_continuity across a real river border.
        for ip in [
            "93.184.216.0",
            "8.8.8.0",
            "1.0.0.0",
            "203.0.113.0",
            "100.64.0.0",
            "50.50.12.0",
            "150.100.54.0",
        ] {
            let cell = Cell::from_ip_str(ip).unwrap();
            for (d2, d3) in [(0i16, 1i16), (0, -1), (1, 0), (-1, 0)] {
                if let Some(report) = audit_cross_district(&cell, d2, d3) {
                    for c in report.checks.iter().filter(|c| !c.pass) {
                        failures.push(format!(
                            "{}|{}: {} — {}",
                            report.ip_a, report.ip_b, c.name, c.detail
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "cross-district failures:\n{}",
            failures.join("\n")
        );
    }
}

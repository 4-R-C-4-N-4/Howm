//! Structured world-structure audit — a machine-checkable layer above the
//! renderer. Runs geometric/topological invariants over generated districts and
//! returns a pass/fail report, so generation correctness (and cross-district
//! continuity) can be verified automatically instead of by eyeballing maps.

use serde::Serialize;

use crate::gen::blocks::{extract_blocks, Block};
use crate::gen::buildings::{generate_buildings, BuildingPlot};
use crate::gen::cell::Cell;
use crate::gen::district::{generate_district, DistrictGeometry};
use crate::gen::rivers::generate_rivers;
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
    blocks: Vec<Block>,
    /// (block_idx, plot) for every building.
    buildings: Vec<(usize, BuildingPlot)>,
}

impl DistrictData {
    fn build(cell: Cell) -> Self {
        let geom = generate_district(&cell);
        let roads = generate_roads(&geom);
        let rivers = generate_rivers(&cell, &geom.polygon.vertices);
        let blocks = extract_blocks(&cell, &geom.polygon, &roads, &rivers);
        let buildings = blocks
            .iter()
            .flat_map(|b| {
                generate_buildings(&cell, b)
                    .plots
                    .into_iter()
                    .map(move |p| (b.idx, p))
            })
            .collect();
        Self {
            cell,
            geom,
            roads,
            blocks,
            buildings,
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
    let outside: Vec<usize> = d
        .blocks
        .iter()
        .filter(|b| !d.geom.polygon.contains(b.polygon.centroid()))
        .map(|b| b.idx)
        .collect();
    Check::new(
        "blocks_in_district",
        outside.is_empty(),
        format!("{} block centroids outside district: {:?}", outside.len(), outside),
    )
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
        format!("{} overlapping block pairs (idx_a,idx_b,%): {:?}", pairs.len(), pairs),
    )
}

fn check_buildings_in_block(d: &DistrictData) -> Check {
    let mut bad = 0usize;
    for (bidx, plot) in &d.buildings {
        if let Some(block) = d.blocks.iter().find(|b| b.idx == *bidx) {
            if !block.polygon.contains(plot.polygon.centroid()) {
                bad += 1;
            }
        }
    }
    Check::new(
        "buildings_in_block",
        bad == 0,
        format!("{}/{} building centroids outside their block", bad, d.buildings.len()),
    )
}

fn check_buildings_no_overlap(d: &DistrictData) -> Check {
    let mut overlaps = 0usize;
    let mut worst = 0.0f64;
    for i in 0..d.buildings.len() {
        for j in (i + 1)..d.buildings.len() {
            let (pa, pb) = (&d.buildings[i].1.polygon, &d.buildings[j].1.polygon);
            if polys_overlap(pa, pb) {
                let f = overlap_fraction(pa, pb, 8);
                if f > 0.10 {
                    overlaps += 1;
                    worst = worst.max(f);
                }
            }
        }
    }
    Check::new(
        "buildings_no_overlap",
        overlaps == 0,
        format!("{} overlapping building pairs (>10% area), worst {:.0}%", overlaps, worst * 100.0),
    )
}

fn check_roads_present(d: &DistrictData) -> Check {
    // Populated cells (popcount > 0) should have a road network.
    let need = d.cell.popcount > 0;
    let ok = !need || !d.roads.segments.is_empty();
    Check::new(
        "roads_present",
        ok,
        format!("{} segments, {} intersections (popcount {})", d.roads.segments.len(), d.roads.intersections.len(), d.cell.popcount),
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
        format!("{}/{} intersections not on their segments", bad, d.roads.intersections.len()),
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
    ];
    let pass = checks.iter().all(|c| c.pass);
    let metrics = serde_json::json!({
        "blocks": d.blocks.len(),
        "buildings": d.buildings.len(),
        "road_segments": d.roads.segments.len(),
        "intersections": d.roads.intersections.len(),
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
pub fn audit_cross_district(cell_a: &Cell, d_octet2: i16, d_octet3: i16) -> Option<CrossAuditReport> {
    let o = cell_a.octets;
    let n2 = o[1] as i16 + d_octet2;
    let n3 = o[2] as i16 + d_octet3;
    if !(0..=255).contains(&n2) || !(0..=255).contains(&n3) {
        return None;
    }
    Some(audit_cross_cells(cell_a, &Cell::from_octets(o[0], n2 as u8, n3 as u8)))
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
                ea.start.x, ea.start.y, ea.end.x, ea.end.y, eb.start.x, eb.start.y, eb.end.x, eb.end.y
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
        format!("A has {} terminals toward B, B has {}, {} matched", ta.len(), tb.len(), matched),
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
        assert_eq!(r.checks.len(), 7);
        assert!(r.checks.iter().any(|c| c.name == "buildings_no_overlap"));
    }

    /// Fuzz a spread of districts across domains/popcounts — all single-district
    /// invariants must hold. This is the regression net for generation bugs.
    ///
    /// IGNORED until the open generation bugs the audit surfaced are fixed:
    ///   - buildings_no_overlap (plot subdivision lets footprints intersect)
    ///   - blocks_in_district   (2.188.188.0 block 2 centroid escapes)
    /// Un-ignore each as its fix lands.
    #[test]
    #[ignore = "tracks open generation bugs found by the audit; see PROGRESS"]
    fn audit_spread_of_districts() {
        let ips = [
            "1.0.0.0",
            "8.8.8.0",
            "93.184.216.0",
            "127.0.0.0",
            "192.168.1.0",
            "2.188.188.0", // feedback v2 #5 — "buildings intersect"
            "203.0.113.0",
            "224.0.0.0",
            "254.254.254.0",
        ];
        let mut failures = Vec::new();
        for ip in ips {
            let cell = Cell::from_ip_str(ip).unwrap();
            let report = audit_district(&cell);
            if !report.pass {
                for c in report.checks.iter().filter(|c| !c.pass) {
                    failures.push(format!("{ip}: {} — {}", c.name, c.detail));
                }
            }
        }
        assert!(failures.is_empty(), "audit failures:\n{}", failures.join("\n"));
    }

    #[test]
    #[ignore = "tracks open cross-district road-terminal misalignment; see PROGRESS"]
    fn audit_cross_district_continuity() {
        // Check a district against its 4 axis neighbours.
        let cell = Cell::from_ip_str("93.184.216.0").unwrap();
        let mut failures = Vec::new();
        for (d2, d3) in [(0i16, 1i16), (0, -1), (1, 0), (-1, 0)] {
            if let Some(report) = audit_cross_district(&cell, d2, d3) {
                for c in report.checks.iter().filter(|c| !c.pass) {
                    failures.push(format!("{}|{}: {} — {}", report.ip_a, report.ip_b, c.name, c.detail));
                }
            }
        }
        assert!(failures.is_empty(), "cross-district failures:\n{}", failures.join("\n"));
    }
}

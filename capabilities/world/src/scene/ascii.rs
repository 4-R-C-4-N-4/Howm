//! ASCII top-down map of a district — a text rasterisation of the generated
//! geometry that can be inspected directly in a terminal / tool output (a layer
//! above the SVG/3D renderer, so structure can be verified without a browser).
//!
//! Layers, painted back-to-front: block fill (by type) → rivers → roads →
//! building footprints → road intersections. A legend + stats footer summarise
//! the district's identity and entity counts.

use crate::gen::blocks::{Block, BlockType};
use crate::gen::buildings::BuildingPlot;
use crate::gen::cell::Cell;
use crate::gen::rivers::RiverSegment;
use crate::gen::roads::RoadNetwork;
use crate::types::{Point, Polygon};

use std::fmt::Write as _;

pub struct AsciiConfig {
    pub cols: usize,
    pub rows: usize,
}

impl Default for AsciiConfig {
    fn default() -> Self {
        Self {
            cols: 100,
            rows: 48,
        }
    }
}

/// World→grid mapping over the district bounding box, preserving aspect with a
/// 1-cell border. Characters are ~2:1 tall, so Y is not squashed further.
struct Grid {
    cols: usize,
    rows: usize,
    min_x: f64,
    min_y: f64,
    sx: f64,
    sy: f64,
    cells: Vec<char>,
}

impl Grid {
    fn new(poly: &Polygon, cfg: &AsciiConfig) -> Self {
        let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
        let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);
        for p in &poly.vertices {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        let w = (max_x - min_x).max(1.0);
        let h = (max_y - min_y).max(1.0);
        Self {
            cols: cfg.cols,
            rows: cfg.rows,
            min_x,
            min_y,
            sx: (cfg.cols as f64 - 1.0) / w,
            sy: (cfg.rows as f64 - 1.0) / h,
            cells: vec![' '; cfg.cols * cfg.rows],
        }
    }

    /// World point → (col, row). Row is flipped so +Y (north) is up.
    fn to_cell(&self, p: Point) -> (i64, i64) {
        let c = ((p.x - self.min_x) * self.sx).round() as i64;
        let r = (self.rows as i64 - 1) - ((p.y - self.min_y) * self.sy).round() as i64;
        (c, r)
    }

    fn center_of(&self, col: usize, row: usize) -> Point {
        let x = self.min_x + col as f64 / self.sx;
        let y = self.min_y + (self.rows - 1 - row) as f64 / self.sy;
        Point::new(x, y)
    }

    fn put(&mut self, col: i64, row: i64, ch: char) {
        if col >= 0 && col < self.cols as i64 && row >= 0 && row < self.rows as i64 {
            self.cells[row as usize * self.cols + col as usize] = ch;
        }
    }

    /// Draw a line of `ch` between two world points (Bresenham over grid cells).
    fn line(&mut self, a: Point, b: Point, ch: char) {
        let (mut x0, mut y0) = self.to_cell(a);
        let (x1, y1) = self.to_cell(b);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put(x0, y0, ch);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn render(&self) -> String {
        let mut s = String::with_capacity((self.cols + 1) * self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                s.push(self.cells[r * self.cols + c]);
            }
            s.push('\n');
        }
        s
    }
}

/// Sample a cubic bezier into `n` points.
fn sample_bezier(p0: Point, c1: Point, c2: Point, p3: Point, n: usize) -> Vec<Point> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            Point::new(
                a * p0.x + b * c1.x + c * c2.x + d * p3.x,
                a * p0.y + b * c1.y + c * c2.y + d * p3.y,
            )
        })
        .collect()
}

fn block_char(bt: BlockType) -> char {
    match bt {
        BlockType::Building => '.',
        BlockType::Park => ',',
        BlockType::Plaza => '·',
        BlockType::Water => '~',
        BlockType::Riverbank => ':',
    }
}

/// Render an ASCII map of a district. `buildings` is the flattened plot list.
pub fn generate_district_ascii(
    cell: &Cell,
    district_polygon: &Polygon,
    blocks: &[Block],
    roads: &RoadNetwork,
    rivers: &[RiverSegment],
    buildings: &[BuildingPlot],
    cfg: &AsciiConfig,
) -> String {
    let mut g = Grid::new(district_polygon, cfg);

    // Layer 0 — block fill: every cell whose centre lies in a block.
    for row in 0..g.rows {
        for col in 0..g.cols {
            let p = g.center_of(col, row);
            if !district_polygon.contains(p) {
                continue;
            }
            let ch = blocks
                .iter()
                .find(|b| b.polygon.contains(p))
                .map(|b| block_char(b.block_type))
                .unwrap_or(' ');
            g.put(col as i64, row as i64, ch);
        }
    }

    // Layer 1 — rivers (cubic bezier sampled to a polyline).
    for seg in rivers {
        let pts = sample_bezier(seg.entry, seg.cp1, seg.cp2, seg.exit, 16);
        for w in pts.windows(2) {
            g.line(w[0], w[1], '≈');
        }
    }

    // Layer 2 — roads.
    for seg in &roads.segments {
        g.line(seg.a, seg.b, '#');
    }

    // Layer 3 — building footprints (centroid marker).
    for plot in buildings {
        let c = plot.polygon.centroid();
        let (col, row) = g.to_cell(c);
        g.put(col, row, 'B');
    }

    // Layer 4 — intersections.
    for ix in &roads.intersections {
        let (col, row) = g.to_cell(ix.position);
        g.put(col, row, '+');
    }

    // Assemble output with a header, the grid, a legend, and stats.
    let mut out = String::new();
    let count = |t: BlockType| blocks.iter().filter(|b| b.block_type == t).count();
    let _ = writeln!(
        out,
        "{} | pop {} ({:.2}) age {:.2} {:?} hue {:.0}",
        cell.ip_prefix(),
        cell.popcount,
        cell.popcount_ratio,
        cell.age,
        cell.domain,
        cell.hue
    );
    let _ = writeln!(out, "{}", "─".repeat(cfg.cols.min(100)));
    out.push_str(&g.render());
    let _ = writeln!(out, "{}", "─".repeat(cfg.cols.min(100)));
    let _ = writeln!(
        out,
        "legend: '.'=building-block ','=park '·'=plaza '~'=water ':'=riverbank '#'=road '≈'=river 'B'=building '+'=intersection"
    );
    let _ = writeln!(
        out,
        "blocks: {} (building {}, park {}, plaza {}, water {}, riverbank {}) | roads: {} seg, {} intersections | rivers: {} | buildings: {}",
        blocks.len(),
        count(BlockType::Building),
        count(BlockType::Park),
        count(BlockType::Plaza),
        count(BlockType::Water),
        count(BlockType::Riverbank),
        roads.segments.len(),
        roads.intersections.len(),
        rivers.len(),
        buildings.len(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_nonempty_grid_with_roads() {
        let cell = Cell::from_ip_str("93.184.216.0").unwrap();
        let dist = crate::gen::district::generate_district(&cell);
        let roads = crate::gen::roads::generate_roads(&dist);
        let rivers = crate::gen::rivers::generate_rivers(&cell, &dist.polygon.vertices);
        let blocks = crate::gen::blocks::extract_blocks(&cell, &dist.polygon, &roads, &rivers);
        let buildings: Vec<_> = blocks
            .iter()
            .flat_map(|b| crate::gen::buildings::generate_buildings(&cell, b).plots)
            .collect();
        let txt = generate_district_ascii(
            &cell,
            &dist.polygon,
            &blocks,
            &roads,
            &rivers,
            &buildings,
            &AsciiConfig::default(),
        );
        assert!(txt.contains('#'), "expected road cells");
        assert!(txt.contains("blocks:"), "expected stats footer");
        // Deterministic.
        let txt2 = generate_district_ascii(
            &cell,
            &dist.polygon,
            &blocks,
            &roads,
            &rivers,
            &buildings,
            &AsciiConfig::default(),
        );
        assert_eq!(txt, txt2);
    }
}

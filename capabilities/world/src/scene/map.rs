//! Top-down SVG map generator for districts.
//!
//! Renders roads, rivers, blocks, buildings, fixtures, flora, creatures
//! as a 2D overhead map. Useful for debugging layout and as a navigational
//! feature in the capability.

use crate::gen::aesthetic::AestheticPalette;
use crate::gen::blocks::{Block, BlockType};
use crate::gen::buildings::BuildingPlot;
use crate::gen::cell::Cell;
use crate::gen::creatures::Creature;
use crate::gen::fixtures::Fixture;
use crate::gen::flora::Flora;
use crate::gen::rivers::RiverSegment;
use crate::gen::roads::RoadNetwork;
use crate::types::{Point, Polygon};

use std::fmt::Write;

/// Configuration for map rendering.
pub struct MapConfig {
    pub width: f64,
    pub height: f64,
    pub padding: f64,
    pub show_grid: bool,
    pub show_labels: bool,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 800.0,
            padding: 20.0,
            show_grid: false,
            show_labels: true,
        }
    }
}

/// Viewport transform — maps world coordinates to SVG coordinates.
struct Viewport {
    min_x: f64,
    min_y: f64,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    svg_height: f64,
}

impl Viewport {
    fn from_polygon(poly: &Polygon, width: f64, height: f64, padding: f64) -> Self {
        let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
        let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);
        for p in &poly.vertices {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }

        let world_w = (max_x - min_x).max(1.0);
        let world_h = (max_y - min_y).max(1.0);
        let usable_w = width - 2.0 * padding;
        let usable_h = height - 2.0 * padding;
        let scale = (usable_w / world_w).min(usable_h / world_h);

        Self {
            min_x,
            min_y,
            scale,
            offset_x: padding + (usable_w - world_w * scale) * 0.5,
            offset_y: padding + (usable_h - world_h * scale) * 0.5,
            svg_height: height,
        }
    }

    /// World point → SVG coordinate (Y-flipped for screen).
    fn transform(&self, p: &Point) -> (f64, f64) {
        let sx = (p.x - self.min_x) * self.scale + self.offset_x;
        let sy = self.svg_height - ((p.y - self.min_y) * self.scale + self.offset_y);
        (sx, sy)
    }
}

/// Hue (0-360) to CSS hsl string.
fn hsl(h: f64, s: f64, l: f64) -> String {
    format!("hsl({:.0},{:.0}%,{:.0}%)", h, s * 100.0, l * 100.0)
}

/// Generate a complete SVG map for a district.
pub fn generate_district_map(
    cell: &Cell,
    palette: &AestheticPalette,
    district_polygon: &Polygon,
    blocks: &[Block],
    road_network: &RoadNetwork,
    rivers: &[RiverSegment],
    buildings_per_block: &[Vec<BuildingPlot>],
    fixtures_per_block: &[Vec<Fixture>],
    flora_per_block: &[Vec<Flora>],
    creatures: &[(Point, String)],  // (position, ecological_role)
    config: &MapConfig,
) -> String {
    let vp = Viewport::from_polygon(district_polygon, config.width, config.height, config.padding);
    let mut svg = String::with_capacity(32768);

    // SVG header
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" style="background:#111">
<defs>
  <style>
    text {{ font-family: monospace; fill: #888; font-size: 10px; }}
    .label {{ font-size: 12px; fill: #ccc; text-anchor: middle; }}
    .title {{ font-size: 14px; fill: #fff; font-weight: bold; text-anchor: middle; }}
  </style>
</defs>
"#,
        w = config.width,
        h = config.height,
    )
    .unwrap();

    // Title
    if config.show_labels {
        write!(
            svg,
            r#"<text class="title" x="{}" y="16">{} — pop:{} age:{:.2} {:?}</text>
"#,
            config.width * 0.5,
            cell.ip_prefix(),
            cell.popcount,
            cell.age,
            cell.domain,
        )
        .unwrap();
    }

    // Blocks — coloured by type
    for block in blocks {
        let (fill, opacity) = match block.block_type {
            BlockType::Building => (hsl(palette.hue, 0.15, 0.2), 0.6),
            BlockType::Park => ("hsl(120,30%,20%)".to_string(), 0.6),
            BlockType::Plaza => (hsl(palette.hue, 0.1, 0.25), 0.5),
            BlockType::Water => ("hsl(210,40%,25%)".to_string(), 0.7),
            BlockType::Riverbank => ("hsl(180,20%,20%)".to_string(), 0.5),
        };
        svg_polygon(
            &mut svg,
            &block.polygon.vertices,
            &vp,
            &fill,
            "#555",
            0.5,
            Some(opacity),
        );
    }

    // Rivers — bezier curves sampled to polylines
    for river in rivers {
        let mut pts = Vec::with_capacity(11);
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            pts.push(river.at(t));
        }
        svg_polyline(&mut svg, &pts, &vp, "hsl(210,50%,40%)", 3.0, Some(0.8));
    }

    // Roads
    for seg in &road_network.segments {
        let (ax, ay) = vp.transform(&seg.a);
        let (bx, by) = vp.transform(&seg.b);
        svg.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#888\" stroke-width=\"1.5\" stroke-linecap=\"round\" opacity=\"0.7\"/>\n",
            ax, ay, bx, by,
        ));
    }

    // Buildings
    for (block_idx, plots) in buildings_per_block.iter().enumerate() {
        for plot in plots {
            let fill = if plot.is_public {
                hsl(palette.hue, 0.4, 0.45)
            } else {
                hsl(palette.hue, 0.2, 0.35)
            };
            svg_polygon(&mut svg, &plot.polygon.vertices, &vp, &fill, "#aaa", 0.8, Some(0.85));

            // Height indicator — brighter = taller
            if plot.height > 5.0 {
                let (cx, cy) = vp.transform(&plot.centroid);
                let r = 2.0 + plot.height * 0.3;
                let _ = write!(svg, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#fff\" stroke-width=\"0.5\" opacity=\"0.4\"/>\n", cx, cy, r);
            }
        }
    }

    // Fixtures — small dots
    for fixtures in fixtures_per_block {
        for f in fixtures {
            let (fx, fy) = vp.transform(&f.position);
            let color = if f.emissive_light {
                "hsl(45,80%,60%)"
            } else {
                "hsl(0,0%,50%)"
            };
            write!(
                svg,
                r#"<circle cx="{:.1}" cy="{:.1}" r="1.5" fill="{}" opacity="0.6"/>
"#,
                fx, fy, color,
            )
            .unwrap();
        }
    }

    // Flora — green dots
    for flora_list in flora_per_block {
        for f in flora_list {
            let (fx, fy) = vp.transform(&f.position);
            let r = 1.0 + f.scale * 0.3;
            write!(
                svg,
                r#"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="hsl(120,40%,35%)" opacity="0.5"/>
"#,
                fx, fy, r,
            )
            .unwrap();
        }
    }

    // Creatures — magenta diamonds
    for (pos, _role) in creatures {
        let (cx, cy) = vp.transform(pos);
        let r = 3.0;
        let _ = write!(svg, "<polygon points=\"{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" fill=\"hsl(300,70%,60%)\" stroke=\"hsl(300,90%,80%)\" stroke-width=\"0.5\" opacity=\"0.9\"/>\n",
            cx, cy - r, cx + r, cy, cx, cy + r, cx - r, cy);
    }

    // District boundary — drawn last so it's on top of everything
    svg_polygon(&mut svg, &district_polygon.vertices, &vp, "none", "#ff4444", 3.0, Some(0.9));

    // Legend
    if config.show_labels {
        let lx = config.width - 140.0;
        let mut ly = config.height - 120.0;
        let items = [
            ("#888", "Roads"),
            ("hsl(210,50%,40%)", "Rivers"),
            (&hsl(palette.hue, 0.4, 0.45), "Public buildings"),
            (&hsl(palette.hue, 0.2, 0.35), "Private buildings"),
            ("hsl(120,30%,20%)", "Parks"),
            ("hsl(210,40%,25%)", "Water"),
            ("hsl(45,80%,60%)", "Lights"),
            ("hsl(120,40%,35%)", "Flora"),
            ("hsl(300,70%,60%)", "Creatures"),
        ];
        for (color, label) in items {
            let _ = write!(svg, "<rect x=\"{:.0}\" y=\"{:.0}\" width=\"10\" height=\"10\" fill=\"{}\" opacity=\"0.8\"/>\n", lx, ly, color);
            let _ = write!(svg, "<text x=\"{:.0}\" y=\"{:.0}\" fill=\"#aaa\" font-size=\"10\">{}</text>\n", lx + 14.0, ly + 9.0, label);
            ly += 14.0;
        }
    }

    svg.push_str("</svg>\n");
    svg
}

// ── SVG helpers ──

fn svg_polygon(
    svg: &mut String,
    vertices: &[Point],
    vp: &Viewport,
    fill: &str,
    stroke: &str,
    stroke_width: f64,
    opacity: Option<f64>,
) {
    if vertices.is_empty() {
        return;
    }
    svg.push_str("<polygon points=\"");
    for (i, p) in vertices.iter().enumerate() {
        let (sx, sy) = vp.transform(p);
        if i > 0 {
            svg.push(' ');
        }
        write!(svg, "{:.1},{:.1}", sx, sy).unwrap();
    }
    write!(
        svg,
        "\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"",
        fill, stroke, stroke_width
    )
    .unwrap();
    if let Some(o) = opacity {
        write!(svg, " opacity=\"{}\"", o).unwrap();
    }
    svg.push_str("/>\n");
}

fn svg_polyline(
    svg: &mut String,
    points: &[Point],
    vp: &Viewport,
    stroke: &str,
    stroke_width: f64,
    opacity: Option<f64>,
) {
    if points.len() < 2 {
        return;
    }
    svg.push_str("<polyline points=\"");
    for (i, p) in points.iter().enumerate() {
        let (sx, sy) = vp.transform(p);
        if i > 0 {
            svg.push(' ');
        }
        write!(svg, "{:.1},{:.1}", sx, sy).unwrap();
    }
    write!(
        svg,
        "\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"",
        stroke, stroke_width
    )
    .unwrap();
    if let Some(o) = opacity {
        write!(svg, " opacity=\"{}\"", o).unwrap();
    }
    svg.push_str("/>\n");
}

// ═══════════════════════════════════════════════════════════════════════════
// Neighborhood map — 3×3 grid of districts
// ═══════════════════════════════════════════════════════════════════════════

/// Generate a neighborhood SVG showing the center district and its 8 neighbors.
pub fn generate_neighborhood_map(center: &Cell) -> String {
    use crate::gen::{blocks, buildings, creatures, district, fixtures, flora, hash, roads, rivers, zones, aesthetic::AestheticPalette};

    let [o1, o2, o3] = center.octets;
    let cfg = crate::gen::config::config();

    // Collect all districts to render (center + 8 neighbors)
    struct DistrictData {
        cell: Cell,
        polygon: Polygon,
        blocks: Vec<Block>,
        roads_segs: Vec<(Point, Point)>,
        river_pts: Vec<Vec<Point>>,
        building_verts: Vec<Vec<Point>>,
        building_public: Vec<bool>,
        fixture_pts: Vec<Point>,
        fixture_emissive: Vec<bool>,
        flora_pts: Vec<(Point, f64)>,
        creature_pts: Vec<Point>,
    }

    let mut all_districts: Vec<DistrictData> = Vec::new();

    // Global bounding box
    let mut gmin_x = f64::MAX;
    let mut gmax_x = f64::MIN;
    let mut gmin_y = f64::MAX;
    let mut gmax_y = f64::MIN;

    for do2 in -1i16..=1 {
        for do3 in -1i16..=1 {
            let n2 = o2 as i16 + do2;
            let n3 = o3 as i16 + do3;
            if n2 < 0 || n2 > 255 || n3 < 0 || n3 > 255 {
                continue;
            }

            let cell = Cell::from_octets(o1, n2 as u8, n3 as u8);
            let palette = AestheticPalette::from_cell(&cell);
            let dist = district::generate_district(&cell);
            let road_net = roads::generate_roads(&dist);
            let river_data = rivers::generate_rivers(&cell, &dist.polygon.vertices);
            let blks = blocks::extract_blocks(&cell, &dist.polygon, &road_net, &river_data);

            // Update global bounds
            for v in &dist.polygon.vertices {
                gmin_x = gmin_x.min(v.x);
                gmax_x = gmax_x.max(v.x);
                gmin_y = gmin_y.min(v.y);
                gmax_y = gmax_y.max(v.y);
            }

            // Roads
            let roads_segs: Vec<(Point, Point)> = road_net.segments.iter()
                .map(|s| (s.a, s.b))
                .collect();

            // Rivers
            let river_pts: Vec<Vec<Point>> = river_data.iter()
                .map(|r| {
                    let mut pts = Vec::new();
                    for i in 0..=10 {
                        pts.push(r.at(i as f64 / 10.0));
                    }
                    pts
                })
                .collect();

            // Buildings
            let mut building_verts = Vec::new();
            let mut building_public = Vec::new();
            let mut fixture_pts = Vec::new();
            let mut fixture_emissive = Vec::new();
            let mut flora_pts = Vec::new();
            let mut creature_pts = Vec::new();

            for block in &blks {
                let b = buildings::generate_buildings(&cell, block);
                for plot in &b.plots {
                    building_verts.push(plot.polygon.vertices.clone());
                    building_public.push(plot.is_public);
                }

                let f = fixtures::generate_fixtures(&cell, block, Some(&road_net));
                for fix in f.zone_fixtures.iter().chain(f.road_fixtures.iter()) {
                    fixture_pts.push(fix.position);
                    fixture_emissive.push(fix.emissive_light);
                }

                let fl = flora::generate_flora(&cell, block, Some(&road_net));
                for flo in fl.block_flora.iter().chain(fl.road_flora.iter()) {
                    flora_pts.push((flo.position, flo.scale));
                }

                let cr = creatures::generate_creatures(&cell, block);
                for (ci, c) in cr.creatures.iter().enumerate() {
                    let ps = hash::ha(c.creature_seed ^ block.idx as u32 ^ ci as u32 ^ 0x9f3a);
                    let pos = zones::point_in_polygon_seeded(&block.polygon, ps);
                    creature_pts.push(pos);
                }
            }

            all_districts.push(DistrictData {
                cell,
                polygon: dist.polygon,
                blocks: blks,
                roads_segs,
                river_pts,
                building_verts,
                building_public,
                fixture_pts,
                fixture_emissive,
                flora_pts,
                creature_pts,
            });
        }
    }

    // Build viewport from global bounds
    let svg_w = 1200.0_f64;
    let svg_h = 1200.0_f64;
    let padding = 30.0;
    let world_w = (gmax_x - gmin_x).max(1.0);
    let world_h = (gmax_y - gmin_y).max(1.0);
    let usable_w = svg_w - 2.0 * padding;
    let usable_h = svg_h - 2.0 * padding;
    let scale = (usable_w / world_w).min(usable_h / world_h);
    let off_x = padding + (usable_w - world_w * scale) * 0.5;
    let off_y = padding + (usable_h - world_h * scale) * 0.5;

    let tx = |p: &Point| -> (f64, f64) {
        let sx = (p.x - gmin_x) * scale + off_x;
        let sy = svg_h - ((p.y - gmin_y) * scale + off_y);
        (sx, sy)
    };

    let mut svg = String::with_capacity(131072);

    // Header
    write!(svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {svg_w} {svg_h}\" width=\"{svg_w}\" height=\"{svg_h}\" style=\"background:#111\">\n\
         <defs><style>text {{ font-family: monospace; fill: #888; font-size: 10px; }}</style></defs>\n"
    ).unwrap();

    // Title
    write!(svg, "<text x=\"{:.0}\" y=\"16\" fill=\"#fff\" font-size=\"14\" font-weight=\"bold\" text-anchor=\"middle\">Neighborhood: {}</text>\n",
        svg_w * 0.5, center.ip_prefix()).unwrap();

    // Render each district
    for dd in &all_districts {
        let palette = AestheticPalette::from_cell(&dd.cell);
        let is_center = dd.cell.key == center.key;
        let boundary_color = if is_center { "#ff4444" } else { "#666" };
        let boundary_width = if is_center { 2.5 } else { 1.0 };
        let boundary_opacity = if is_center { 0.9 } else { 0.5 };

        // District boundary
        svg.push_str("<polygon points=\"");
        for (i, v) in dd.polygon.vertices.iter().enumerate() {
            let (sx, sy) = tx(v);
            if i > 0 { svg.push(' '); }
            write!(svg, "{:.1},{:.1}", sx, sy).unwrap();
        }
        write!(svg, "\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" opacity=\"{}\"/>\n",
            boundary_color, boundary_width, boundary_opacity).unwrap();

        // Block fills
        for block in &dd.blocks {
            let (fill, opacity) = match block.block_type {
                BlockType::Building => (hsl(palette.hue, 0.12, 0.18), 0.4),
                BlockType::Park => ("hsl(120,25%,16%)".to_string(), 0.4),
                BlockType::Plaza => (hsl(palette.hue, 0.08, 0.2), 0.3),
                BlockType::Water => ("hsl(210,35%,20%)".to_string(), 0.5),
                BlockType::Riverbank => ("hsl(180,15%,16%)".to_string(), 0.4),
            };
            svg_polygon_tx(&mut svg, &block.polygon.vertices, &tx, &fill, "#333", 0.3, Some(opacity));
        }

        // Roads
        for (a, b) in &dd.roads_segs {
            let (ax, ay) = tx(a);
            let (bx, by) = tx(b);
            let _ = write!(svg, "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\" stroke-width=\"1\" opacity=\"0.5\"/>\n",
                ax, ay, bx, by, "#777");
        }

        // Rivers
        for pts in &dd.river_pts {
            if pts.len() >= 2 {
                svg.push_str("<polyline points=\"");
                for (i, p) in pts.iter().enumerate() {
                    let (sx, sy) = tx(p);
                    if i > 0 { svg.push(' '); }
                    write!(svg, "{:.1},{:.1}", sx, sy).unwrap();
                }
                svg.push_str("\" fill=\"none\" stroke=\"hsl(210,50%,35%)\" stroke-width=\"2\" opacity=\"0.6\"/>\n");
            }
        }

        // Buildings
        for (i, verts) in dd.building_verts.iter().enumerate() {
            let fill = if dd.building_public[i] {
                hsl(palette.hue, 0.35, 0.4)
            } else {
                hsl(palette.hue, 0.18, 0.3)
            };
            svg_polygon_tx(&mut svg, verts, &tx, &fill, "#999", 0.5, Some(0.7));
        }

        // Fixtures
        for (i, p) in dd.fixture_pts.iter().enumerate() {
            let (fx, fy) = tx(p);
            let color = if dd.fixture_emissive[i] { "hsl(45,80%,60%)" } else { "hsl(0,0%,45%)" };
            let _ = write!(svg, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"1\" fill=\"{}\" opacity=\"0.5\"/>\n", fx, fy, color);
        }

        // Flora
        for (p, s) in &dd.flora_pts {
            let (fx, fy) = tx(p);
            let r = 0.5 + s * 0.15;
            let _ = write!(svg, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"hsl(120,35%,30%)\" opacity=\"0.4\"/>\n", fx, fy, r);
        }

        // Creatures
        for p in &dd.creature_pts {
            let (cx, cy) = tx(p);
            let _ = write!(svg, "<polygon points=\"{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" fill=\"hsl(300,70%,60%)\" opacity=\"0.8\"/>\n",
                cx, cy - 2.0, cx + 2.0, cy, cx, cy + 2.0, cx - 2.0, cy);
        }

        // District label
        let center_pt = dd.polygon.centroid();
        let (lx, ly) = tx(&center_pt);
        let _ = write!(svg, "<text x=\"{:.0}\" y=\"{:.0}\" fill=\"{}\" font-size=\"9\" text-anchor=\"middle\">{}</text>\n",
            lx, ly, if is_center { "#ff8888" } else { "#777" }, dd.cell.ip_prefix());
    }

    // Compass rose (top-right)
    let cx = svg_w - 40.0;
    let cy = 50.0;
    let _ = write!(svg, "<text x=\"{cx}\" y=\"{:.0}\" fill=\"#aaa\" font-size=\"12\" text-anchor=\"middle\">N</text>\n", cy - 15.0);
    let _ = write!(svg, "<text x=\"{cx}\" y=\"{:.0}\" fill=\"#666\" font-size=\"10\" text-anchor=\"middle\">S</text>\n", cy + 25.0);
    let _ = write!(svg, "<text x=\"{:.0}\" y=\"{cy}\" fill=\"#666\" font-size=\"10\" text-anchor=\"end\">W</text>\n", cx - 18.0);
    let _ = write!(svg, "<text x=\"{:.0}\" y=\"{cy}\" fill=\"#666\" font-size=\"10\">E</text>\n", cx + 14.0);
    let _ = write!(svg, "<line x1=\"{cx}\" y1=\"{:.0}\" x2=\"{cx}\" y2=\"{:.0}\" stroke=\"#aaa\" stroke-width=\"1\"/>\n", cy - 10.0, cy + 10.0);
    let _ = write!(svg, "<line x1=\"{:.0}\" y1=\"{cy}\" x2=\"{:.0}\" y2=\"{cy}\" stroke=\"#666\" stroke-width=\"1\"/>\n", cx - 10.0, cx + 10.0);

    svg.push_str("</svg>\n");
    svg
}

/// Helper: draw a polygon using a transform function.
fn svg_polygon_tx(
    svg: &mut String,
    vertices: &[Point],
    tx: &dyn Fn(&Point) -> (f64, f64),
    fill: &str,
    stroke: &str,
    stroke_width: f64,
    opacity: Option<f64>,
) {
    if vertices.is_empty() { return; }
    svg.push_str("<polygon points=\"");
    for (i, p) in vertices.iter().enumerate() {
        let (sx, sy) = tx(p);
        if i > 0 { svg.push(' '); }
        write!(svg, "{:.1},{:.1}", sx, sy).unwrap();
    }
    write!(svg, "\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"", fill, stroke, stroke_width).unwrap();
    if let Some(o) = opacity {
        write!(svg, " opacity=\"{}\"", o).unwrap();
    }
    svg.push_str("/>\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::cell::Cell;
    use crate::gen::aesthetic::AestheticPalette;

    #[test]
    fn viewport_transform_basic() {
        let poly = Polygon::new(vec![
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 100.0),
        ]);
        let vp = Viewport::from_polygon(&poly, 800.0, 800.0, 20.0);

        // Origin maps to bottom-left area
        let (sx, sy) = vp.transform(&Point::new(0.0, 0.0));
        assert!(sx > 0.0 && sx < 400.0);
        assert!(sy > 400.0); // Y flipped

        // Top-right maps to top-right area
        let (sx2, sy2) = vp.transform(&Point::new(100.0, 100.0));
        assert!(sx2 > sx);
        assert!(sy2 < sy); // higher world Y = lower SVG Y
    }

    #[test]
    fn generate_map_produces_valid_svg() {
        let cell = Cell::from_octets(93, 184, 216);
        let palette = AestheticPalette::from_cell(&cell);
        let dist = crate::gen::district::generate_district(&cell);
        let roads = crate::gen::roads::generate_roads(&dist);
        let rivers = crate::gen::rivers::generate_rivers(&cell, &dist.polygon.vertices);
        let blocks = crate::gen::blocks::extract_blocks(
            &cell, &dist.polygon, &roads, &rivers,
        );

        let mut buildings = Vec::new();
        let mut fixtures = Vec::new();
        let mut flora = Vec::new();
        for block in &blocks {
            let b = crate::gen::buildings::generate_buildings(&cell, block);
            buildings.push(b.plots);
            let f = crate::gen::fixtures::generate_fixtures(&cell, block, Some(&roads));
            let mut all_fix = f.zone_fixtures;
            all_fix.extend(f.road_fixtures);
            fixtures.push(all_fix);
            let fl = crate::gen::flora::generate_flora(&cell, block, Some(&roads));
            let mut all_flora = fl.block_flora;
            all_flora.extend(fl.road_flora);
            flora.push(all_flora);
        }

        let svg = generate_district_map(
            &cell, &palette, &dist.polygon,
            &blocks, &roads, &rivers,
            &buildings, &fixtures, &flora,
            &vec![],  // no creatures in test
            &MapConfig::default(),
        );

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("<polygon")); // district boundary + blocks
        assert!(svg.contains("<line")); // roads
        assert!(svg.contains("<circle")); // fixtures/flora
        assert!(svg.len() > 1000); // non-trivial
    }
}

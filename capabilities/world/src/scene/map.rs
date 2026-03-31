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

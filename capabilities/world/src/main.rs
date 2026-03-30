use axum::{
    extract::Path as AxumPath,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use clap::Parser;
use include_dir::{include_dir, Dir};
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod gen;
mod hdl;
mod scene;
mod types;

static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");

#[derive(Parser, Debug)]
#[command(name = "world", about = "Howm world generation capability")]
struct Config {
    #[arg(long, default_value = "7010", env = "PORT")]
    port: u16,

    #[arg(long, default_value = "/data", env = "DATA_DIR")]
    data_dir: std::path::PathBuf,

    #[arg(long, default_value = "7000", env = "HOWM_DAEMON_PORT")]
    daemon_port: u16,

    #[arg(long, default_value = "http://127.0.0.1:7000", env = "HOWM_DAEMON_URL")]
    daemon_url: String,
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn parse_cell(ip: &str) -> Option<gen::cell::Cell> {
    gen::cell::Cell::from_ip_str(ip)
}

fn bad_request() -> Response {
    (StatusCode::BAD_REQUEST, "Invalid IPv4 address").into_response()
}

async fn health() -> &'static str {
    "ok"
}

// ─── Full district generation (Phase 4) ────────────────────────────────────

async fn district_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let district = gen::district::generate_district(&cell);
    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let road_network = gen::roads::generate_roads(&district);
    let rivers = gen::rivers::generate_rivers(&cell, &district.polygon.vertices);
    let blocks = gen::blocks::extract_blocks(
        &cell,
        &district.polygon,
        &road_network,
        &rivers,
    );

    let now_ms = current_time_ms();
    let atmosphere = gen::atmosphere::compute_atmosphere(&cell, now_ms);
    let environment = hdl::mapping::map_district_environment(&cell, &atmosphere, &palette);

    // Generate objects with description graphs
    let mut block_data = Vec::new();
    for block in &blocks {
        let buildings = gen::buildings::generate_buildings(&cell, block);
        let fixtures = gen::fixtures::generate_fixtures(&cell, block, Some(&road_network));
        let zones = gen::zones::generate_zones(cell.key, block);
        let flora = gen::flora::generate_flora(&cell, block, Some(&road_network));
        let creatures = gen::creatures::generate_creatures(&cell, block);

        // Map to description graphs
        let building_graphs: Vec<_> = buildings.plots.iter()
            .map(|b| serde_json::json!({
                "base_record": b,
                "description": hdl::mapping::map_building(b, &palette),
            }))
            .collect();

        let fixture_graphs: Vec<_> = fixtures.zone_fixtures.iter()
            .chain(fixtures.road_fixtures.iter())
            .map(|f| serde_json::json!({
                "base_record": f,
                "description": hdl::mapping::map_fixture(f, &palette),
            }))
            .collect();

        let flora_graphs: Vec<_> = flora.block_flora.iter()
            .chain(flora.road_flora.iter())
            .map(|f| serde_json::json!({
                "base_record": f,
                "description": hdl::mapping::map_flora(f, &palette),
            }))
            .collect();

        // Surface growth overlays
        let surface_overlays: Vec<_> = flora.surface_growth.iter()
            .map(|f| {
                let coverage = cell.inverted_age.min(1.0);
                serde_json::json!({
                    "base_record": f,
                    "overlay": hdl::mapping::map_surface_growth(coverage, f),
                })
            })
            .collect();

        let creature_graphs: Vec<_> = creatures.creatures.iter()
            .map(|c| serde_json::json!({
                "base_record": c,
                "description": hdl::mapping::map_creature(c, &palette),
            }))
            .collect();

        block_data.push(serde_json::json!({
            "block_idx": block.idx,
            "block_type": block.block_type,
            "buildings": building_graphs,
            "fixtures": fixture_graphs,
            "zones": zones,
            "flora": flora_graphs,
            "surface_growth": surface_overlays,
            "creatures": creature_graphs,
        }));
    }

    // Conveyances (district-level)
    let conveyances = gen::conveyances::generate_conveyances(&cell, &road_network);
    let conveyance_graphs: Vec<_> = conveyances.parked.iter()
        .chain(conveyances.route_following.iter())
        .map(|c| serde_json::json!({
            "base_record": c,
            "description": hdl::mapping::map_conveyance(c, &palette),
        }))
        .collect();

    let response = serde_json::json!({
        "hdl_version": 1,
        "cell": {
            "key": cell.key,
            "ip_prefix": cell.ip_prefix(),
            "popcount": cell.popcount,
            "popcount_ratio": cell.popcount_ratio,
            "age": cell.age,
            "domain": cell.domain,
            "hue": cell.hue,
        },
        "aesthetic": palette,
        "polygon": district.polygon,
        "shared_edges": district.shared_edges,
        "seed_position": district.seed_position,
        "blocks": block_data,
        "conveyances": conveyance_graphs,
        "atmosphere": atmosphere,
        "environment": environment,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

// ─── Geometry only ─────────────────────────────────────────────────────────

async fn district_geometry_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let district = gen::district::generate_district(&cell);
    let road_network = gen::roads::generate_roads(&district);
    let rivers = gen::rivers::generate_rivers(&cell, &district.polygon.vertices);
    let blocks = gen::blocks::extract_blocks(
        &cell,
        &district.polygon,
        &road_network,
        &rivers,
    );

    let response = serde_json::json!({
        "cell": {
            "key": district.cell.key,
            "ip_prefix": district.cell.ip_prefix(),
            "popcount": district.cell.popcount,
            "domain": district.cell.domain,
        },
        "polygon": district.polygon,
        "shared_edges": district.shared_edges,
        "seed_position": district.seed_position,
        "roads": road_network,
        "rivers": rivers,
        "blocks": blocks,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

// ─── Objects only (base records + description graphs) ──────────────────────

async fn district_objects_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let district = gen::district::generate_district(&cell);
    let road_network = gen::roads::generate_roads(&district);
    let rivers = gen::rivers::generate_rivers(&cell, &district.polygon.vertices);
    let blocks = gen::blocks::extract_blocks(
        &cell,
        &district.polygon,
        &road_network,
        &rivers,
    );

    let mut buildings_out = Vec::new();
    let mut fixtures_out = Vec::new();
    let mut flora_out = Vec::new();
    let mut creatures_out = Vec::new();
    let mut zones_out = Vec::new();

    for block in &blocks {
        let buildings = gen::buildings::generate_buildings(&cell, block);
        let fixtures = gen::fixtures::generate_fixtures(&cell, block, Some(&road_network));
        let zones = gen::zones::generate_zones(cell.key, block);
        let flora = gen::flora::generate_flora(&cell, block, Some(&road_network));
        let creatures = gen::creatures::generate_creatures(&cell, block);

        for b in &buildings.plots {
            buildings_out.push(serde_json::json!({
                "base_record": b,
                "description": hdl::mapping::map_building(b, &palette),
            }));
        }
        for f in fixtures.zone_fixtures.iter().chain(fixtures.road_fixtures.iter()) {
            fixtures_out.push(serde_json::json!({
                "base_record": f,
                "description": hdl::mapping::map_fixture(f, &palette),
            }));
        }
        for f in flora.block_flora.iter().chain(flora.road_flora.iter()) {
            flora_out.push(serde_json::json!({
                "base_record": f,
                "description": hdl::mapping::map_flora(f, &palette),
            }));
        }
        for c in &creatures.creatures {
            creatures_out.push(serde_json::json!({
                "base_record": c,
                "description": hdl::mapping::map_creature(c, &palette),
            }));
        }
        zones_out.push(serde_json::json!({
            "block_idx": block.idx,
            "zones": zones,
        }));
    }

    let conveyances = gen::conveyances::generate_conveyances(&cell, &road_network);
    let conveyance_graphs: Vec<_> = conveyances.parked.iter()
        .chain(conveyances.route_following.iter())
        .map(|c| serde_json::json!({
            "base_record": c,
            "description": hdl::mapping::map_conveyance(c, &palette),
        }))
        .collect();

    let now_ms = current_time_ms();
    let atmosphere = gen::atmosphere::compute_atmosphere(&cell, now_ms);

    let response = serde_json::json!({
        "hdl_version": 1,
        "cell": {
            "key": cell.key,
            "ip_prefix": cell.ip_prefix(),
            "popcount": cell.popcount,
            "domain": cell.domain,
        },
        "blocks": blocks.len(),
        "buildings": buildings_out,
        "fixtures": fixtures_out,
        "zones": zones_out,
        "flora": flora_out,
        "creatures": creatures_out,
        "conveyances": conveyance_graphs,
        "atmosphere": atmosphere,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

// ─── Atmosphere standalone ─────────────────────────────────────────────────

async fn district_atmosphere_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let now_ms = current_time_ms();
    let atmosphere = gen::atmosphere::compute_atmosphere(&cell, now_ms);
    let environment = hdl::mapping::map_district_environment(&cell, &atmosphere, &palette);

    let response = serde_json::json!({
        "atmosphere": atmosphere,
        "environment": environment,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

// ─── Neighbor summaries ────────────────────────────────────────────────────

async fn neighbors_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let octets = cell.octets;
    let mut neighbors = Vec::new();

    // 8 cardinal + diagonal neighbors in the /24 grid
    for (do1, do2, do3) in &[
        (0i16, 0i16, 1i16), (0, 0, -1), (0, 1, 0), (0, -1, 0),
        (0, 1, 1), (0, 1, -1), (0, -1, 1), (0, -1, -1),
    ] {
        let n1 = octets[0] as i16 + do1;
        let n2 = octets[1] as i16 + do2;
        let n3 = octets[2] as i16 + do3;

        if n1 < 0 || n1 > 255 || n2 < 0 || n2 > 255 || n3 < 0 || n3 > 255 {
            continue;
        }

        let ncell = gen::cell::Cell::from_octets(n1 as u8, n2 as u8, n3 as u8);
        let palette = gen::aesthetic::AestheticPalette::from_cell(&ncell);

        neighbors.push(serde_json::json!({
            "ip_prefix": ncell.ip_prefix(),
            "key": ncell.key,
            "popcount": ncell.popcount,
            "popcount_ratio": ncell.popcount_ratio,
            "domain": ncell.domain,
            "hue": ncell.hue,
            "age": ncell.age,
            "aesthetic_bucket": palette.aesthetic_bucket,
        }));
    }

    let response = serde_json::json!({
        "center": cell.ip_prefix(),
        "neighbors": neighbors,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

// ─── District map (SVG) ────────────────────────────────────────────────────

async fn district_map_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let district = gen::district::generate_district(&cell);
    let road_network = gen::roads::generate_roads(&district);
    let rivers = gen::rivers::generate_rivers(&cell, &district.polygon.vertices);
    let blocks = gen::blocks::extract_blocks(
        &cell,
        &district.polygon,
        &road_network,
        &rivers,
    );

    let mut buildings = Vec::new();
    let mut fixtures = Vec::new();
    let mut flora = Vec::new();
    for block in &blocks {
        let b = gen::buildings::generate_buildings(&cell, block);
        buildings.push(b.plots);
        let f = gen::fixtures::generate_fixtures(&cell, block, Some(&road_network));
        let mut all_fix = f.zone_fixtures;
        all_fix.extend(f.road_fixtures);
        fixtures.push(all_fix);
        let fl = gen::flora::generate_flora(&cell, block, Some(&road_network));
        let mut all_flora = fl.block_flora;
        all_flora.extend(fl.road_flora);
        flora.push(all_flora);
    }

    let svg = scene::map::generate_district_map(
        &cell,
        &palette,
        &district.polygon,
        &blocks,
        &road_network,
        &rivers,
        &buildings,
        &fixtures,
        &flora,
        &scene::map::MapConfig::default(),
    );

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}

// ─── Astral Scene (compiled) ───────────────────────────────────────────────

async fn district_scene_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let now_ms = current_time_ms();
    let atmo = gen::atmosphere::compute_atmosphere(&cell, now_ms);

    let astral_scene = scene::compiler::compile_district_scene(&cell, &palette, &[], &atmo);

    (StatusCode::OK, axum::Json(astral_scene)).into_response()
}

// ─── UI ────────────────────────────────────────────────────────────────────

fn serve_ui_file(path: &str) -> Response {
    let file_path = if path.is_empty() || path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };

    match UI_DIR.get_file(file_path) {
        Some(file) => {
            let content_type = match file_path.rsplit('.').next() {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "application/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                _ => "application/octet-stream",
            };
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, content_type)],
                file.contents(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

// ─── Main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::parse();

    let app = Router::new()
        .route("/cap/world/health", get(health))
        .route("/cap/world/district/{ip}", get(district_handler))
        .route(
            "/cap/world/district/{ip}/geometry",
            get(district_geometry_handler),
        )
        .route(
            "/cap/world/district/{ip}/objects",
            get(district_objects_handler),
        )
        .route(
            "/cap/world/district/{ip}/atmosphere",
            get(district_atmosphere_handler),
        )
        .route(
            "/cap/world/neighbors/{ip}",
            get(neighbors_handler),
        )
        .route(
            "/cap/world/district/{ip}/scene",
            get(district_scene_handler),
        )
        .route(
            "/cap/world/district/{ip}/map",
            get(district_map_handler),
        )
        .route("/ui/{*path}", get(|path: AxumPath<String>| async move {
            serve_ui_file(&path)
        }))
        .route("/ui/", get(|| async { serve_ui_file("index.html") }));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!("World capability listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

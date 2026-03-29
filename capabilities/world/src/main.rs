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

async fn health() -> &'static str {
    "ok"
}

async fn district_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match gen::cell::Cell::from_ip_str(&ip) {
        Some(c) => c,
        None => {
            return (StatusCode::BAD_REQUEST, "Invalid IPv4 address").into_response();
        }
    };

    let district = gen::district::generate_district(&cell);
    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);

    let response = serde_json::json!({
        "cell": district.cell,
        "polygon": district.polygon,
        "shared_edges": district.shared_edges,
        "seed_position": district.seed_position,
        "aesthetic": palette,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

async fn district_geometry_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match gen::cell::Cell::from_ip_str(&ip) {
        Some(c) => c,
        None => {
            return (StatusCode::BAD_REQUEST, "Invalid IPv4 address").into_response();
        }
    };

    let district = gen::district::generate_district(&cell);

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
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

async fn district_objects_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match gen::cell::Cell::from_ip_str(&ip) {
        Some(c) => c,
        None => {
            return (StatusCode::BAD_REQUEST, "Invalid IPv4 address").into_response();
        }
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

    let mut buildings_json = Vec::new();
    let mut fixtures_json = Vec::new();
    let mut zones_json = Vec::new();
    let mut flora_json = Vec::new();
    let mut creatures_json = Vec::new();

    for block in &blocks {
        buildings_json.push(gen::buildings::generate_buildings(&cell, block));
        fixtures_json.push(gen::fixtures::generate_fixtures(&cell, block, Some(&road_network)));
        let bz = gen::zones::generate_zones(cell.key, block);
        zones_json.push(serde_json::json!({
            "block_idx": block.idx,
            "zones": bz,
        }));
        flora_json.push(gen::flora::generate_flora(&cell, block, Some(&road_network)));
        creatures_json.push(gen::creatures::generate_creatures(&cell, block));
    }

    // Conveyances (district-level, not per-block)
    let conveyances = gen::conveyances::generate_conveyances(&cell, &road_network);

    // Atmosphere at current time
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let atmosphere = gen::atmosphere::compute_atmosphere(&cell, now_ms);

    let response = serde_json::json!({
        "cell": {
            "key": cell.key,
            "ip_prefix": cell.ip_prefix(),
            "popcount": cell.popcount,
            "domain": cell.domain,
        },
        "blocks": blocks.len(),
        "buildings": buildings_json,
        "fixtures": fixtures_json,
        "zones": zones_json,
        "flora": flora_json,
        "creatures": creatures_json,
        "conveyances": conveyances,
        "atmosphere": atmosphere,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

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
        .route("/ui/*path", get(|path: AxumPath<String>| async move {
            serve_ui_file(&path)
        }))
        .route("/ui/", get(|| async { serve_ui_file("index.html") }));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!("World capability listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

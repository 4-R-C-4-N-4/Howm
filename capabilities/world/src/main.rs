use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json,
};
use clap::Parser;
use include_dir::{include_dir, Dir};

use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{
    init_tracing, rpc as sdk_rpc, CapabilityApp, InboundMessage, LocalPeerId, PeerStream,
    PeerTracker,
};

mod audit;
mod gen;
mod hdl;
mod scene;
mod stream;
mod types;

static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");

/// P2P-CD capability id. Must match the daemon's derived `howm.{name}.1` and the
/// access-group grants (`howm.world.room.1`).
const CAP_NAME: &str = "howm.world.room.1";

/// P2P-CD message type for player-presence position updates (fire-and-forget,
/// distinct from RPC message type 22). "PR".
const PRESENCE_MSG: u64 = 0x5052;

/// How long a peer's last position stays "live" in the presence list.
const PRESENCE_TTL_MS: u64 = 10_000;

/// A player's pose, shared with peers (spaces §8.1).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Pose {
    position: [f64; 3],
    #[serde(default)]
    orientation: [f64; 3],
    #[serde(default)]
    velocity: [f64; 3],
    /// The space the player is in (e.g. district ip, "inside:<peer>", "tunnel:..").
    #[serde(default)]
    space: String,
    /// Poster's home-district ip, stamped by the cap on broadcast. Lets peers
    /// fetch the right avatar aesthetic (`/avatar/<home>/<peer>/entity`).
    #[serde(default)]
    home: String,
}

#[derive(Clone)]
struct PeerPose {
    pose: Pose,
    updated_ms: u64,
}

type PresenceMap = std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, PeerPose>>>;

/// Shared capability state. Handlers are stateless today (districts derive purely
/// from the IP); `bridge`/`peers`/`local_id` are the multiplayer/spaces plumbing
/// consumed by later phases (home placement, Inside, presence relay, avatars).
#[derive(Clone)]
#[allow(dead_code)]
struct AppState {
    /// Talks to the daemon (peer list, RPC, send, blob).
    bridge: BridgeClient,
    /// Live set of active world peers (active/inactive driven by the SSE stream).
    peers: PeerTracker,
    /// This node's own peer id — the seed for home/Inside/avatar generation.
    local_id: LocalPeerId,
    /// This node's home district cell (from `--home-ip`) — drives its avatar's
    /// aesthetic, so peers see where it's "from".
    home_cell: gen::cell::Cell,
    /// Latest known pose of each peer (by base64 peer id), for rendering them.
    presence: PresenceMap,
    /// The space the local UI most recently reported — lets the optional test
    /// peer (`--test-peer`) follow the player into whatever district they enter.
    last_space: std::sync::Arc<tokio::sync::RwLock<String>>,
}

/// Inbound P2P-CD capability messages (`POST /p2pcd/inbound`). Handles the
/// `avatar.get` RPC (returns this node's avatar); other methods are accepted
/// and ignored for now (presence updates land in the next multiplayer slice).
async fn inbound(State(state): State<AppState>, Json(msg): Json<InboundMessage>) -> Response {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let raw = match STANDARD.decode(&msg.payload) {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid payload").into_response(),
    };

    // Player-presence position update from a peer (fire-and-forget): store the
    // sender's latest pose so we can render their avatar.
    if msg.message_type == PRESENCE_MSG {
        if let Ok(pose) = serde_json::from_slice::<Pose>(&raw) {
            state.presence.write().await.insert(
                msg.peer_id.clone(),
                PeerPose {
                    pose,
                    updated_ms: current_time_ms(),
                },
            );
        }
        return (StatusCode::OK, Json(serde_json::json!({}))).into_response();
    }

    match sdk_rpc::extract_method(&raw).as_deref() {
        // A peer asks for our avatar. Generate it from our own peer id + home
        // district aesthetic and return it as the RPC response.
        Some("avatar.get") => {
            let palette = gen::aesthetic::AestheticPalette::from_cell(&state.home_cell);
            let pid = state
                .local_id
                .get()
                .await
                .and_then(|s| decode_peer_id(&s))
                .unwrap_or_default();
            let graph = hdl::mapping::map_avatar(&pid, &palette);
            let body = serde_json::to_vec(&graph).unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "response": STANDARD.encode(body) })),
            )
                .into_response()
        }
        _ => (StatusCode::OK, Json(serde_json::json!({}))).into_response(),
    }
}

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

    /// This node's home district IP — its avatar's aesthetic comes from this
    /// district (ideally the node's own public IP).
    #[arg(long, default_value = "1.0.0.0", env = "HOWM_WORLD_HOME_IP")]
    home_ip: String,

    /// Inject a synthetic orbiting peer into the presence map for spot-checking
    /// multiplayer rendering on a single node (no daemon mesh needed). It follows
    /// whichever space the local UI reports, so it appears in your current view.
    #[arg(long)]
    test_peer: bool,
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

// ─── Full district generation (Phase 4) ────────────────────────────────────

async fn district_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let dd = gen::chunk::district_data(&cell);
    let district = dd.geometry.clone();
    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let road_network = dd.roads.clone();
    let blocks = dd.blocks.clone();

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

    let dd = gen::chunk::district_data(&cell);
    let district = dd.geometry.clone();
    let road_network = dd.roads.clone();
    let rivers = dd.rivers.clone();
    let blocks = dd.blocks.clone();

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
    let dd = gen::chunk::district_data(&cell);
    let road_network = dd.roads.clone();
    let blocks = dd.blocks.clone();

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

/// Lightweight, geometry-free seed summary for a cell. Enough for a client to
/// pre-warm an adjacent district (palette, key, domain) before crossing into it.
fn cell_summary(cell: &gen::cell::Cell) -> serde_json::Value {
    let palette = gen::aesthetic::AestheticPalette::from_cell(cell);
    serde_json::json!({
        "ip_prefix": cell.ip_prefix(),
        "key": cell.key,
        "popcount": cell.popcount,
        "popcount_ratio": cell.popcount_ratio,
        "domain": cell.domain,
        "hue": cell.hue,
        "age": cell.age,
        "aesthetic_bucket": palette.aesthetic_bucket,
    })
}

/// Decode a peer id from a path segment. All-hex strings are treated as hex
/// (deterministic test inputs); otherwise base64-standard (real WireGuard-key
/// peer ids). Returns the raw bytes; the home placer uses the first 4.
fn decode_peer_id(s: &str) -> Option<Vec<u8>> {
    let is_hex = s.len() >= 2 && s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex {
        return (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
            .collect();
    }
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.decode(s).ok().filter(|b| !b.is_empty())
}

// ─── Peer home (spaces §1.2) ────────────────────────────────────────────────
//
// Places a peer's unique home structure in the district derived from `ip`,
// seeded by the peer id. Deterministic — same (ip, peer_id) → same home.

async fn district_home_handler(
    AxumPath((ip, peer_id)): AxumPath<(String, String)>,
) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let pid = match decode_peer_id(&peer_id) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "Invalid peer id").into_response(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let home = gen::home::place_home_in_cell(&cell, &pid);
    let description = hdl::mapping::map_home(&home, &palette);

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "base_record": home,
            "description": description,
        })),
    )
        .into_response()
}

// ─── Portal description (spaces §5.1) ───────────────────────────────────────

/// The fixed portal entity description graph — useful for clients/tests to
/// inspect the doorway HDL that every portal (home/inside/tunnel) animates from.
async fn portal_handler() -> Response {
    (StatusCode::OK, axum::Json(hdl::mapping::map_portal())).into_response()
}

// ─── Presence (player positions, spaces §4/§8.1) ────────────────────────────
//
// The local UI POSTs its pose at 2–4 Hz; we broadcast it to every active world
// peer (the Outside peer-to-peer model). Incoming peer poses arrive at
// /p2pcd/inbound (PRESENCE_MSG) and land in the presence map. The UI GETs the
// live peer poses to render their avatars. (Inside host-mediated relay and
// per-space scoping are a follow-up — this is the direct broadcast core.)

/// Decode a base64 peer id to the 32-byte array `send_msg` expects.
fn peer_id_bytes32(b64: &str) -> Option<[u8; 32]> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.decode(b64).ok()?.as_slice().try_into().ok()
}

async fn presence_post(State(state): State<AppState>, Json(mut pose): Json<Pose>) -> Response {
    // Remember which space the local player is in (drives the --test-peer follow).
    if !pose.space.is_empty() {
        *state.last_space.write().await = pose.space.clone();
    }
    // Stamp our home district so peers can fetch our avatar's aesthetic.
    let o = state.home_cell.octets;
    pose.home = format!("{}.{}.{}.0", o[0], o[1], o[2]);
    let payload = serde_json::to_vec(&pose).unwrap_or_default();
    let mut sent = 0usize;
    for ap in state.peers.peers().await {
        if let Some(bytes) = peer_id_bytes32(&ap.peer_id) {
            if state
                .bridge
                .send_msg(&bytes, PRESENCE_MSG, &payload)
                .await
                .is_ok()
            {
                sent += 1;
            }
        }
    }
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "broadcast_to": sent })),
    )
        .into_response()
}

/// Query for `GET /presence`. `spaces` (comma-separated) scopes the result to
/// peers standing in any of those spaces — the local district plus its loaded
/// neighbours for the stitched view. `space` is the single-space alias; omit
/// both to see every live peer.
#[derive(serde::Deserialize, Default)]
struct PresenceQuery {
    #[serde(default)]
    space: Option<String>,
    #[serde(default)]
    spaces: Option<String>,
}

async fn presence_get(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<PresenceQuery>,
) -> Response {
    let now = current_time_ms();
    // Set of requested spaces (from `spaces` or the single `space` alias).
    let wanted: Option<std::collections::HashSet<&str>> = q
        .spaces
        .as_deref()
        .map(|s| s.split(',').filter(|x| !x.is_empty()).collect())
        .or_else(|| q.space.as_deref().map(|s| std::iter::once(s).collect()));

    let map = state.presence.read().await;
    let peers: Vec<_> = map
        .iter()
        .filter(|(_, pp)| now.saturating_sub(pp.updated_ms) < PRESENCE_TTL_MS)
        // Scope to the requested spaces so a client only sees (and aligns with)
        // peers in districts it has loaded.
        .filter(|(_, pp)| wanted.as_ref().map_or(true, |set| set.contains(pp.pose.space.as_str())))
        .map(|(id, pp)| {
            serde_json::json!({
                "peer_id": id,
                "position": pp.pose.position,
                "orientation": pp.pose.orientation,
                "velocity": pp.pose.velocity,
                "space": pp.pose.space,
                "home": pp.pose.home,
                "age_ms": now.saturating_sub(pp.updated_ms),
            })
        })
        .collect();
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "peers": peers })),
    )
        .into_response()
}

/// Inject a synthetic peer that orbits slowly in whatever space the local UI
/// reports, refreshing its pose so it stays "live". Purely for single-node
/// spot-checks of multiplayer rendering (`--test-peer`).
fn spawn_test_peer(
    presence: PresenceMap,
    last_space: std::sync::Arc<tokio::sync::RwLock<String>>,
) {
    tokio::spawn(async move {
        const PEER_ID: &str = "7e57beef"; // hex-decodable, so /avatar/.../entity resolves
        const HOME: &str = "120.90.200.0"; // a distinct district → recognisable avatar hue
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(400));
        loop {
            ticker.tick().await;
            let space = last_space.read().await.clone();
            if space.is_empty() {
                continue; // wait until the UI reports a space to stand in
            }
            let now = current_time_ms();
            // Slow orbit around a point in front of the spawn camera, which sits
            // at the district seed looking toward -z.
            let angle = (now as f64 / 1000.0 * 0.6).rem_euclid(std::f64::consts::TAU);
            let (r, cz) = (4.0_f64, -10.0_f64);
            let pose = Pose {
                position: [r * angle.cos(), 1.5, cz + r * angle.sin()],
                orientation: [0.0, angle + std::f64::consts::FRAC_PI_2, 0.0],
                velocity: [0.0; 3],
                space,
                home: HOME.to_string(),
            };
            presence
                .write()
                .await
                .insert(PEER_ID.to_string(), PeerPose { pose, updated_ms: now });
        }
    });
}

// ─── Peer avatar (spaces §8.2) ──────────────────────────────────────────────
//
// A peer's avatar description graph, built from their peer id and home district
// (`:ip`) aesthetic. Peers fetch each other's avatars over the `avatar.get` RPC
// (handled in `inbound`); this HTTP form lets the renderer/clients fetch one by
// (home-ip, peer-id) and is the inspection/test surface.

async fn avatar_handler(AxumPath((ip, peer_id)): AxumPath<(String, String)>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let pid = match decode_peer_id(&peer_id) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "Invalid peer id").into_response(),
    };
    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let graph = hdl::mapping::map_avatar(&pid, &palette);
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "peer_id": peer_id, "description": graph })),
    )
        .into_response()
}

/// A peer's avatar as a renderable Astral entity (resolved geometry + material),
/// positioned at the origin — the presence client repositions it at the peer's
/// pose. Deterministic from (`ip` = home district, `peer_id`), so clients cache it.
async fn avatar_entity_handler(AxumPath((ip, peer_id)): AxumPath<(String, String)>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let pid = match decode_peer_id(&peer_id) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "Invalid peer id").into_response(),
    };
    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let entity = scene::compiler::compile_avatar(&pid, &palette, 0.0, 0.0, 0.0);
    (StatusCode::OK, axum::Json(entity)).into_response()
}

// ─── Peer Inside (spaces §2) ────────────────────────────────────────────────
//
// Generates a peer's interior: an entry hall plus one room per installed
// capability, laid out around the hall. The district `ip` supplies the
// aesthetic (palette); the peer id seeds the layout. `caps` defaults to the
// built-in social capability set when omitted.

const DEFAULT_INSIDE_CAPS: [&str; 5] = [
    "social.feed",
    "social.files",
    "social.messaging",
    "social.presence",
    "social.voice",
];

#[derive(serde::Deserialize)]
struct InsideParams {
    /// Comma-separated installed-capability names. Defaults to the social set.
    caps: Option<String>,
    /// Active peer-tunnel count (feeds the entry-hall area).
    tunnels: Option<usize>,
    // Room-feature counts (spaces §2.4) — stubbed live state per decision D3.
    feed_posts: Option<usize>,
    feed_unread: Option<usize>,
    message_threads: Option<usize>,
    messages: Option<usize>,
    files: Option<usize>,
}

fn feature_counts_from(p: &InsideParams) -> gen::room_features::FeatureCounts {
    gen::room_features::FeatureCounts {
        feed_posts: p.feed_posts.unwrap_or(0),
        feed_unread: p.feed_unread.unwrap_or(0),
        message_threads: p.message_threads.unwrap_or(0),
        messages: p.messages.unwrap_or(0),
        files: p.files.unwrap_or(0),
    }
}

async fn district_inside_handler(
    AxumPath((ip, peer_id)): AxumPath<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<InsideParams>,
) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let pid = match decode_peer_id(&peer_id) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "Invalid peer id").into_response(),
    };

    let caps: Vec<String> = match params.caps {
        Some(s) => s
            .split(',')
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect(),
        None => DEFAULT_INSIDE_CAPS.iter().map(|s| s.to_string()).collect(),
    };
    let tunnels = params.tunnels.unwrap_or(0);

    let inside = gen::inside::generate_inside(&cell, &pid, &caps, tunnels);
    (StatusCode::OK, axum::Json(inside)).into_response()
}

/// Renderable Astral scene for a peer's Inside (rooms → walls/floors/doors).
async fn district_inside_scene_handler(
    AxumPath((ip, peer_id)): AxumPath<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<InsideParams>,
) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let pid = match decode_peer_id(&peer_id) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "Invalid peer id").into_response(),
    };
    let caps: Vec<String> = match &params.caps {
        Some(s) => s
            .split(',')
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect(),
        None => DEFAULT_INSIDE_CAPS.iter().map(|s| s.to_string()).collect(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let inside = gen::inside::generate_inside(&cell, &pid, &caps, params.tunnels.unwrap_or(0));
    let mut scene = scene::compiler::compile_inside_scene(&inside, &palette);

    // Inject room-feature entities (feed posts, threads, files — spaces §2.4).
    let counts = feature_counts_from(&params);
    for f in gen::room_features::generate_room_features(&inside, &counts) {
        scene
            .entities
            .push(scene::compiler::compile_room_feature(&f, &palette));
    }

    (StatusCode::OK, axum::Json(scene)).into_response()
}

/// The room-feature list for a peer's Inside (spaces §2.4) — feed posts, message
/// threads, and files, with counts from query params (stubbed live state).
async fn district_inside_features_handler(
    AxumPath((ip, peer_id)): AxumPath<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<InsideParams>,
) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let pid = match decode_peer_id(&peer_id) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "Invalid peer id").into_response(),
    };
    let caps: Vec<String> = match params.caps {
        Some(ref s) => s
            .split(',')
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect(),
        None => DEFAULT_INSIDE_CAPS.iter().map(|s| s.to_string()).collect(),
    };
    let inside = gen::inside::generate_inside(&cell, &pid, &caps, params.tunnels.unwrap_or(0));
    let features = gen::room_features::generate_room_features(&inside, &feature_counts_from(&params));
    (StatusCode::OK, axum::Json(features)).into_response()
}

// ─── Underground tunnel (spaces §3) ─────────────────────────────────────────
//
// The 1-to-1 space between two connected peers. Geometry comes from connection
// metrics (latency→length, bandwidth→width, active caps→height); the aesthetic
// is a gradient lerp between the two peers' districts. Metrics are query params
// (stubbed defaults per decision D3 until the connection layer feeds them).

#[derive(serde::Deserialize)]
struct TunnelParams {
    latency: Option<f64>,
    bandwidth: Option<f64>,
    /// Mutually-active capability names (comma-separated) → markers + height.
    caps: Option<String>,
    uptime: Option<f64>,
}

fn tunnel_metrics_from(params: &TunnelParams) -> gen::tunnel::TunnelMetrics {
    let mut m = gen::tunnel::TunnelMetrics::default();
    if let Some(v) = params.latency {
        m.latency_ms = v;
    }
    if let Some(v) = params.bandwidth {
        m.bandwidth_kbps = v;
    }
    if let Some(v) = params.uptime {
        m.uptime = v;
    }
    if let Some(ref s) = params.caps {
        m.active_caps = s
            .split(',')
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect();
    }
    m
}

/// Parse both endpoints; returns the two cells + decoded peer ids, or an error
/// response.
fn parse_tunnel_endpoints(
    ip_a: &str,
    peer_a: &str,
    ip_b: &str,
    peer_b: &str,
) -> Result<(gen::cell::Cell, Vec<u8>, gen::cell::Cell, Vec<u8>), Response> {
    let cell_a = parse_cell(ip_a).ok_or_else(bad_request)?;
    let cell_b = parse_cell(ip_b).ok_or_else(bad_request)?;
    let pa = decode_peer_id(peer_a)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Invalid peer id").into_response())?;
    let pb = decode_peer_id(peer_b)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Invalid peer id").into_response())?;
    Ok((cell_a, pa, cell_b, pb))
}

async fn underground_handler(
    AxumPath((ip_a, peer_a, ip_b, peer_b)): AxumPath<(String, String, String, String)>,
    axum::extract::Query(params): axum::extract::Query<TunnelParams>,
) -> Response {
    let (cell_a, pa, cell_b, pb) = match parse_tunnel_endpoints(&ip_a, &peer_a, &ip_b, &peer_b) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let metrics = tunnel_metrics_from(&params);
    let tunnel = gen::tunnel::generate_tunnel(&cell_a, &pa, &cell_b, &pb, &metrics);
    (StatusCode::OK, axum::Json(tunnel)).into_response()
}

async fn underground_scene_handler(
    AxumPath((ip_a, peer_a, ip_b, peer_b)): AxumPath<(String, String, String, String)>,
    axum::extract::Query(params): axum::extract::Query<TunnelParams>,
) -> Response {
    let (cell_a, pa, cell_b, pb) = match parse_tunnel_endpoints(&ip_a, &peer_a, &ip_b, &peer_b) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let metrics = tunnel_metrics_from(&params);
    let tunnel = gen::tunnel::generate_tunnel(&cell_a, &pa, &cell_b, &pb, &metrics);
    let scene = scene::compiler::compile_tunnel_scene(&tunnel);
    (StatusCode::OK, axum::Json(scene)).into_response()
}

// ─── District prefetch (lightweight seed bundle) ────────────────────────────
//
// Declared in manifest.json as `district_prefetch`. Returns the center cell
// plus its 8 grid neighbours as geometry-free seed summaries. Clients streaming
// across district borders use this to warm palettes/keys for the district they
// are about to enter without paying for a full scene compile.

async fn district_prefetch_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let octets = cell.octets;
    let mut neighbors = Vec::new();

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
        neighbors.push(cell_summary(&ncell));
    }

    let response = serde_json::json!({
        "center": cell_summary(&cell),
        "neighbors": neighbors,
        "generated_at": current_time_ms(),
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
    let dd = gen::chunk::district_data(&cell);
    let district = dd.geometry.clone();
    let road_network = dd.roads.clone();
    let rivers = dd.rivers.clone();
    let blocks = dd.blocks.clone();

    let mut buildings = Vec::new();
    let mut fixtures = Vec::new();
    let mut flora = Vec::new();
    let mut creatures = Vec::new();
    let now_ms = current_time_ms();
    let is_night = gen::atmosphere::is_night(
        gen::atmosphere::compute_atmosphere(&cell, now_ms).time_of_day,
    );
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
        // Creatures with habitat-aware, time-synced, night-gated placement (so the
        // map reflects where they actually are — verification surface).
        let block_zones = gen::zones::generate_zones(cell.key, block);
        for pc in gen::creatures::place_creatures(&cell, block, &block_zones, now_ms, is_night) {
            creatures.push((
                pc.position,
                pc.creature.ecological_role.archetype_str().to_string(),
            ));
        }
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
        &creatures,
        &scene::map::MapConfig::default(),
    );

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}

// ─── Neighborhood map (3×3 districts) ──────────────────────────────────────

async fn neighborhood_map_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let svg = scene::map::generate_neighborhood_map(&cell);

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}

// ─── Structure audit (machine-checkable invariants) ─────────────────────────

async fn district_audit_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    (StatusCode::OK, axum::Json(audit::audit_district(&cell))).into_response()
}

async fn cross_audit_handler(AxumPath((ip_a, ip_b)): AxumPath<(String, String)>) -> Response {
    let (a, b) = match (parse_cell(&ip_a), parse_cell(&ip_b)) {
        (Some(a), Some(b)) => (a, b),
        _ => return bad_request(),
    };
    (StatusCode::OK, axum::Json(audit::audit_cross_cells(&a, &b))).into_response()
}

/// Audit a peer's spaces entities (home/inside/tunnel to a second peer).
async fn spaces_audit_handler(
    AxumPath((ip_a, peer_a, ip_b, peer_b)): AxumPath<(String, String, String, String)>,
) -> Response {
    let (cell_a, pa, cell_b, pb) = match parse_tunnel_endpoints(&ip_a, &peer_a, &ip_b, &peer_b) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    (
        StatusCode::OK,
        axum::Json(audit::audit_spaces(&cell_a, &pa, &cell_b, &pb)),
    )
        .into_response()
}

// ─── ASCII map (agent/terminal-inspectable) ─────────────────────────────────

async fn district_ascii_handler(AxumPath(ip): AxumPath<String>) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };
    let dd = gen::chunk::district_data(&cell);
    let dist = dd.geometry.clone();
    let roads = dd.roads.clone();
    let rivers = dd.rivers.clone();
    let blocks = dd.blocks.clone();
    let buildings: Vec<_> = blocks
        .iter()
        .flat_map(|b| gen::buildings::generate_buildings(&cell, b).plots)
        .collect();

    let txt = scene::ascii::generate_district_ascii(
        &cell,
        &dist.polygon,
        &blocks,
        &roads,
        &rivers,
        &buildings,
        &scene::ascii::AsciiConfig::default(),
    );
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        txt,
    )
        .into_response()
}

// ─── Astral Scene (compiled) ───────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SceneParams {
    /// Comma-separated peer ids (hex or base64) whose homes to place in the scene.
    homes: Option<String>,
}

async fn district_scene_handler(
    AxumPath(ip): AxumPath<String>,
    axum::extract::Query(params): axum::extract::Query<SceneParams>,
) -> Response {
    let cell = match parse_cell(&ip) {
        Some(c) => c,
        None => return bad_request(),
    };

    let palette = gen::aesthetic::AestheticPalette::from_cell(&cell);
    let now_ms = current_time_ms();
    let atmo = gen::atmosphere::compute_atmosphere(&cell, now_ms);

    let mut astral_scene = scene::compiler::compile_district_scene(&cell, &palette, &atmo, now_ms);

    // Optionally place peer homes into the rendered district (spaces §1.2).
    if let Some(homes) = params.homes {
        for pid in homes.split(',').filter(|s| !s.is_empty()).filter_map(decode_peer_id) {
            let home = gen::home::place_home_in_cell(&cell, &pid);
            // A portal beside each home → that peer's Inside (spaces §5.1).
            let portal = scene::compiler::compile_portal(
                &format!("inside:{:08x}", home.peer_id_u32),
                home.position.x + home.footprint_radius + 1.0,
                home.height * 0.4,
                home.position.y,
                palette.hue,
            );
            astral_scene
                .entities
                .push(scene::compiler::compile_home(&home, &palette));
            astral_scene.entities.push(portal);
        }
    }

    (StatusCode::OK, axum::Json(astral_scene)).into_response()
}

// ─── Main ──────────────────────────────────────────────────────────────────
//
// The cap runs behind the daemon proxy: a browser request to
// `/cap/world/district/X` is forwarded by the daemon with the `/cap/world/`
// prefix stripped, so routes are registered bare (`/district/{ip}`), exactly
// like the other capabilities. `/health`, `/p2pcd/inbound`, and `/ui/*` are
// provided by `CapabilityApp`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::parse();

    // P2P-CD plumbing: daemon bridge, live peer set, and our own peer id.
    let bridge = BridgeClient::new(config.daemon_port);
    let peers = PeerTracker::new(CAP_NAME);
    peers.init_from_daemon(&bridge).await;
    let _stream = PeerStream::drive_existing(peers.clone(), bridge.events_url(CAP_NAME), None, None);
    let local_id = LocalPeerId::lazy(bridge.clone()).await;
    let home_cell = parse_cell(&config.home_ip)
        .unwrap_or_else(|| gen::cell::Cell::from_octets(1, 0, 0));

    let state = AppState {
        bridge,
        peers,
        local_id,
        home_cell,
        presence: PresenceMap::default(),
        last_space: std::sync::Arc::new(tokio::sync::RwLock::new(String::new())),
    };

    if config.test_peer {
        tracing::info!("--test-peer: injecting a synthetic orbiting peer");
        spawn_test_peer(state.presence.clone(), state.last_space.clone());
    }

    CapabilityApp::new(CAP_NAME, config.port, state)
        .with_ui(&UI_DIR)
        .with_inbound_handler(inbound)
        .with_routes(|router| {
            router
                .route("/district/{ip}", get(district_handler))
                .route("/district/{ip}/geometry", get(district_geometry_handler))
                .route("/district/{ip}/objects", get(district_objects_handler))
                .route(
                    "/district/{ip}/atmosphere",
                    get(district_atmosphere_handler),
                )
                .route("/district/{ip}/prefetch", get(district_prefetch_handler))
                .route("/district/{ip}/home/{peer_id}", get(district_home_handler))
                .route(
                    "/district/{ip}/inside/{peer_id}",
                    get(district_inside_handler),
                )
                .route(
                    "/district/{ip}/inside/{peer_id}/scene",
                    get(district_inside_scene_handler),
                )
                .route(
                    "/district/{ip}/inside/{peer_id}/features",
                    get(district_inside_features_handler),
                )
                .route(
                    "/underground/{ip_a}/{peer_a}/{ip_b}/{peer_b}",
                    get(underground_handler),
                )
                .route(
                    "/underground/{ip_a}/{peer_a}/{ip_b}/{peer_b}/scene",
                    get(underground_scene_handler),
                )
                .route("/district/{ip}/scene", get(district_scene_handler))
                .route("/district/{ip}/map", get(district_map_handler))
                .route("/district/{ip}/map.txt", get(district_ascii_handler))
                .route("/district/{ip}/audit", get(district_audit_handler))
                .route("/audit/cross/{ip_a}/{ip_b}", get(cross_audit_handler))
                .route(
                    "/audit/spaces/{ip_a}/{peer_a}/{ip_b}/{peer_b}",
                    get(spaces_audit_handler),
                )
                .route(
                    "/district/{ip}/neighborhood",
                    get(neighborhood_map_handler),
                )
                .route("/district/{ip}/live", get(stream::handler::ws_handler))
                .route("/neighbors/{ip}", get(neighbors_handler))
                .route("/portal", get(portal_handler))
                .route("/avatar/{ip}/{peer_id}", get(avatar_handler))
                .route("/avatar/{ip}/{peer_id}/entity", get(avatar_entity_handler))
                .route("/presence", get(presence_get).post(presence_post))
        })
        .run()
        .await
}

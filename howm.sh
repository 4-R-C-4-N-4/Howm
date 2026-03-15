#!/usr/bin/env bash
# howm.sh — single-command launcher for the Howm P2P platform
#
# Usage:
#   ./howm.sh [OPTIONS]
#
# Options:
#   --port PORT             Daemon listen port (default: 7000)
#   --data-dir DIR          Data directory (default: ./data)
#   --name NAME             Node name (default: hostname)
#   --wg-port PORT          WireGuard listen port (default: 51820)
#   --wg-endpoint HOST:PORT Public WireGuard endpoint for peers
#   --no-wg                 Disable WireGuard (LAN-only mode)
#   --no-ui                 Skip the web UI
#   --no-social-feed        Skip building/installing the social-feed capability
#   --dev                   Pass --dev flag to daemon (enables CORS for Vite proxy)
#   --help                  Show this help
#
# Examples:
#   ./howm.sh                                          # start a standalone node
#   ./howm.sh --wg-endpoint myhost.com:51820           # node reachable at myhost.com
#   ./howm.sh --port 7010 --name node-b --data-dir ./data-b
#   ./howm.sh --no-wg                                  # LAN-only, no WireGuard
#   ./howm.sh --no-ui --no-social-feed                 # daemon-only, no extras

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Defaults ────────────────────────────────────────────────────────────────
PORT=7000
DATA_DIR="$ROOT_DIR/data"
NODE_NAME=""
WG_PORT=51820
WG_ENDPOINT=""
NO_WG=0
NO_UI=0
NO_SOCIAL_FEED=0
DEV_FLAG=""

# ── Parse args ──────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)              PORT="$2";           shift 2 ;;
        --data-dir)          DATA_DIR="$2";       shift 2 ;;
        --name)              NODE_NAME="$2";      shift 2 ;;
        --wg-port)           WG_PORT="$2";        shift 2 ;;
        --wg-endpoint)       WG_ENDPOINT="$2";    shift 2 ;;
        --no-wg)             NO_WG=1;             shift   ;;
        --no-ui)             NO_UI=1;             shift   ;;
        --no-social-feed)    NO_SOCIAL_FEED=1;    shift   ;;
        --dev)               DEV_FLAG="--dev";    shift   ;;
        --help|-h)
            grep '^#' "$0" | sed 's/^# \{0,2\}//'
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run ./howm.sh --help for usage."
            exit 1
            ;;
    esac
done

# ── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()    { echo -e "${CYAN}[howm]${NC} $*"; }
success() { echo -e "${GREEN}[howm]${NC} $*"; }
warn()    { echo -e "${YELLOW}[howm]${NC} $*"; }
error()   { echo -e "${RED}[howm]${NC} $*" >&2; }

# ── Check prerequisites ─────────────────────────────────────────────────────
check_cmd() {
    if ! command -v "$1" &>/dev/null; then
        error "Required command not found: $1"
        error "  $2"
        exit 1
    fi
}

check_cmd cargo "Install Rust from https://rustup.rs"

HAS_DOCKER=0
if command -v docker &>/dev/null && docker info &>/dev/null 2>&1; then
    HAS_DOCKER=1
fi

if [[ $NO_WG -eq 0 && $HAS_DOCKER -eq 0 ]]; then
    warn "Docker not available — WireGuard container cannot start."
    warn "Use --no-wg for LAN-only mode, or install Docker."
    NO_WG=1
fi

if [[ $NO_SOCIAL_FEED -eq 0 && $HAS_DOCKER -eq 0 ]]; then
    warn "Docker not available — skipping social-feed capability"
    NO_SOCIAL_FEED=1
fi

if [[ $NO_UI -eq 0 ]]; then
    if ! command -v npm &>/dev/null; then
        warn "npm not found — skipping web UI (use --no-ui to suppress)"
        NO_UI=1
    fi
fi

# ── Build daemon ─────────────────────────────────────────────────────────────
info "Building daemon (release)..."
cd "$ROOT_DIR/node"
cargo build --release 2>&1 | tail -3
DAEMON_BIN="$ROOT_DIR/node/target/release/daemon"
success "Daemon built: $DAEMON_BIN"

# ── Build web UI ─────────────────────────────────────────────────────────────
UI_PID=""
if [[ $NO_UI -eq 0 ]]; then
    info "Installing UI dependencies..."
    cd "$ROOT_DIR/ui/web"
    npm install --silent
    info "Starting web UI dev server..."
    npm run dev &
    UI_PID=$!
    success "Web UI starting on http://localhost:5173 (PID $UI_PID)"
fi

# ── Build social-feed image ─────────────────────────────────────────────────
SOCIAL_FEED_IMAGE="cap-social-feed:0.1"
SOCIAL_FEED_BUILT=0
if [[ $NO_SOCIAL_FEED -eq 0 ]]; then
    info "Building social-feed Docker image ($SOCIAL_FEED_IMAGE)..."
    cd "$ROOT_DIR"
    if docker build -q -t "$SOCIAL_FEED_IMAGE" capabilities/social-feed/; then
        success "social-feed image built: $SOCIAL_FEED_IMAGE"
        SOCIAL_FEED_BUILT=1
    else
        warn "Docker build failed — continuing without social-feed"
    fi
fi

# ── Start daemon ────────────────────────────────────────────────────────────
mkdir -p "$DATA_DIR"

DAEMON_ARGS=(--port "$PORT" --data-dir "$DATA_DIR")
[[ -n "$NODE_NAME" ]]     && DAEMON_ARGS+=(--name "$NODE_NAME")
[[ -n "$DEV_FLAG" ]]      && DAEMON_ARGS+=("$DEV_FLAG")

# WireGuard flags
if [[ $NO_WG -eq 1 ]]; then
    DAEMON_ARGS+=(--no-wg)
else
    DAEMON_ARGS+=(--wg-port "$WG_PORT")
    [[ -n "$WG_ENDPOINT" ]] && DAEMON_ARGS+=(--wg-endpoint "$WG_ENDPOINT")
fi

info "Starting daemon on port $PORT (data: $DATA_DIR)..."
cd "$ROOT_DIR"
"$DAEMON_BIN" "${DAEMON_ARGS[@]}" &
DAEMON_PID=$!
success "Daemon started (PID $DAEMON_PID)"

# ── Wait for daemon to be ready ─────────────────────────────────────────────
info "Waiting for daemon to accept connections..."
for i in $(seq 1 60); do
    if curl -sf "http://localhost:$PORT/node/info" &>/dev/null; then
        break
    fi
    sleep 1
    if [[ $i -eq 60 ]]; then
        error "Daemon did not start within 60 seconds"
        kill "$DAEMON_PID" 2>/dev/null || true
        exit 1
    fi
done

# Read the API token for authenticated requests
API_TOKEN=""
if [[ -f "$DATA_DIR/api_token" ]]; then
    API_TOKEN=$(cat "$DATA_DIR/api_token")
fi

success "Daemon is ready at http://localhost:$PORT"

# ── Install social-feed capability ──────────────────────────────────────────
if [[ $SOCIAL_FEED_BUILT -eq 1 ]]; then
    info "Installing social-feed capability..."
    RESP=$(curl -sf -X POST "http://localhost:$PORT/capabilities/install" \
        -H 'Content-Type: application/json' \
        -H "Authorization: Bearer $API_TOKEN" \
        -d "{\"image\":\"$SOCIAL_FEED_IMAGE\"}" || true)
    if echo "$RESP" | grep -q '"name"'; then
        success "social-feed installed and running"
    else
        warn "social-feed install returned unexpected response: $RESP"
    fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}┌─────────────────────────────────────────────────┐${NC}"
echo -e "${GREEN}│  Howm is running                                │${NC}"
echo -e "${GREEN}├─────────────────────────────────────────────────┤${NC}"
printf "${GREEN}│${NC}  Daemon API:  http://localhost:%-17s${GREEN}│${NC}\n" "$PORT"
if [[ -n "$API_TOKEN" ]]; then
printf "${GREEN}│${NC}  API Token:   %-33s${GREEN}│${NC}\n" "$API_TOKEN"
fi
if [[ $NO_UI -eq 0 ]]; then
echo -e "${GREEN}│${NC}  Web UI:      http://localhost:5173              ${GREEN}│${NC}"
fi
if [[ $SOCIAL_FEED_BUILT -eq 1 ]]; then
printf "${GREEN}│${NC}  Social feed: http://localhost:%s/cap/social/feed\n" "$PORT"
fi
if [[ $NO_WG -eq 0 ]]; then
  WG_INFO="WG port $WG_PORT"
  [[ -n "$WG_ENDPOINT" ]] && WG_INFO="$WG_ENDPOINT"
  printf "${GREEN}│${NC}  WireGuard:   %-33s${GREEN}│${NC}\n" "$WG_INFO"
else
  echo -e "${GREEN}│${NC}  WireGuard:   disabled (LAN-only)                ${GREEN}│${NC}"
fi
echo -e "${GREEN}│                                                 │${NC}"
echo -e "${GREEN}│${NC}  Press Ctrl+C to stop all processes             ${GREEN}│${NC}"
echo -e "${GREEN}└─────────────────────────────────────────────────┘${NC}"
echo ""

# ── Cleanup on exit ──────────────────────────────────────────────────────────
cleanup() {
    echo ""
    info "Shutting down..."
    kill "$DAEMON_PID" 2>/dev/null || true
    [[ -n "$UI_PID" ]] && kill "$UI_PID" 2>/dev/null || true
    # Stop capability containers (daemon shutdown handler does this too)
    docker ps --filter "name=howm-cap-" --format "{{.ID}}" 2>/dev/null \
        | xargs -r docker stop &>/dev/null || true
    docker ps --filter "name=howm-wg-" --format "{{.ID}}" 2>/dev/null \
        | xargs -r docker stop &>/dev/null || true
    info "Done."
}

trap cleanup EXIT INT TERM

# ── Wait ─────────────────────────────────────────────────────────────────────
wait "$DAEMON_PID" 2>/dev/null || true

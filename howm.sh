#!/usr/bin/env bash
# howm.sh — single-command launcher for the Howm P2P platform
#
# Usage:
#   ./howm.sh [OPTIONS]
#
# Options:
#   --port PORT             Daemon listen port (default: 7000)
#   --data-dir DIR          Data directory (default: ~/.local/howm)
#   --name NAME             Node name (default: hostname)
#   --wg-port PORT          WireGuard listen port (default: 41641)
#   --wg-endpoint HOST:PORT Public WireGuard endpoint for peers
#   --no-ui                 Skip the web UI
#   --dev                   Pass --dev flag to daemon (enables CORS for Vite proxy)
#   --debug                 Show daemon logs in the foreground
#   --release               Build in release mode (default: debug)
#   --help                  Show this help
#
# Examples:
#   ./howm.sh                                          # start a standalone node
#   ./howm.sh --wg-endpoint myhost.com:51820           # node reachable at myhost.com
#   ./howm.sh --port 7010 --name node-b --data-dir /tmp/howm-b
#   ./howm.sh --no-ui                                  # daemon-only, no web UI

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Defaults ────────────────────────────────────────────────────────────────
PORT=7000
DATA_DIR=""
NODE_NAME=""
WG_PORT=41641
WG_ENDPOINT=""
NO_UI=0
DEV_FLAG=""
DEBUG_FLAG=""
RELEASE_MODE=0

# ── Parse args ──────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)              PORT="$2";           shift 2 ;;
        --data-dir)          DATA_DIR="$2";       shift 2 ;;
        --name)              NODE_NAME="$2";      shift 2 ;;
        --wg-port)           WG_PORT="$2";        shift 2 ;;
        --wg-endpoint)       WG_ENDPOINT="$2";    shift 2 ;;
        --no-ui)             NO_UI=1;             shift   ;;
        --dev)               DEV_FLAG="--dev";    shift   ;;
        --debug)             DEBUG_FLAG="--debug"; shift  ;;
        --release)           RELEASE_MODE=1;      shift   ;;
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
check_cmd wg "Install wireguard-tools (e.g. pacman -S wireguard-tools)"

if [[ $NO_UI -eq 0 ]]; then
    if ! command -v npm &>/dev/null; then
        warn "npm not found — skipping web UI (use --no-ui to suppress)"
        NO_UI=1
    fi
fi

# ── Build web UI ─────────────────────────────────────────────────────────────
# In production mode the UI is compiled into the binary via include_dir!, so it
# must be built BEFORE cargo build.  In dev mode Vite serves it separately.
if [[ $NO_UI -eq 0 ]]; then
    info "Installing UI dependencies..."
    cd "$ROOT_DIR/ui/web"
    npm install --silent
    if [[ -n "$DEV_FLAG" ]]; then
        info "Starting web UI dev server..."
        npm run dev &
        UI_PID=$!
        success "Web UI dev server starting on http://localhost:5173 (PID $UI_PID)"
    else
        info "Building web UI (production)..."
        npm run build 2>&1 | tail -3
        success "Web UI built — will be embedded into the binary"
    fi
    cd "$ROOT_DIR"
fi

# ── Build daemon ─────────────────────────────────────────────────────────────
BUILD_EXIT=0
if [[ $RELEASE_MODE -eq 1 ]]; then
    info "Building howm (release)..."
    BUILD_OUT=$(cd "$ROOT_DIR/node" && cargo build --release 2>&1) || BUILD_EXIT=$?
    HOWM_BIN="$ROOT_DIR/node/target/release/howm"
else
    info "Building howm (debug)..."
    BUILD_OUT=$(cd "$ROOT_DIR/node" && cargo build 2>&1) || BUILD_EXIT=$?
    HOWM_BIN="$ROOT_DIR/node/target/debug/howm"
fi

if [[ $BUILD_EXIT -ne 0 ]]; then
    error "Build failed (exit $BUILD_EXIT):"
    echo "$BUILD_OUT"
    exit 1
fi
if echo "$BUILD_OUT" | grep -q "Compiling howm"; then
    success "Howm rebuilt (source changes detected)"
else
    success "Howm up to date (no changes)"
fi

# ── Start daemon ────────────────────────────────────────────────────────────
DAEMON_ARGS=(--port "$PORT")
[[ -n "$DATA_DIR" ]]      && DAEMON_ARGS+=(--data-dir "$DATA_DIR")
[[ -n "$NODE_NAME" ]]     && DAEMON_ARGS+=(--name "$NODE_NAME")
[[ -n "$DEV_FLAG" ]]      && DAEMON_ARGS+=("$DEV_FLAG")
[[ -n "$DEBUG_FLAG" ]]    && DAEMON_ARGS+=("$DEBUG_FLAG")

# WireGuard flags
DAEMON_ARGS+=(--wg-port "$WG_PORT")
[[ -n "$WG_ENDPOINT" ]] && DAEMON_ARGS+=(--wg-endpoint "$WG_ENDPOINT")

# Kill any stale howm process on this port
STALE_PID=$(lsof -ti "tcp:$PORT" 2>/dev/null || true)
if [[ -n "$STALE_PID" ]]; then
    warn "Port $PORT already in use (PID $STALE_PID) — killing stale process"
    kill "$STALE_PID" 2>/dev/null || true
    sleep 1
    # Force-kill if still alive
    kill -9 "$STALE_PID" 2>/dev/null || true
    sleep 0.5
fi

info "Starting howm on port $PORT..."
cd "$ROOT_DIR"
"$HOWM_BIN" "${DAEMON_ARGS[@]}" &
DAEMON_PID=$!
success "Howm started (PID $DAEMON_PID)"

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
EFFECTIVE_DATA_DIR="${DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/howm}"
if [[ -f "$EFFECTIVE_DATA_DIR/api_token" ]]; then
    API_TOKEN=$(cat "$EFFECTIVE_DATA_DIR/api_token")
fi

success "Howm is ready at http://localhost:$PORT"

# ── Build & install capabilities ──────────────────────────────────────────────
CAP_DIR="$ROOT_DIR/capabilities"
if [[ -d "$CAP_DIR" ]] && [[ -n "$API_TOKEN" ]]; then
    for cap in "$CAP_DIR"/*/manifest.json; do
        [[ -f "$cap" ]] || continue
        cap_root="$(dirname "$cap")"
        cap_name="$(basename "$cap_root")"

        # Build the capability (Cargo project) — skip if binary already exists
        if [[ -f "$cap_root/Cargo.toml" ]]; then
            if [[ $RELEASE_MODE -eq 1 ]]; then
                CAP_BIN="$cap_root/target/release/$cap_name"
            else
                CAP_BIN="$cap_root/target/debug/$cap_name"
            fi
            # Also accept the release binary when running in debug mode
            if [[ ! -x "$CAP_BIN" ]] && [[ -x "$cap_root/target/release/$cap_name" ]]; then
                CAP_BIN="$cap_root/target/release/$cap_name"
                info "Capability '$cap_name': using existing release binary"
            elif [[ -x "$CAP_BIN" ]]; then
                info "Capability '$cap_name': binary up to date"
            else
                CAP_BUILD_EXIT=0
                if [[ $RELEASE_MODE -eq 1 ]]; then
                    info "Building capability '$cap_name' (release)..."
                    (cd "$cap_root" && cargo build --release 2>&1) || CAP_BUILD_EXIT=$?
                else
                    info "Building capability '$cap_name' (debug)..."
                    (cd "$cap_root" && cargo build 2>&1) || CAP_BUILD_EXIT=$?
                fi
                if [[ $CAP_BUILD_EXIT -ne 0 ]]; then
                    warn "Capability '$cap_name' build failed (exit $CAP_BUILD_EXIT) — skipping"
                    continue
                fi
            fi
        fi

        # Install via the daemon API (idempotent — will 400 if already installed)
        INSTALL_RESP=$(curl -sf -X POST "http://localhost:$PORT/capabilities/install" \
            -H "Authorization: Bearer $API_TOKEN" \
            -H "Content-Type: application/json" \
            -d "{\"path\": \"$cap_root\"}" 2>&1) || true
        if echo "$INSTALL_RESP" | grep -q '"capability"'; then
            success "Capability '$cap_name' installed"
        elif echo "$INSTALL_RESP" | grep -q 'already installed'; then
            info "Capability '$cap_name' already registered"
        else
            warn "Capability '$cap_name' install: $INSTALL_RESP"
        fi
    done
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}┌─────────────────────────────────────────────────┐${NC}"
echo -e "${GREEN}│  Howm is running                                │${NC}"
echo -e "${GREEN}├─────────────────────────────────────────────────┤${NC}"
if [[ -n "$API_TOKEN" ]]; then
printf "${GREEN}│${NC}  API Token:   %-33s${GREEN}│${NC}\n" "$API_TOKEN"
fi
if [[ $NO_UI -eq 0 ]]; then
  if [[ -n "$DEV_FLAG" ]]; then
    echo -e "${GREEN}│${NC}  Web UI:      http://localhost:5173              ${GREEN}│${NC}"
  else
    printf "${GREEN}│${NC}  Web UI:      http://localhost:%-17s${GREEN}│${NC}\n" "$PORT"
  fi
fi
WG_INFO="WG port $WG_PORT"
[[ -n "$WG_ENDPOINT" ]] && WG_INFO="$WG_ENDPOINT"
printf "${GREEN}│${NC}  WireGuard:    %-33s${GREEN}│${NC}\n" "$WG_INFO"
echo -e "${GREEN}│                                                 │${NC}"
echo -e "${GREEN}│${NC}  Press Ctrl+C to stop                           ${GREEN}│${NC}"
echo -e "${GREEN}└─────────────────────────────────────────────────┘${NC}"
echo ""

# ── Cleanup on exit ──────────────────────────────────────────────────────────
cleanup() {
    echo ""
    info "Shutting down..."
    # Send SIGTERM to daemon (triggers graceful shutdown internally)
    kill "$DAEMON_PID" 2>/dev/null || true
    # Wait briefly for graceful shutdown
    for _ in $(seq 1 10); do
        kill -0 "$DAEMON_PID" 2>/dev/null || break
        sleep 0.5
    done
    # Force-kill if still alive
    kill -9 "$DAEMON_PID" 2>/dev/null || true
    [[ -n "${UI_PID:-}" ]] && kill "$UI_PID" 2>/dev/null || true
    info "Done."
}

trap cleanup EXIT INT TERM

# ── Wait ─────────────────────────────────────────────────────────────────────
wait "$DAEMON_PID" 2>/dev/null || true

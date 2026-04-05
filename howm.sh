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
#   --debug-log             Show daemon logs in the foreground
#   --debug                 Build in debug mode instead of release (default: release)
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
RELEASE_MODE=1

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
        --debug-log)         DEBUG_FLAG="--debug"; shift  ;;
        --debug)             RELEASE_MODE=0;      shift   ;;
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
    # Remove stale release binaries for each capability so the daemon install
    # logic falls through to the freshly-built debug binary instead of the old release.
    for cap in "$ROOT_DIR/capabilities"/*/; do
        cap_name="$(basename "$cap")"
        stale_release="$cap/target/release/$cap_name"
        [[ -f "$stale_release" ]] && rm -f "$stale_release"
    done
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

# Kill any stale howm process.
# Must check BOTH the HTTP port and the P2P-CD listener port (7654) because the
# old daemon holds both.  Killing only the HTTP port leaves port 7654 occupied,
# which causes the new P2P-CD engine to fail on bind and the peer sessions to drop.
P2PCD_PORT=7654
STALE_PIDS=$(lsof -t -i "tcp:$PORT" -i "tcp:$P2PCD_PORT" 2>/dev/null | sort -u || true)
if [[ -n "$STALE_PIDS" ]]; then
    for _pid in $STALE_PIDS; do
        warn "Port $PORT/$P2PCD_PORT already in use (PID $_pid) — killing stale process"
        kill "$_pid" 2>/dev/null || true
    done
    sleep 1
    # Force-kill if still alive
    for _pid in $STALE_PIDS; do
        kill -9 "$_pid" 2>/dev/null || true
    done
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
# The daemon uses dirs::data_local_dir() which resolves to:
#   Linux:  $XDG_DATA_HOME/howm  (default: ~/.local/share/howm)
#   macOS:  ~/Library/Application Support/howm
API_TOKEN=""
if [[ -n "$DATA_DIR" ]]; then
    EFFECTIVE_DATA_DIR="$DATA_DIR"
elif [[ "$(uname)" == "Darwin" ]]; then
    EFFECTIVE_DATA_DIR="$HOME/Library/Application Support/howm"
else
    EFFECTIVE_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/howm"
fi
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
        # The daemon uses the manifest "name" field (e.g. "social.feed"),
        # not the directory name (e.g. "feed"), for API routes.
        cap_api_name=$(grep -o '"name"[[:space:]]*:[[:space:]]*"[^"]*"' "$cap" | head -1 | sed 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')

        # Build the capability (Cargo project).
        # Always run cargo build — it's incremental and a no-op when unchanged.
        # This ensures source changes (including embedded UI assets) are picked up.
        if [[ -f "$cap_root/Cargo.toml" ]]; then
            # Force rebuild — cargo doesn't track embedded UI assets (include_dir!),
            # so touch main.rs to guarantee recompilation picks up UI changes.
            touch "$cap_root/src/main.rs" 2>/dev/null || true

            CAP_BUILD_EXIT=0
            if [[ $RELEASE_MODE -eq 1 ]]; then
                info "Building capability '$cap_name' (release)..."
                CAP_BUILD_OUT=$(cd "$cap_root" && cargo build --release 2>&1) || CAP_BUILD_EXIT=$?
                CAP_BIN="$cap_root/target/release/$cap_name"
            else
                info "Building capability '$cap_name' (debug)..."
                CAP_BUILD_OUT=$(cd "$cap_root" && cargo build 2>&1) || CAP_BUILD_EXIT=$?
                CAP_BIN="$cap_root/target/debug/$cap_name"
            fi
            if [[ $CAP_BUILD_EXIT -ne 0 ]]; then
                warn "Capability '$cap_name' build failed (exit $CAP_BUILD_EXIT) — skipping"
                echo "$CAP_BUILD_OUT" | tail -5
                continue
            fi
            if echo "$CAP_BUILD_OUT" | grep -q "Compiling"; then
                success "Capability '$cap_name' rebuilt"
            else
                info "Capability '$cap_name' up to date"
            fi
        fi

        # Install via the daemon API.
        # Always uninstall first to pick up manifest/binary changes cleanly.
        curl -sf -X DELETE "http://localhost:$PORT/capabilities/$cap_api_name" \
            -H "Authorization: Bearer $API_TOKEN" &>/dev/null || true
        sleep 2

        # Install with retry + exponential backoff (daemon may rate-limit or still be cleaning up)
        INSTALLED=0
        for attempt in 1 2 3 4 5; do
            INSTALL_RESP=$(curl -s -X POST "http://localhost:$PORT/capabilities/install" \
                -H "Authorization: Bearer $API_TOKEN" \
                -H "Content-Type: application/json" \
                -d "{\"path\": \"$cap_root\"}" 2>&1) || true
            if echo "$INSTALL_RESP" | grep -q '"capability"'; then
                success "Capability '$cap_name' installed"
                INSTALLED=1
                break
            fi
            BACKOFF=$((attempt * 2))
            [[ $attempt -lt 5 ]] && { info "Capability '$cap_name' install attempt $attempt failed, retrying in ${BACKOFF}s..."; sleep "$BACKOFF"; }
        done
        if [[ $INSTALLED -eq 0 ]]; then
            warn "Capability '$cap_name' install failed after 5 attempts: $INSTALL_RESP"
        fi
    done
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}┌─────────────────────────────────────────────────┐${NC}"
echo -e "${GREEN}│  Howm is running                                │${NC}"
echo -e "${GREEN}├─────────────────────────────────────────────────┤${NC}"
if [[ -n "$API_TOKEN" ]]; then
printf "${GREEN}│${NC}  API Token:   %-33s${GREEN}│${NC}\n" "${API_TOKEN:0:8}… (see $EFFECTIVE_DATA_DIR/api_token)"
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

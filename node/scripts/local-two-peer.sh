#!/usr/bin/env bash
# local-two-peer.sh — Run two howm daemon instances on loopback for manual integration testing.
#
# No WireGuard needed. Both peers run on 127.0.0.1 with different ports.
# Capabilities are signalled by editing the TOML files below.
#
# Usage:
#   ./scripts/local-two-peer.sh           # start both, Ctrl-C to stop
#   ./scripts/local-two-peer.sh --logs    # tail logs after starting
#
# Requirements:
#   cargo build --bin howm (or pass HOWM_BIN=/path/to/howm)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOWM_BIN="${HOWM_BIN:-$REPO_ROOT/target/debug/howm}"

ALICE_DIR="/tmp/howm-alice"
BOB_DIR="/tmp/howm-bob"
ALICE_P2PCD_PORT=17654
BOB_P2PCD_PORT=17655
ALICE_HTTP_PORT=17000
BOB_HTTP_PORT=17001

# ── Build if needed ────────────────────────────────────────────────────────────
if [[ ! -f "$HOWM_BIN" ]]; then
  echo "==> Building howm..."
  cargo build --manifest-path "$REPO_ROOT/daemon/Cargo.toml" --bin howm 2>&1
fi

# ── Config dirs ───────────────────────────────────────────────────────────────
mkdir -p "$ALICE_DIR" "$BOB_DIR"

cat > "$ALICE_DIR/peer.toml" <<TOML
[identity]
display_name = "alice"

[transport]
listen_port = $ALICE_P2PCD_PORT
wireguard_interface = "lo"
http_port = $ALICE_HTTP_PORT

[discovery]
poll_interval_ms = 500

# Tell Alice where Bob is (bypasses WG peer discovery)
[peer_addr_overrides]
"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbg=" = "127.0.0.1:$BOB_P2PCD_PORT"

[capabilities.heartbeat]
name = "core.heartbeat.liveness.1"
role = "both"
mutual = true

[capabilities.heartbeat.params]
interval_ms = 2000
timeout_ms  = 6000

[capabilities.social]
name  = "howm.social.feed.1"
role  = "both"
mutual = true

[data]
dir = "$ALICE_DIR/data"
TOML

cat > "$BOB_DIR/peer.toml" <<TOML
[identity]
display_name = "bob"

[transport]
listen_port = $BOB_P2PCD_PORT
wireguard_interface = "lo"
http_port = $BOB_HTTP_PORT

[discovery]
poll_interval_ms = 500

[peer_addr_overrides]
"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaag=" = "127.0.0.1:$ALICE_P2PCD_PORT"

[capabilities.heartbeat]
name = "core.heartbeat.liveness.1"
role = "both"
mutual = true

[capabilities.heartbeat.params]
interval_ms = 2000
timeout_ms  = 6000

[capabilities.social]
name  = "howm.social.feed.1"
role  = "both"
mutual = true

[data]
dir = "$BOB_DIR/data"
TOML

mkdir -p "$ALICE_DIR/data" "$BOB_DIR/data"

# ── Launch ────────────────────────────────────────────────────────────────────
echo "==> Starting Alice  (P2PCD :$ALICE_P2PCD_PORT  HTTP :$ALICE_HTTP_PORT)"
HOWM_CONFIG="$ALICE_DIR/peer.toml" RUST_LOG=howm=debug \
  "$HOWM_BIN" > "$ALICE_DIR/howm.log" 2>&1 &
ALICE_PID=$!

echo "==> Starting Bob    (P2PCD :$BOB_P2PCD_PORT  HTTP :$BOB_HTTP_PORT)"
HOWM_CONFIG="$BOB_DIR/peer.toml" RUST_LOG=howm=debug \
  "$HOWM_BIN" > "$BOB_DIR/howm.log" 2>&1 &
BOB_PID=$!

cleanup() {
  echo ""
  echo "==> Stopping Alice ($ALICE_PID) and Bob ($BOB_PID)..."
  kill "$ALICE_PID" "$BOB_PID" 2>/dev/null || true
  wait "$ALICE_PID" "$BOB_PID" 2>/dev/null || true
  echo "==> Done."
}
trap cleanup EXIT INT TERM

echo ""
echo "  Alice log : $ALICE_DIR/howm.log"
echo "  Bob   log : $BOB_DIR/howm.log"
echo ""
echo "  Alice API : http://127.0.0.1:$ALICE_HTTP_PORT"
echo "  Bob   API : http://127.0.0.1:$BOB_HTTP_PORT"
echo ""
echo "  Useful checks:"
echo "    curl http://127.0.0.1:$ALICE_HTTP_PORT/p2pcd/sessions"
echo "    curl http://127.0.0.1:$BOB_HTTP_PORT/p2pcd/sessions"
echo "    curl http://127.0.0.1:$ALICE_HTTP_PORT/p2pcd/capabilities"
echo ""
echo "  Press Ctrl-C to stop."

if [[ "${1:-}" == "--logs" ]]; then
  sleep 1
  tail -f "$ALICE_DIR/howm.log" "$BOB_DIR/howm.log"
else
  wait "$ALICE_PID" "$BOB_PID"
fi

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$SCRIPT_DIR/common.sh"

DAEMON_PIDS=()
TEST_DATA_DIR="$ROOT_DIR/test-data"
FAILED=0

trap cleanup EXIT

echo "=== Howm Integration Test ==="
echo ""

# 1. Build daemon
echo "--- Building daemon ---"
cd "$ROOT_DIR/node"
cargo build --release -p daemon 2>&1 | tail -3
DAEMON_BIN="$ROOT_DIR/node/target/release/daemon"

# Check Docker availability
HAS_DOCKER=0
if command -v docker &>/dev/null && docker info &>/dev/null 2>&1; then
    HAS_DOCKER=1
fi

SKIP_DOCKER=0
if [[ $HAS_DOCKER -eq 1 ]]; then
    echo "--- Building social-feed Docker image ---"
    if ! docker build -q -t cap-social-feed:0.1 "$ROOT_DIR/capabilities/social-feed/"; then
        echo "WARNING: Docker build failed. Skipping Docker-dependent tests."
        SKIP_DOCKER=1
    fi
else
    echo "WARNING: Docker not available. Skipping Docker-dependent tests."
    SKIP_DOCKER=1
fi

# 2. Start 3 daemon instances (WG disabled for integration tests)
echo ""
echo "--- Starting 3 daemon instances ---"

mkdir -p "$TEST_DATA_DIR/a" "$TEST_DATA_DIR/b" "$TEST_DATA_DIR/c"

"$DAEMON_BIN" --port 7000 --data-dir "$TEST_DATA_DIR/a" --name "node-a" --wg-enabled false &
DAEMON_PIDS+=($!)
echo "node-a started (PID ${DAEMON_PIDS[-1]}, port 7000)"

"$DAEMON_BIN" --port 7010 --data-dir "$TEST_DATA_DIR/b" --name "node-b" --wg-enabled false &
DAEMON_PIDS+=($!)
echo "node-b started (PID ${DAEMON_PIDS[-1]}, port 7010)"

"$DAEMON_BIN" --port 7020 --data-dir "$TEST_DATA_DIR/c" --name "node-c" --no-wg &
DAEMON_PIDS+=($!)
echo "node-c started (PID ${DAEMON_PIDS[-1]}, port 7020)"

# Wait for all three to be ready
wait_for_port 7000 30
wait_for_port 7010 30
wait_for_port 7020 30

# Read API tokens
TOKEN_A=$(read_token "$TEST_DATA_DIR/a")
TOKEN_B=$(read_token "$TEST_DATA_DIR/b")
TOKEN_C=$(read_token "$TEST_DATA_DIR/c")
echo "API tokens loaded for all 3 nodes"

echo ""
echo "--- Test: Identity ---"
assert_json "http://localhost:7000/node/info" ".name" "node-a"
assert_json "http://localhost:7010/node/info" ".name" "node-b"
assert_json "http://localhost:7020/node/info" ".name" "node-c"

# Extract node IDs for later
NODE_A_ID=$(curl -sf "http://localhost:7000/node/info" | jq -r ".node_id")
NODE_B_ID=$(curl -sf "http://localhost:7010/node/info" | jq -r ".node_id")
echo "node-a ID: $NODE_A_ID"
echo "node-b ID: $NODE_B_ID"

echo ""
echo "--- Test: WireGuard Status ---"
assert_json "http://localhost:7000/node/wireguard" ".status" "disabled"

echo ""
echo "--- Test: Bearer Auth Required ---"
# POST without auth should fail
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
    "http://localhost:7000/node/invite" \
    -H 'Content-Type: application/json' -d '{}')
if [[ "$HTTP_CODE" == "401" ]]; then
    echo "PASS: POST /node/invite without token returns 401"
else
    echo "FAIL: Expected 401, got $HTTP_CODE"
    FAILED=1
fi

# POST with correct auth should succeed
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
    "http://localhost:7000/node/invite" \
    -H 'Content-Type: application/json' \
    -H "Authorization: Bearer $TOKEN_A" \
    -d '{}')
if [[ "$HTTP_CODE" == "200" ]]; then
    echo "PASS: POST /node/invite with correct token returns 200"
else
    echo "FAIL: Expected 200, got $HTTP_CODE"
    FAILED=1
fi

# GET should work without auth
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:7000/node/info")
if [[ "$HTTP_CODE" == "200" ]]; then
    echo "PASS: GET /node/info without token returns 200"
else
    echo "FAIL: Expected 200, got $HTTP_CODE"
    FAILED=1
fi

echo ""
echo "--- Test: Invite System ---"
INVITE=$(post_json "http://localhost:7000/node/invite" '{}' "$TOKEN_A" | jq -r ".invite_code")
echo "Generated invite: ${INVITE:0:60}..."
if [[ "$INVITE" == howm://invite/* ]]; then
    echo "PASS: Invite code has correct format"
else
    echo "FAIL: Unexpected invite format: $INVITE"
    FAILED=1
fi

echo ""
echo "--- Test: Complete Invite (PSK-based, no bearer needed) ---"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
    "http://localhost:7000/node/complete-invite" \
    -H 'Content-Type: application/json' \
    -d '{"psk":"fake-psk","my_pubkey":"test","my_endpoint":"1.2.3.4:51820","my_wg_address":"10.47.0.99"}')
# This should return 410 Gone (fake PSK doesn't match any invite) — not 401
if [[ "$HTTP_CODE" == "410" ]]; then
    echo "PASS: complete-invite with bad PSK returns 410 (not 401)"
else
    echo "FAIL: Expected 410, got $HTTP_CODE"
    FAILED=1
fi

if [ "$SKIP_DOCKER" = "1" ]; then
    echo ""
    echo "--- Skipping Docker-dependent capability tests ---"
else
    echo ""
    echo "--- Test: Install Social Feed Capability on node-a ---"
    INSTALL_RESP=$(post_json "http://localhost:7000/capabilities/install" \
        '{"image":"cap-social-feed:0.1"}' "$TOKEN_A")
    echo "Install response: $INSTALL_RESP"
    sleep 3  # Wait for container to start
    assert_json "http://localhost:7000/capabilities" ".capabilities[0].name" "social.feed"
    assert_json "http://localhost:7000/capabilities" ".capabilities[0].status" "Running"

    echo ""
    echo "--- Test: Post via proxy on node-a ---"
    post_json "http://localhost:7000/cap/social/post" '{"content":"hello from node-a"}' > /dev/null
    assert_json_contains "http://localhost:7000/cap/social/feed" ".posts[0].content" "hello from node-a"

    echo ""
    echo "--- Test: Health check ---"
    assert_json "http://localhost:7000/cap/social/health" ".status" "ok"
fi

echo ""
if [[ $FAILED -eq 0 ]]; then
    echo "=== ALL TESTS PASSED ==="
    exit 0
else
    echo "=== SOME TESTS FAILED ==="
    exit 1
fi

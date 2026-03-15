#!/usr/bin/env bash
# Common helpers for Howm integration tests

# wait_for_port <port> <timeout_seconds>
# Polls until port accepts connections or timeout
wait_for_port() {
    local port=$1
    local timeout=${2:-30}
    local elapsed=0
    echo "Waiting for port $port..."
    while ! nc -z localhost "$port" 2>/dev/null; do
        sleep 1
        elapsed=$((elapsed + 1))
        if [ "$elapsed" -ge "$timeout" ]; then
            echo "ERROR: Port $port did not open within ${timeout}s"
            return 1
        fi
    done
    echo "Port $port is ready"
}

# assert_json <url> <jq_filter> <expected>
# Curls the URL and checks that jq_filter output equals expected
assert_json() {
    local url=$1
    local filter=$2
    local expected=$3
    local actual
    actual=$(curl -sf "$url" | jq -r "$filter" 2>/dev/null)
    if [ "$actual" = "$expected" ]; then
        echo "PASS: $url | $filter == $expected"
    else
        echo "FAIL: $url | $filter"
        echo "  Expected: $expected"
        echo "  Actual:   $actual"
        return 1
    fi
}

# assert_json_contains <url> <jq_filter> <substring>
# Checks that the jq output contains the substring
assert_json_contains() {
    local url=$1
    local filter=$2
    local substring=$3
    local actual
    actual=$(curl -sf "$url" | jq -r "$filter" 2>/dev/null)
    if echo "$actual" | grep -q "$substring"; then
        echo "PASS: $url | $filter contains '$substring'"
    else
        echo "FAIL: $url | $filter does not contain '$substring'"
        echo "  Got: $actual"
        return 1
    fi
}

# post_json <url> <json_body>
# Posts JSON and returns the response
post_json() {
    local url=$1
    local body=$2
    curl -sf -X POST -H 'Content-Type: application/json' -d "$body" "$url"
}

# cleanup trap: kills daemon PIDs stored in DAEMON_PIDS array, removes test-data
cleanup() {
    echo "Cleaning up..."
    for pid in "${DAEMON_PIDS[@]:-}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    # Remove test data
    rm -rf "${TEST_DATA_DIR:-./test-data}"
    echo "Cleanup done"
}

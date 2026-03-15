#!/usr/bin/env bash
# Common helpers for Howm integration tests

# wait_for_port <port> <timeout_seconds>
# Polls until port accepts connections or timeout
wait_for_port() {
    local port=$1
    local timeout=${2:-30}
    local elapsed=0
    echo "Waiting for port $port..."
    while ! curl -sf "http://localhost:$port/node/info" &>/dev/null; do
        sleep 1
        elapsed=$((elapsed + 1))
        if [ "$elapsed" -ge "$timeout" ]; then
            echo "ERROR: Port $port did not respond within ${timeout}s"
            return 1
        fi
    done
    echo "Port $port is ready"
}

# read_token <data_dir>
# Reads the API bearer token from the data directory
read_token() {
    local data_dir=$1
    cat "$data_dir/api_token" 2>/dev/null || echo ""
}

# assert_json <url> <jq_filter> <expected> [token]
# Curls the URL and checks that jq_filter output equals expected
assert_json() {
    local url=$1
    local filter=$2
    local expected=$3
    local token=${4:-}
    local auth_header=""
    [[ -n "$token" ]] && auth_header="-H \"Authorization: Bearer $token\""
    local actual
    actual=$(eval curl -sf $auth_header "\"$url\"" | jq -r "$filter" 2>/dev/null)
    if [ "$actual" = "$expected" ]; then
        echo "PASS: $url | $filter == $expected"
    else
        echo "FAIL: $url | $filter"
        echo "  Expected: $expected"
        echo "  Actual:   $actual"
        return 1
    fi
}

# assert_json_contains <url> <jq_filter> <substring> [token]
assert_json_contains() {
    local url=$1
    local filter=$2
    local substring=$3
    local token=${4:-}
    local auth_header=""
    [[ -n "$token" ]] && auth_header="-H \"Authorization: Bearer $token\""
    local actual
    actual=$(eval curl -sf $auth_header "\"$url\"" | jq -r "$filter" 2>/dev/null)
    if echo "$actual" | grep -q "$substring"; then
        echo "PASS: $url | $filter contains '$substring'"
    else
        echo "FAIL: $url | $filter does not contain '$substring'"
        echo "  Got: $actual"
        return 1
    fi
}

# post_json <url> <json_body> [token]
# Posts JSON and returns the response. If token is provided, adds auth.
post_json() {
    local url=$1
    local body=$2
    local token=${3:-}
    local auth_args=()
    [[ -n "$token" ]] && auth_args=(-H "Authorization: Bearer $token")
    curl -sf -X POST -H 'Content-Type: application/json' "${auth_args[@]}" -d "$body" "$url"
}

# cleanup trap: kills daemon PIDs stored in DAEMON_PIDS array, removes test-data
cleanup() {
    echo "Cleaning up..."
    for pid in "${DAEMON_PIDS[@]:-}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    # Stop any leftover howm containers
    docker ps --filter "name=howm-" --format "{{.ID}}" 2>/dev/null \
        | xargs -r docker stop &>/dev/null || true
    # Remove test data
    rm -rf "${TEST_DATA_DIR:-./test-data}"
    echo "Cleanup done"
}

#!/usr/bin/env bash
set -euo pipefail

PORT=6399
SERVER_PID=""
PASSED=0
FAILED=0

cleanup() {
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

fail() {
    echo "FAIL: $1"
    FAILED=$((FAILED + 1))
}

pass() {
    echo "PASS: $1"
    PASSED=$((PASSED + 1))
}

# Check prerequisites.
for cmd in valkey-server valkey-cli; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "Error: $cmd not found on PATH."
        echo "Install Valkey: see https://valkey.io/download/"
        exit 1
    fi
done

# Build the module.
echo "Building valkey-wasm..."
cargo build --release

# Detect shared library extension.
case "$(uname -s)" in
    Darwin) EXT="dylib" ;;
    *)      EXT="so" ;;
esac

LIB_PATH="./target/release/libvalkey_wasm.$EXT"
if [ ! -f "$LIB_PATH" ]; then
    echo "Error: built library not found at $LIB_PATH"
    exit 1
fi

# Start valkey-server with the module loaded.
echo "Starting valkey-server on port $PORT..."
valkey-server --port "$PORT" --loadmodule "$LIB_PATH" --loglevel warning &
SERVER_PID=$!

# Wait for server to be ready.
echo "Waiting for server..."
for i in $(seq 1 50); do
    if valkey-cli -p "$PORT" PING 2>/dev/null | grep -q PONG; then
        break
    fi
    if [ "$i" -eq 50 ]; then
        echo "Error: valkey-server did not become ready in time."
        exit 1
    fi
    sleep 0.1
done

# Test: MODULE LIST contains module named "wasm".
# valkey-cli outputs each field on its own line, so we look for "name"
# followed by "wasm" on the next line.
echo ""
echo "--- Running tests ---"
MODULE_LIST=$(valkey-cli -p "$PORT" MODULE LIST)
if echo "$MODULE_LIST" | grep -qxF "wasm"; then
    pass "MODULE LIST shows wasm"
else
    fail "MODULE LIST does not show wasm (got: $MODULE_LIST)"
fi

# Test: wasm.ping returns PONG.
PING_RESULT=$(valkey-cli -p "$PORT" wasm.ping)
if [ "$PING_RESULT" = "PONG" ]; then
    pass "wasm.ping returns PONG"
else
    fail "wasm.ping returned '$PING_RESULT', expected 'PONG'"
fi

# Summary.
echo ""
echo "--- Results: $PASSED passed, $FAILED failed ---"
if [ "$FAILED" -gt 0 ]; then
    exit 1
fi

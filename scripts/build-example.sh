#!/usr/bin/env bash
set -euo pipefail

# Check prerequisites
if ! command -v wasm-tools &>/dev/null; then
    echo "ERROR: wasm-tools not found. Install with: cargo install wasm-tools" >&2
    exit 1
fi

if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
    echo "ERROR: wasm32-unknown-unknown target not installed. Run: rustup target add wasm32-unknown-unknown" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXAMPLE_DIR="$REPO_ROOT/examples/get-test"

echo "==> Building example-get-test..."
cargo build \
    --target wasm32-unknown-unknown \
    --release \
    --manifest-path "$EXAMPLE_DIR/Cargo.toml"

CORE_WASM="$EXAMPLE_DIR/target/wasm32-unknown-unknown/release/example_get_test.wasm"
COMPONENT_WASM="$EXAMPLE_DIR/get-test.component.wasm"

echo "==> Componentizing..."
wasm-tools component new "$CORE_WASM" -o "$COMPONENT_WASM"

echo "==> Validating component WIT..."
wasm-tools component wit "$COMPONENT_WASM"

SIZE=$(wc -c < "$COMPONENT_WASM" | tr -d ' ')
echo ""
echo "Success! Component: $COMPONENT_WASM ($SIZE bytes)"

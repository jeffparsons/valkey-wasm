#!/usr/bin/env bash
set -euo pipefail

# Rebuild test fixture Wasm components from examples and WAT files.
# Run after changing WIT or guest code.

if ! command -v wasm-tools &>/dev/null; then
    echo "ERROR: wasm-tools not found. Install with: cargo install wasm-tools" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

for example in echo-args get-test; do
    dir="$REPO_ROOT/examples/$example"
    # Cargo crate name: hyphens become underscores.
    crate_name=$(grep '^name' "$dir/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/' | tr '-' '_')

    echo "Building $example..."
    cargo build \
        --target wasm32-unknown-unknown \
        --release \
        --manifest-path "$dir/Cargo.toml"

    wasm-tools component new \
        "$dir/target/wasm32-unknown-unknown/release/${crate_name}.wasm" \
        -o "$REPO_ROOT/src/testdata/$example.component.wasm"

    echo "  → src/testdata/$example.component.wasm"
done

# WAT-based fixtures.
for wat in "$REPO_ROOT"/src/testdata/*.wat; do
    out="${wat%.wat}.component.wasm"
    echo "Parsing $(basename "$wat")..."
    wasm-tools parse "$wat" -o "$out"
done

echo "Done."

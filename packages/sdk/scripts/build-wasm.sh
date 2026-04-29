#!/usr/bin/env bash
#
# build-wasm.sh — Build mnemonic-core to WebAssembly for the SDK.
#
# Output: core/pkg-web/  (target = web, chosen in Task 1 — works on Node 20+,
# Bun, Deno without a bundler).
#
# Prerequisites:
#   - wasm-pack ≥ 0.14 must be on PATH (cargo install wasm-pack).
#   - The core crate's `wasm` cargo feature must be enabled (it is, see
#     core/Cargo.toml).
#
# wasm-pack quirk: cargo ≥ 1.92 renamed `--out-dir` to `--artifact-dir`, so
# wasm-pack's own `--out-dir` flag breaks for any value other than the default
# (`pkg`). We build into the default `pkg`, then `mv` it to the
# target-segregated name `pkg-web` so the artifact path is stable for SDK
# consumers and won't collide with webapp's `core/pkg/` build.
#
# Usage:
#   bash packages/sdk/scripts/build-wasm.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "error: wasm-pack not found in PATH" >&2
  echo "  install with: cargo install wasm-pack" >&2
  exit 1
fi

cd "$REPO_ROOT"

# Wipe any stale default-output directory before building.
rm -rf core/pkg core/pkg-web

wasm-pack build core --target web --features wasm

# Re-anchor the output to a target-suffixed dir so multiple targets can coexist
# and the path is unambiguous from the SDK side.
mv core/pkg core/pkg-web
echo "✓ SDK wasm artifact at $REPO_ROOT/core/pkg-web/"

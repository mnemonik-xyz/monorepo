#!/usr/bin/env bash
#
# build-wasm.sh — Build the mnemonic-core Rust crate to WebAssembly for the webapp.
#
# Prerequisites:
#   - wasm-pack must be installed (one-time, NOT an npm dependency):
#       cargo install wasm-pack
#   - The sibling crate core/Cargo.toml must declare a `wasm` feature
#     (added by Task 2 of mnemonic-integrations). Without it, the build below
#     fails with "feature `wasm` not found".
#
# Output:
#   webapp/src/wasm/ contains the wasm-pack `--target web` ES module:
#     - mnemonic_core.js          (JS shim, default export = init())
#     - mnemonic_core.d.ts        (TypeScript types)
#     - mnemonic_core_bg.wasm     (the compiled WebAssembly)
#     - mnemonic_core_bg.wasm.d.ts
#     - package.json              (wasm-pack metadata, ignored by Vite)
#   This directory is git-ignored — it is regenerated on every `npm run build`.
#
# Usage:
#   bash webapp/scripts/build-wasm.sh
#   # or, from inside webapp/:
#   npm run build:wasm
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "error: wasm-pack not found in PATH" >&2
  echo "  install with: cargo install wasm-pack" >&2
  exit 1
fi

cd "$REPO_ROOT"
# wasm-pack interprets `--out-dir` relative to the crate manifest (core/), so
# we pass an absolute path to keep the output anchored at webapp/src/wasm/
# regardless of the build cwd.
wasm-pack build core \
  --target web \
  --out-dir "$REPO_ROOT/webapp/src/wasm" \
  --release \
  --features wasm

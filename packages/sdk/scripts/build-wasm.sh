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
rm -rf core/pkg core/pkg-web core/pkg-nodejs

wasm-pack build core --target web --features wasm

# Re-anchor the output to a target-suffixed dir so multiple targets can coexist
# and the path is unambiguous from the SDK side.
mv core/pkg core/pkg-web
echo "✓ SDK wasm artifact at $REPO_ROOT/core/pkg-web/"

# Also produce the `--target nodejs` build for the SDK's golden-fixture
# test (`packages/sdk/test/cose.golden.test.ts`) which loads the WASM via
# Node's CJS-friendly `import()` path. This artifact is NOT shipped in the
# npm tarball — production consumers use the `--target web` build above.
wasm-pack build core --target nodejs --features wasm
mv core/pkg core/pkg-nodejs
echo "✓ test wasm artifact at $REPO_ROOT/core/pkg-nodejs/"

# Post-build size optimization. wasm-pack 0.14 already runs `wasm-opt -O`,
# but a follow-up `-Oz --strip-debug --strip-producers` shaves a further
# ~3.5 KB and removes producer / debug metadata. Tried -O4 / -O3 / -Os / -Oz
# on 2026-04-30 — `-Oz` was the only level that produced a smaller artifact
# (see work/mnemonic-cli/decisions.md, "wasm-opt size reduction"). Optional
# tool: if absent we ship the wasm-pack default.
if command -v wasm-opt >/dev/null 2>&1; then
  WASM_TMP="$(mktemp -t mnemonic_core_opt.XXXXXX.wasm)"
  wasm-opt -Oz --strip-debug --strip-producers \
    "$REPO_ROOT/core/pkg-web/mnemonic_core_bg.wasm" \
    -o "$WASM_TMP"
  mv "$WASM_TMP" "$REPO_ROOT/core/pkg-web/mnemonic_core_bg.wasm"
  echo "✓ wasm-opt: applied -Oz + --strip-debug + --strip-producers"
else
  echo "  wasm-opt not installed; shipping wasm-pack default. Install via:"
  echo "    macOS:  brew install binaryen"
  echo "    Linux:  apt-get install binaryen   (or build from source)"
fi

# Mirror ONLY the `--target web` artifact into the SDK's published
# `dist/wasm/` so it ships inside the npm tarball. SDK 0.1.2 dropped the
# `--target nodejs` mirror — that artifact crashes at module-eval on Node
# 20+ macOS (WebAssembly.Table.grow under --reference-types). The SDK now
# loads the `--target web` artifact uniformly and, on Node/Bun/Deno, reads
# the .wasm from disk via `node:fs/promises.readFile` and passes the bytes
# directly to `default(bytes)` — bypassing the broken `fetch(file://)` path.
# See packages/sdk/src/wasm.ts and decisions.md "SDK 0.1.2 — fs-shim WASM
# loader".
#
# Note: the `--target nodejs` build above is still produced because
# `packages/sdk/test/cose.golden.test.ts` imports it directly from
# `core/pkg-nodejs/`. That artifact is NOT shipped in the npm tarball.
SDK_DIST_WASM="$REPO_ROOT/packages/sdk/dist/wasm"
rm -rf "$SDK_DIST_WASM"
mkdir -p "$SDK_DIST_WASM"
cp \
  "$REPO_ROOT/core/pkg-web/mnemonic_core.js" \
  "$REPO_ROOT/core/pkg-web/mnemonic_core_bg.wasm" \
  "$REPO_ROOT/core/pkg-web/mnemonic_core.d.ts" \
  "$REPO_ROOT/core/pkg-web/mnemonic_core_bg.wasm.d.ts" \
  "$SDK_DIST_WASM/"
echo "✓ SDK wasm artifact mirrored to $SDK_DIST_WASM/"

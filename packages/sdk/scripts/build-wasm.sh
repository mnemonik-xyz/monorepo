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

# Mirror BOTH wasm-pack outputs into the SDK's published `dist/wasm/` so the
# artifacts ship inside the npm tarball. The SDK runtime-detects host env
# (Node/Bun/Deno vs. browser) and picks the matching subdir — see
# packages/sdk/src/wasm.ts.
#
# Why both:
#   - `--target web` uses `fetch(import.meta.url)` to load the .wasm. Browsers
#     are happy. Node 20+/22 undici fetch() does NOT support `file://` URLs,
#     and Bun/Deno hit a related WebAssembly.Table.grow() crash. T15 P0.
#   - `--target nodejs` emits CJS-shaped JS that loads the .wasm via
#     `fs.readFileSync` at module-eval — works under Node/Bun/Deno without
#     any fetch shim.
SDK_DIST_WASM="$REPO_ROOT/packages/sdk/dist/wasm"
rm -rf "$SDK_DIST_WASM"
mkdir -p "$SDK_DIST_WASM/web" "$SDK_DIST_WASM/nodejs"

cp \
  "$REPO_ROOT/core/pkg-web/mnemonic_core.js" \
  "$REPO_ROOT/core/pkg-web/mnemonic_core_bg.wasm" \
  "$REPO_ROOT/core/pkg-web/mnemonic_core.d.ts" \
  "$REPO_ROOT/core/pkg-web/mnemonic_core_bg.wasm.d.ts" \
  "$SDK_DIST_WASM/web/"
echo "✓ SDK wasm artifact (web) mirrored to $SDK_DIST_WASM/web/"

cp \
  "$REPO_ROOT/core/pkg-nodejs/mnemonic_core.js" \
  "$REPO_ROOT/core/pkg-nodejs/mnemonic_core_bg.wasm" \
  "$REPO_ROOT/core/pkg-nodejs/mnemonic_core.d.ts" \
  "$REPO_ROOT/core/pkg-nodejs/mnemonic_core_bg.wasm.d.ts" \
  "$SDK_DIST_WASM/nodejs/"
# wasm-pack --target nodejs also emits a `mnemonic_core_bg.js` CJS shim
# alongside `mnemonic_core.js`. Copy it too — `mnemonic_core.js` requires it.
if [ -f "$REPO_ROOT/core/pkg-nodejs/mnemonic_core_bg.js" ]; then
  cp "$REPO_ROOT/core/pkg-nodejs/mnemonic_core_bg.js" "$SDK_DIST_WASM/nodejs/"
fi
# CRITICAL: the SDK's own `package.json` has `"type": "module"`, which makes
# Node interpret every `.js` under it as ESM. The `--target nodejs` artifact
# is CommonJS (uses `require()` and `module.exports`). Drop a scoped
# `package.json` that flips the directory to CJS so Node loads the artifact
# correctly when SDK code dynamic-imports it.
cat > "$SDK_DIST_WASM/nodejs/package.json" <<'EOF'
{
  "type": "commonjs"
}
EOF
echo "✓ SDK wasm artifact (nodejs) mirrored to $SDK_DIST_WASM/nodejs/"

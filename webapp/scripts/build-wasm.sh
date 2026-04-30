#!/usr/bin/env bash
#
# build-wasm.sh — Build the mnemonic-core Rust crate to WebAssembly for both
# the webapp (`webapp/src/wasm/`) AND the SDK (`core/pkg-web/`).
#
# Both consumers use the same `--target web` artifact bytes; the webapp loads
# them from `src/wasm/` (Vite-bundled), the SDK loads them from `core/pkg-web/`
# (workspace-internal). Building both in one pass keeps them in lockstep —
# every webapp build automatically refreshes the SDK artifact too. Decision 3
# of work/mnemonic-cli/tech-spec.md picked `--target web` for the SDK; see
# packages/sdk/README.md "Runtime targets" for the smoke matrix.
#
# Prerequisites:
#   - wasm-pack ≥ 0.14 must be installed (NOT an npm dependency):
#       cargo install wasm-pack
#   - The sibling crate core/Cargo.toml must declare the `wasm` feature
#     (already added by mnemonic-integrations Task 2).
#
# wasm-pack quirk: cargo ≥ 1.92 renamed `--out-dir` to `--artifact-dir`, which
# breaks wasm-pack's `--out-dir <PATH>` for any path other than the default
# `pkg`. Workaround: build into the default `core/pkg/` first, then mirror to
# both consumer-facing destinations.
#
# Output layout:
#   webapp/src/wasm/        — git-ignored, regenerated on every `npm run build`
#     mnemonic_core.js          (JS shim, default export = init())
#     mnemonic_core.d.ts        (TypeScript types)
#     mnemonic_core_bg.wasm     (compiled WebAssembly)
#     mnemonic_core_bg.wasm.d.ts
#     package.json              (wasm-pack metadata, ignored by Vite)
#   core/pkg-web/           — git-ignored, consumed by @mnemonik-xyz/sdk
#     same files, identical bytes.
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

# Wipe stale outputs so the post-build mirror operations cannot pick up old
# files left over from a previous failed build.
rm -rf core/pkg core/pkg-web webapp/src/wasm

# Build into the default `core/pkg/` directory — the only `--out-dir` value
# wasm-pack reliably supports under cargo ≥ 1.92.
wasm-pack build core --target web --features wasm

# Mirror the artifact to both consumers. Using `cp -R` (not `mv`) so the
# webapp and SDK have independent copies — touching one doesn't perturb the
# other if a downstream tool decides to mutate files in place.
mkdir -p webapp/src
cp -R core/pkg webapp/src/wasm
mv core/pkg core/pkg-web

echo "✓ webapp wasm  -> $REPO_ROOT/webapp/src/wasm/"
echo "✓ sdk wasm     -> $REPO_ROOT/core/pkg-web/"

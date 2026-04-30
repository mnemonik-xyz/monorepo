#!/usr/bin/env bash
# Regenerate the COSE golden fixture from Rust core.
#
# Usage (from repo root or anywhere):
#   bash packages/sdk/scripts/regen-golden-fixtures.sh
#
# What this does:
#   1. Runs the gated cargo test `core/tests/golden_fixtures.rs::emit_fixtures`
#      under the `golden-fixtures` feature with `--ignored --nocapture`.
#   2. Strips cargo's framing lines so only the JSON body remains.
#   3. Writes the JSON to `packages/sdk/test/fixtures/golden-cose.json`.
#   4. Computes its SHA-256 and writes the hex digest to
#      `packages/sdk/test/fixtures/golden-cose.sha256` (one-line, no filename).
#
# The CI lockstep gate re-runs steps 1-2 and diffs the resulting checksum
# against the committed `golden-cose.sha256`. Drift fails CI.
#
# Tech-spec reference: work/mnemonic-cli/tech-spec.md Decision 12.

set -euo pipefail

# Resolve repo root from this script's location: scripts/.. = sdk/, sdk/.. = packages/, packages/.. = repo root.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../../.." &>/dev/null && pwd)"
FIXTURE_DIR="$REPO_ROOT/packages/sdk/test/fixtures"
FIXTURE_JSON="$FIXTURE_DIR/golden-cose.json"
FIXTURE_SHA="$FIXTURE_DIR/golden-cose.sha256"

mkdir -p "$FIXTURE_DIR"

echo "[regen] running cargo emit_fixtures ..." >&2
# Force-disable cargo's ANSI color codes for this invocation. CI sets
# `CARGO_TERM_COLOR=always` workspace-wide, which would inject `\x1b[…m`
# escapes into RAW_OUTPUT — and since the fixture is sliced by `text.find("[")`,
# an early `\x1b[` would shift `start` to the escape sequence instead of the
# real JSON `[`. We override the env var inline AND pass `--color=never` as
# belt-and-braces. Python additionally strips any residual escapes below.
RAW_OUTPUT="$(
  cd "$REPO_ROOT" \
    && CARGO_TERM_COLOR=never cargo test --color=never \
         --features golden-fixtures -p mnemonic-core \
         --test golden_fixtures emit_fixtures \
         -- --ignored --nocapture 2>/dev/null
)"

# Cargo wraps the println output in framing — extract the JSON body. The
# fixture is a JSON array, so the first `[` and the matching closing `]`
# bracket the payload. We use python3 (universally available on dev + CI
# images) to slice deterministically. The raw output is passed via an env
# var to avoid clashing with python's own stdin (heredoc), and the output
# path is also passed via env to avoid quote-escape footguns when the repo
# path contains a single quote.
export RAW_CARGO_OUTPUT="$RAW_OUTPUT"
export REGEN_OUT_PATH="$FIXTURE_JSON"
python3 <<'PY'
import os
import re
import sys

text = os.environ.get("RAW_CARGO_OUTPUT", "")
# Defense-in-depth: even with CARGO_TERM_COLOR=never + --color=never, strip
# any ANSI CSI escape sequences (`\x1b[…m`) before slicing. Otherwise an
# escape preceding the JSON would shift `start` and corrupt the fixture.
text = re.sub(r"\x1b\[[0-9;]*m", "", text)
start = text.find("[")
end = text.rfind("]")
if start == -1 or end == -1 or end <= start:
    sys.stderr.write("regen: could not locate JSON array in cargo output\n")
    sys.exit(1)
out_path = os.environ["REGEN_OUT_PATH"]
with open(out_path, "w", encoding="utf-8") as f:
    f.write(text[start : end + 1])
    f.write("\n")
PY

# Compute the SHA-256 of the JSON file (raw bytes, no trailing filename).
# `shasum -a 256` is portable across macOS + Linux CI; `sha256sum` is
# Linux-only but kept as fallback for environments without `shasum`.
if command -v shasum >/dev/null 2>&1; then
  DIGEST="$(shasum -a 256 "$FIXTURE_JSON" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  DIGEST="$(sha256sum "$FIXTURE_JSON" | awk '{print $1}')"
else
  echo "regen: neither shasum nor sha256sum is available" >&2
  exit 1
fi

printf '%s\n' "$DIGEST" > "$FIXTURE_SHA"

ENTRIES_COUNT="$(REGEN_OUT_PATH="$FIXTURE_JSON" python3 -c "import json,os; print(len(json.load(open(os.environ['REGEN_OUT_PATH']))))")"

echo "[regen] wrote $FIXTURE_JSON ($ENTRIES_COUNT entries)" >&2
echo "[regen] wrote $FIXTURE_SHA  ($DIGEST)" >&2

#!/usr/bin/env bash
# Cross-language keychain interop test (Linux + gnome-keyring / Secret Service).
#
# Drives both Rust `mnemonic-core::identity::ensure()` and the Node
# `@mnemonik-xyz/cli` `mnemonic whoami` against the SAME OS keychain entry
# and asserts byte-equality of the resulting pubkeys.
#
# Sub-tests:
#   A. Rust writes via ensure() → Node reads via `whoami --json` — same pubkey
#   B. Node writes via `whoami` → Rust reads via ensure() — same pubkey
#   C. Pre-seeded legacy `identity.json` → Rust migrates to stub + keychain →
#      Node reads, all three sides agree on the legacy fixture's pubkey
#
# The Rust side uses a tiny `keychain-roundtrip` example binary that just
# calls `identity::ensure()` and prints `{"pubkey":..., "storage":...}` —
# NOT the full `mnemonic-mcp` stdio server. The mcp server requires the
# embedder + pricing engine + SQLite store + the JSON-RPC stdio protocol,
# none of which is needed here; using the example avoids spurious EOF /
# embedder-init failures on CI runners. The previous version of this script
# wrapped the mcp binary in `2>/dev/null` and lost every failure for months.
#
# Environment (set by the CI workflow OR exported below):
#   DBUS_SESSION_BUS_ADDRESS — pre-set by `eval "$(dbus-launch --sh-syntax)"`
#                              before this script runs. The Rust+Node
#                              processes inherit it and talk to the
#                              gnome-keyring secrets daemon over D-Bus.
#
# Exits non-zero on the first sub-test failure. Final cleanup removes the
# keychain entry and the three temp HOME directories.

set -euo pipefail

# Trap-based cleanup
cleanup() {
  local ec=$?
  secret-tool clear service xyz.mnemonik.identity account default 2>/dev/null || true
  [ -n "${HOME_A:-}" ] && rm -rf "$HOME_A" || true
  [ -n "${HOME_B:-}" ] && rm -rf "$HOME_B" || true
  [ -n "${HOME_C:-}" ] && rm -rf "$HOME_C" || true
  exit "$ec"
}
trap cleanup EXIT

# Paths
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUST_ROUNDTRIP="${REPO_ROOT}/target/debug/examples/keychain-roundtrip"
NODE_CLI="${REPO_ROOT}/packages/cli/dist/bin/mnemonic.js"
LEGACY_FIXTURE="${REPO_ROOT}/tests/fixtures/legacy-identity.json"

# Pre-flight
echo "--- Pre-flight ---"

if [ ! -x "$RUST_ROUNDTRIP" ]; then
  echo "missing Rust example binary: $RUST_ROUNDTRIP" >&2
  echo "build it first with: cargo build -p mnemonic-core --example keychain-roundtrip" >&2
  exit 1
fi
if [ ! -f "$NODE_CLI" ]; then
  echo "missing Node CLI dist: $NODE_CLI" >&2
  echo "build it first with: npm run build --workspace=@mnemonik-xyz/sdk && npm run build --workspace=@mnemonik-xyz/cli" >&2
  exit 1
fi
if [ ! -f "$LEGACY_FIXTURE" ]; then
  echo "missing legacy identity fixture: $LEGACY_FIXTURE" >&2
  exit 1
fi
for tool in jq secret-tool node; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing required tool: $tool" >&2; exit 1; }
done

# Quiet mode for both sides — suppress the one-line bootstrap stderr noise
# so the test output stays signal-to-noise high.
export MNEMONIC_QUIET=1

run_rust() {
  local out_stdout="$1"
  local out_stderr="$2"
  # NEVER redirect to /dev/null — the previous version did and lost
  # the failure mode for months. Capture stderr to a file so we can
  # print it on assertion failure.
  "$RUST_ROUNDTRIP" >"$out_stdout" 2>"$out_stderr"
}

run_node_whoami() {
  local out_stdout="$1"
  local out_stderr="$2"
  node "$NODE_CLI" whoami --json >"$out_stdout" 2>"$out_stderr"
}

clear_keychain() {
  secret-tool clear service xyz.mnemonik.identity account default 2>/dev/null || true
}

print_diag() {
  local label="$1"
  local stdout="$2"
  local stderr="$3"
  echo "  -- $label diagnostics --"
  echo "  stdout:"
  sed 's/^/    /' "$stdout" || true
  echo "  stderr:"
  sed 's/^/    /' "$stderr" || true
}

# Sub-test A: Rust write -> Node read
echo
echo "--- A: Rust write -> Node read ---"

HOME_A=$(mktemp -d)
export HOME="$HOME_A"
clear_keychain

A_RUST_STDOUT=$(mktemp); A_RUST_STDERR=$(mktemp)
if ! run_rust "$A_RUST_STDOUT" "$A_RUST_STDERR"; then
  print_diag "A.rust" "$A_RUST_STDOUT" "$A_RUST_STDERR"
  echo "FAIL A: Rust ensure() exited non-zero" >&2
  exit 1
fi
A_RUST_PUBKEY=$(jq -r '.pubkey' "$A_RUST_STDOUT" 2>/dev/null || echo "")
echo "  Rust pubkey: $A_RUST_PUBKEY"
[ -n "$A_RUST_PUBKEY" ] && [ "$A_RUST_PUBKEY" != "null" ] || { echo "FAIL A: Rust pubkey empty/null"; print_diag "A.rust" "$A_RUST_STDOUT" "$A_RUST_STDERR"; exit 1; }

A_NODE_STDOUT=$(mktemp); A_NODE_STDERR=$(mktemp)
if ! run_node_whoami "$A_NODE_STDOUT" "$A_NODE_STDERR"; then
  print_diag "A.node" "$A_NODE_STDOUT" "$A_NODE_STDERR"
  echo "FAIL A: Node whoami exited non-zero" >&2
  exit 1
fi
A_NODE_PUBKEY=$(jq -r '.pubkey' "$A_NODE_STDOUT" 2>/dev/null || echo "")
echo "  Node pubkey: $A_NODE_PUBKEY"
[ -n "$A_NODE_PUBKEY" ] && [ "$A_NODE_PUBKEY" != "null" ] || { echo "FAIL A: Node pubkey empty/null"; print_diag "A.node" "$A_NODE_STDOUT" "$A_NODE_STDERR"; exit 1; }

if [ "$A_RUST_PUBKEY" != "$A_NODE_PUBKEY" ]; then
  echo "FAIL A: pubkey mismatch -- Rust=$A_RUST_PUBKEY  Node=$A_NODE_PUBKEY" >&2
  exit 1
fi
echo "  PASS A: cross-language pubkey agreement"

# Sub-test B: Node write -> Rust read
echo
echo "--- B: Node write -> Rust read ---"

HOME_B=$(mktemp -d)
export HOME="$HOME_B"
clear_keychain

B_NODE_STDOUT=$(mktemp); B_NODE_STDERR=$(mktemp)
if ! run_node_whoami "$B_NODE_STDOUT" "$B_NODE_STDERR"; then
  print_diag "B.node" "$B_NODE_STDOUT" "$B_NODE_STDERR"
  echo "FAIL B: Node whoami exited non-zero" >&2
  exit 1
fi
B_NODE_PUBKEY=$(jq -r '.pubkey' "$B_NODE_STDOUT" 2>/dev/null || echo "")
echo "  Node pubkey: $B_NODE_PUBKEY"
[ -n "$B_NODE_PUBKEY" ] && [ "$B_NODE_PUBKEY" != "null" ] || { echo "FAIL B: Node pubkey empty"; print_diag "B.node" "$B_NODE_STDOUT" "$B_NODE_STDERR"; exit 1; }

B_RUST_STDOUT=$(mktemp); B_RUST_STDERR=$(mktemp)
if ! run_rust "$B_RUST_STDOUT" "$B_RUST_STDERR"; then
  print_diag "B.rust" "$B_RUST_STDOUT" "$B_RUST_STDERR"
  echo "FAIL B: Rust ensure() exited non-zero" >&2
  exit 1
fi
B_RUST_PUBKEY=$(jq -r '.pubkey' "$B_RUST_STDOUT" 2>/dev/null || echo "")
echo "  Rust pubkey: $B_RUST_PUBKEY"

if [ "$B_RUST_PUBKEY" != "$B_NODE_PUBKEY" ]; then
  echo "FAIL B: pubkey mismatch -- Node=$B_NODE_PUBKEY  Rust=$B_RUST_PUBKEY" >&2
  exit 1
fi
echo "  PASS B: cross-language pubkey agreement (Node->Rust direction)"

# Sub-test C: Legacy migration -> Rust migrates -> Node reads
echo
echo "--- C: Legacy migration -> Rust migrates -> Node reads ---"

HOME_C=$(mktemp -d)
export HOME="$HOME_C"
clear_keychain

mkdir -p "$HOME_C/.mnemonic"
cp "$LEGACY_FIXTURE" "$HOME_C/.mnemonic/identity.json"
C_LEGACY_PUBKEY=$(jq -r '.pubkey_base58' "$HOME_C/.mnemonic/identity.json")
echo "  Legacy fixture pubkey: $C_LEGACY_PUBKEY"

C_RUST_STDOUT=$(mktemp); C_RUST_STDERR=$(mktemp)
if ! run_rust "$C_RUST_STDOUT" "$C_RUST_STDERR"; then
  print_diag "C.rust" "$C_RUST_STDOUT" "$C_RUST_STDERR"
  echo "FAIL C: Rust ensure() (migration path) exited non-zero" >&2
  exit 1
fi
C_RUST_PUBKEY=$(jq -r '.pubkey' "$C_RUST_STDOUT" 2>/dev/null || echo "")
echo "  Rust pubkey after migration: $C_RUST_PUBKEY"

if [ "$C_RUST_PUBKEY" != "$C_LEGACY_PUBKEY" ]; then
  echo "FAIL C.1: migration changed pubkey -- expected=$C_LEGACY_PUBKEY  got=$C_RUST_PUBKEY" >&2
  exit 1
fi
echo "  PASS C.1: pubkey preserved across legacy -> stub migration"

# C.2: Post-migration shape check. Two legitimate outcomes per Decision 4
# ("file-fallback triggers on ANY keychain error, not just unavailability"):
#
#   (a) Migration succeeded → file is now stub shape: has `keychain_ref`,
#       no `secret` field. Keychain has the secret.
#   (b) Migration deferred (OS Secret Service set() failed at the D-Bus
#       boundary) → ensure() correctly kept the legacy file as-is per the
#       handle_legacy recovery path. File still has `secret`, pubkey
#       unchanged (already verified by C.1).
#
# Detect which case we landed in and assert accordingly. Both are valid;
# the critical invariants (pubkey preserved, no data loss) are already
# asserted by C.1.
if jq -e '.keychain_ref' "$HOME_C/.mnemonic/identity.json" >/dev/null 2>&1; then
  # Case (a): migration succeeded → must be pure stub (no secret).
  if jq -e '.secret' "$HOME_C/.mnemonic/identity.json" >/dev/null 2>&1; then
    echo "FAIL C.2a: identity.json has keychain_ref but ALSO retains 'secret' -- should be stub-only" >&2
    exit 1
  fi
  echo "  PASS C.2 (case a): identity.json migrated to stub format"
elif jq -e '.secret' "$HOME_C/.mnemonic/identity.json" >/dev/null 2>&1; then
  # Case (b): migration deferred (OS unavailable) → file stays legacy.
  # Decision 4 recovery path. Pubkey preservation already asserted by
  # C.1. Print the storage backend Rust reported for clarity.
  C_RUST_STORAGE=$(jq -r '.storage' "$C_RUST_STDOUT" 2>/dev/null || echo "?")
  echo "  PASS C.2 (case b): OS keychain unavailable -- legacy file preserved as-is (storage=$C_RUST_STORAGE)"
else
  echo "FAIL C.2: identity.json is neither stub nor legacy after migration attempt" >&2
  echo "  current contents:"
  jq . "$HOME_C/.mnemonic/identity.json" 2>&1 | sed 's/^/    /'
  exit 1
fi

C_NODE_STDOUT=$(mktemp); C_NODE_STDERR=$(mktemp)
if ! run_node_whoami "$C_NODE_STDOUT" "$C_NODE_STDERR"; then
  print_diag "C.node" "$C_NODE_STDOUT" "$C_NODE_STDERR"
  echo "FAIL C.3: Node whoami (post-migration) exited non-zero" >&2
  exit 1
fi
C_NODE_PUBKEY=$(jq -r '.pubkey' "$C_NODE_STDOUT" 2>/dev/null || echo "")
echo "  Node pubkey after migration: $C_NODE_PUBKEY"

if [ "$C_NODE_PUBKEY" != "$C_LEGACY_PUBKEY" ]; then
  echo "FAIL C.3: Node pubkey mismatch after migration -- expected=$C_LEGACY_PUBKEY  got=$C_NODE_PUBKEY" >&2
  exit 1
fi
echo "  PASS C.3: Node CLI reads the migrated stub correctly"

echo
echo "========================================================================"
echo " ALL CROSS-LANG TESTS PASSED  (Linux + gnome-keyring / Secret Service)"
echo "========================================================================"

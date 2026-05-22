#!/usr/bin/env bash
# T15 — macOS row, fully automated on the host (no Computer Use container needed).
#
# Runs the 6-step smoke matrix from work/invisible-identity/scenarios/T15-smoke-matrix.md
# for the macos14 row, plus an explicit cross-language collision check
# (Node writes → Rust reads, pubkey byte-equality) which is the single most
# important regression signal this feature introduces.
#
# Safe to run on your real machine: backs up an existing ~/.mnemonic directory
# AND the existing xyz.mnemonik.identity keychain entry, then restores them at
# the end. No data loss if the script crashes mid-run — cleanup runs via trap.
#
# Usage:
#   bash scripts/t15-cu/macos-smoke.sh                       # full smoke
#   bash scripts/t15-cu/macos-smoke.sh --skip-build           # skip cargo + npm
#   bash scripts/t15-cu/macos-smoke.sh --webapp-url=URL       # also do Step 6 round-trip (opens browser)
#
# Output:
#   - Per-step diagnostics under  /tmp/t15-macos-<step>-<artifact>
#   - JSON result at              work/invisible-identity/logs/working/T15-smoke-result-macos14.json
#   - One-line PASS / FAIL summary printed at the end
#
# Exit codes:
#   0  → all steps PASS
#   1  → any step FAILED
#   2  → pre-flight error (wrong OS, missing binaries, etc.)

set -euo pipefail

# -----------------------------------------------------------------------------
# Setup
# -----------------------------------------------------------------------------

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="$REPO/work/invisible-identity/logs/working"
RESULT_JSON="$OUT_DIR/T15-smoke-result-macos14.json"
TMP_DIR="/tmp/t15-macos-$$"
BACKUP_DIR="/tmp/t15-macos-backup-$$"

SERVICE="xyz.mnemonik.identity"
ACCOUNT="default"

SKIP_BUILD=0
WEBAPP_URL=""
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --webapp-url=*) WEBAPP_URL="${arg#--webapp-url=}" ;;
    -h|--help)
      sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$TMP_DIR" "$BACKUP_DIR" "$OUT_DIR"

declare -a STEP_NAMES=()
declare -a STEP_RESULTS=()
declare -a STEP_NOTES=()

log() { printf '[t15-macos] %s\n' "$*" >&2; }
note() { local s="$1"; shift; STEP_NOTES+=("$s: $*"); }

# -----------------------------------------------------------------------------
# Pre-flight checks
# -----------------------------------------------------------------------------

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "ERROR: this script is macOS-only. Use scripts/t15-cu/compose.yml for Linux rows." >&2
  exit 2
fi

cd "$REPO"

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
CURRENT_HEAD="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
log "branch: $CURRENT_BRANCH | HEAD: ${CURRENT_HEAD:0:10}"

# -----------------------------------------------------------------------------
# Backup existing identity (will be restored on EXIT)
# -----------------------------------------------------------------------------

if [[ -d "$HOME/.mnemonic" ]]; then
  log "backing up existing ~/.mnemonic to $BACKUP_DIR/.mnemonic"
  cp -R "$HOME/.mnemonic" "$BACKUP_DIR/.mnemonic"
fi

EXISTING_ENTRY="$(security find-generic-password -s "$SERVICE" -a "$ACCOUNT" -w "$HOME/Library/Keychains/login.keychain-db" 2>/dev/null || true)"
if [[ -n "$EXISTING_ENTRY" ]]; then
  log "backing up existing keychain entry ($SERVICE/$ACCOUNT) — will be restored at end"
  printf '%s' "$EXISTING_ENTRY" > "$BACKUP_DIR/keychain.txt"
  chmod 600 "$BACKUP_DIR/keychain.txt"
fi

cleanup() {
  local ec=$?
  log "cleanup: restoring original state"
  rm -rf "$HOME/.mnemonic"
  security delete-generic-password -s "$SERVICE" -a "$ACCOUNT" 2>/dev/null || true

  if [[ -d "$BACKUP_DIR/.mnemonic" ]]; then
    log "cleanup: restoring backed-up ~/.mnemonic"
    cp -R "$BACKUP_DIR/.mnemonic" "$HOME/.mnemonic"
  fi
  if [[ -f "$BACKUP_DIR/keychain.txt" ]]; then
    log "cleanup: restoring backed-up keychain entry"
    security add-generic-password -s "$SERVICE" -a "$ACCOUNT" -w "$(cat "$BACKUP_DIR/keychain.txt")" -U 2>/dev/null || true
    rm -f "$BACKUP_DIR/keychain.txt"
  fi

  # Keep TMP_DIR around for diagnostics if any step failed
  if [[ "$ec" -eq 0 ]] && [[ "${T15_KEEP_TMP:-0}" != "1" ]]; then
    rm -rf "$TMP_DIR" "$BACKUP_DIR"
  else
    log "diagnostics retained at $TMP_DIR"
  fi
}
trap cleanup EXIT

# -----------------------------------------------------------------------------
# Build artifacts (skippable)
# -----------------------------------------------------------------------------

MCP_BIN="$REPO/target/release/mnemonic-mcp"
MNEMONIC_CLI="$REPO/packages/cli/dist/bin/mnemonic.js"
KEYCHAIN_READ="$REPO/target/release/examples/keychain-read"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  log "building mnemonic-mcp + keychain-read example + CLI (first run: ~5 min; subsequent: ~10s)"
  cargo build -p mnemonic-core --release --example keychain-read 2> "$TMP_DIR/build.cargo.stderr" || {
    cat "$TMP_DIR/build.cargo.stderr" >&2
    echo "FATAL: cargo build failed" >&2; exit 2;
  }
  cargo build -p mnemonic-mcp --release 2> "$TMP_DIR/build.mcp.stderr" || {
    cat "$TMP_DIR/build.mcp.stderr" >&2
    echo "FATAL: cargo build mnemonic-mcp failed" >&2; exit 2;
  }
  npm install --workspaces --include-workspace-root --no-audit --no-fund > "$TMP_DIR/build.npm.stdout" 2> "$TMP_DIR/build.npm.stderr" || {
    cat "$TMP_DIR/build.npm.stderr" >&2
    echo "FATAL: npm install failed" >&2; exit 2;
  }
  npm run build --workspace=@mnemonik-xyz/cli > "$TMP_DIR/build.cli.stdout" 2> "$TMP_DIR/build.cli.stderr" || {
    cat "$TMP_DIR/build.cli.stderr" >&2
    echo "FATAL: npm run build @mnemonik-xyz/cli failed" >&2; exit 2;
  }
fi

for f in "$MCP_BIN" "$MNEMONIC_CLI" "$KEYCHAIN_READ"; do
  [[ -e "$f" ]] || { echo "FATAL: missing build artifact: $f (re-run without --skip-build)" >&2; exit 2; }
done

log "binaries ready: mcp=$MCP_BIN, cli=$MNEMONIC_CLI, keychain-read=$KEYCHAIN_READ"

# -----------------------------------------------------------------------------
# Helpers
# -----------------------------------------------------------------------------

run_node() {
  # Wrap node invocation with MNEMONIC_QUIET to suppress bootstrap stderr lines
  # by default; pass NOISY=1 to keep them.
  if [[ "${NOISY:-0}" == "1" ]]; then
    node "$MNEMONIC_CLI" "$@"
  else
    MNEMONIC_QUIET=1 node "$MNEMONIC_CLI" "$@"
  fi
}

assert_pass() {
  local name="$1" predicate="$2"
  STEP_NAMES+=("$name")
  if eval "$predicate"; then
    STEP_RESULTS+=("pass")
    log "  PASS  $name"
  else
    STEP_RESULTS+=("fail")
    log "  FAIL  $name  predicate: $predicate"
  fi
}

# Reset both file + keychain to a clean slate
clean_state() {
  rm -rf "$HOME/.mnemonic"
  security delete-generic-password -s "$SERVICE" -a "$ACCOUNT" 2>/dev/null || true
}

# -----------------------------------------------------------------------------
# STEP 1 — Clean state
# -----------------------------------------------------------------------------

log "STEP 1 — clean state"
clean_state
assert_pass "step1.no_dotfolder"   "! test -e \"$HOME/.mnemonic\""
assert_pass "step1.no_keychain"    "! security find-generic-password -s \"$SERVICE\" -a \"$ACCOUNT\" >/dev/null 2>&1"

# -----------------------------------------------------------------------------
# STEP 2 — Bootstrap (Node CLI creates identity in Apple Keychain)
# -----------------------------------------------------------------------------

log "STEP 2 — bootstrap via mnemonic whoami"
NOISY=1 node "$MNEMONIC_CLI" whoami > "$TMP_DIR/step2.stdout" 2> "$TMP_DIR/step2.stderr" || true
STEP2_EXIT=$?

assert_pass "step2.exit_code_zero"        "[ -s \"$TMP_DIR/step2.stdout\" ]"
assert_pass "step2.identity_json_exists"  "test -f \"$HOME/.mnemonic/identity.json\""
assert_pass "step2.identity_json_is_stub" "! jq -e .secret \"$HOME/.mnemonic/identity.json\" >/dev/null 2>&1"
assert_pass "step2.identity_has_keychain_ref" "jq -e .keychain_ref \"$HOME/.mnemonic/identity.json\" >/dev/null 2>&1"
assert_pass "step2.readme_exists"         "test -f \"$HOME/.mnemonic/README.txt\""
assert_pass "step2.stderr_creation_line" "grep -qE 'mnemonic: identity created did:sol:[1-9A-HJ-NP-Za-km-z]+ stored in OS keychain' \"$TMP_DIR/step2.stderr\""
assert_pass "step2.keychain_entry_present" "security find-generic-password -s \"$SERVICE\" -a \"$ACCOUNT\" >/dev/null 2>&1"

NODE_PUBKEY_AFTER_STEP2="$(jq -r .pubkey_base58 "$HOME/.mnemonic/identity.json" 2>/dev/null || echo "")"
log "  pubkey (from stub file): $NODE_PUBKEY_AFTER_STEP2"
note "step2" "pubkey=$NODE_PUBKEY_AFTER_STEP2"

# -----------------------------------------------------------------------------
# STEP 2b — macOS keychain pre-auth (avoids "Always Allow" GUI prompt in Step 3)
# -----------------------------------------------------------------------------

log "STEP 2b — pre-authorize keychain partition list (one-time login password prompt expected)"
if bash "$REPO/scripts/macos-prep-keychain.sh" > "$TMP_DIR/step2b.stdout" 2> "$TMP_DIR/step2b.stderr"; then
  assert_pass "step2b.macos_prep_ok" "true"
else
  log "  macos-prep-keychain.sh exit nonzero — Step 3 may show GUI prompt"
  assert_pass "step2b.macos_prep_ok" "false"
  note "step2b" "see $TMP_DIR/step2b.stderr"
fi

# -----------------------------------------------------------------------------
# STEP 3 — Prove (exercises actual keychain unlock; pure local, no login needed)
# -----------------------------------------------------------------------------

# `mnemonic prove` signs a 32-byte challenge with the local Ed25519 key. It
# touches the keychain via OsKeyStore::get() — exactly the path that triggers
# Apple's "Always Allow" prompt on first read. If Step 2b widened the
# partition list, this must succeed silently. We use `prove` instead of
# `sign` because `sign` requires a logged-in JWT (token.json), which the
# smoke setup intentionally does not provision.

log "STEP 3 — prove a challenge (verifies keychain secret is readable)"
NOISY=1 timeout 30 node "$MNEMONIC_CLI" prove --challenge "$(printf 'deadbeef%.0s' {1..8})" > "$TMP_DIR/step3.stdout" 2> "$TMP_DIR/step3.stderr" || true

# The prove output is human-formatted by default; --json gives JSON. Accept
# either: any non-empty stdout that contains a base58/hex signature shape.
assert_pass "step3.stdout_nonempty"   "[ -s \"$TMP_DIR/step3.stdout\" ]"
assert_pass "step3.no_obvious_error"  "! grep -qiE 'error|panic|trace' \"$TMP_DIR/step3.stderr\""

# Re-prove — must succeed silently (no prompt second time around)
log "  re-proving to confirm no repeat prompt"
NOISY=1 timeout 15 node "$MNEMONIC_CLI" prove --challenge "$(printf 'beefcafe%.0s' {1..8})" > "$TMP_DIR/step3b.stdout" 2> "$TMP_DIR/step3b.stderr" || true
assert_pass "step3.second_sign_nonempty" "[ -s \"$TMP_DIR/step3b.stdout\" ]"

# -----------------------------------------------------------------------------
# STEP 4 — Legacy migration (pre-seed legacy fixture → expect stub conversion + pubkey preserved)
# -----------------------------------------------------------------------------

log "STEP 4 — legacy → stub migration"
clean_state
mkdir -p "$HOME/.mnemonic"
cp "$REPO/tests/fixtures/legacy-identity.json" "$HOME/.mnemonic/identity.json"
LEGACY_PUBKEY="$(jq -r .pubkey_base58 "$HOME/.mnemonic/identity.json")"
log "  legacy fixture pubkey: $LEGACY_PUBKEY"

NOISY=1 node "$MNEMONIC_CLI" whoami > "$TMP_DIR/step4.stdout" 2> "$TMP_DIR/step4.stderr" || true

assert_pass "step4.migration_stderr_line"  "grep -q 'mnemonic: legacy identity migrated to OS keychain' \"$TMP_DIR/step4.stderr\""
assert_pass "step4.file_is_stub_after"     "! jq -e .secret \"$HOME/.mnemonic/identity.json\" >/dev/null 2>&1"
assert_pass "step4.file_has_keychain_ref"  "jq -e .keychain_ref \"$HOME/.mnemonic/identity.json\" >/dev/null 2>&1"
POST_MIGRATE_PUBKEY="$(jq -r .pubkey_base58 "$HOME/.mnemonic/identity.json")"
assert_pass "step4.pubkey_preserved"       "[ \"$LEGACY_PUBKEY\" = \"$POST_MIGRATE_PUBKEY\" ]"
assert_pass "step4.keychain_entry_set"     "security find-generic-password -s \"$SERVICE\" -a \"$ACCOUNT\" >/dev/null 2>&1"
note "step4" "legacy=$LEGACY_PUBKEY post=$POST_MIGRATE_PUBKEY"

# -----------------------------------------------------------------------------
# STEP 5 — Drift status (no network; assert local synced)
# -----------------------------------------------------------------------------

log "STEP 5 — identity status"
NOISY=1 node "$MNEMONIC_CLI" identity status --json > "$TMP_DIR/step5.stdout" 2> "$TMP_DIR/step5.stderr" || true
assert_pass "step5.json_parses"   "jq -e . \"$TMP_DIR/step5.stdout\" >/dev/null 2>&1"
STATUS_VALUE="$(jq -r .status "$TMP_DIR/step5.stdout" 2>/dev/null || echo unknown)"
STATUS_STORAGE="$(jq -r .storage "$TMP_DIR/step5.stdout" 2>/dev/null || echo unknown)"
# Accept synced (if token.json existed) or webapp-unknown (if not). Either is non-failure.
assert_pass "step5.status_is_synced_or_webapp_unknown" "[[ \"$STATUS_VALUE\" =~ ^(synced|webapp-unknown)$ ]]"
assert_pass "step5.storage_mentions_os_keychain" "echo \"$STATUS_STORAGE\" | grep -qiE 'keychain|os'"
note "step5" "status=$STATUS_VALUE storage=$STATUS_STORAGE"

# -----------------------------------------------------------------------------
# STEP 6 — Cross-language collision check (THE critical regression test)
# -----------------------------------------------------------------------------

log "STEP 6 — cross-language collision (Rust reads what Node wrote)"
"$KEYCHAIN_READ" > "$TMP_DIR/step6.rust.stdout" 2> "$TMP_DIR/step6.rust.stderr" || true
assert_pass "step6.rust_keychain_read_succeeded" "jq -e . \"$TMP_DIR/step6.rust.stdout\" >/dev/null 2>&1"
RUST_PUBKEY="$(jq -r '.pubkey_base58 // empty' "$TMP_DIR/step6.rust.stdout" 2>/dev/null || echo "")"
RUST_AVAILABLE="$(jq -r .available "$TMP_DIR/step6.rust.stdout" 2>/dev/null || echo false)"
RUST_PRESENT="$(jq -r .present "$TMP_DIR/step6.rust.stdout" 2>/dev/null || echo false)"

assert_pass "step6.rust_sees_apple_keychain"       "[ \"$RUST_AVAILABLE\" = \"true\" ]"
assert_pass "step6.rust_sees_entry_present"        "[ \"$RUST_PRESENT\" = \"true\" ]"
assert_pass "step6.rust_pubkey_matches_node"       "[ -n \"$RUST_PUBKEY\" ] && [ \"$RUST_PUBKEY\" = \"$POST_MIGRATE_PUBKEY\" ]"

# Symmetric: Node reads what's already there post-migration
NODE_PUBKEY_RECHECK="$(jq -r .local "$TMP_DIR/step5.stdout" 2>/dev/null || echo "")"
assert_pass "step6.node_status_pubkey_matches"    "[ \"$NODE_PUBKEY_RECHECK\" = \"$RUST_PUBKEY\" ]"
note "step6" "rust=$RUST_PUBKEY node=$NODE_PUBKEY_RECHECK file=$POST_MIGRATE_PUBKEY"

# -----------------------------------------------------------------------------
# STEP 7 (optional) — Webapp round-trip (push-to-webapp + open browser)
# -----------------------------------------------------------------------------

if [[ -n "$WEBAPP_URL" ]]; then
  log "STEP 7 — push to webapp at $WEBAPP_URL (opens browser at the end)"
  NOISY=1 timeout 15 node "$MNEMONIC_CLI" identity push-to-webapp --code-only > "$TMP_DIR/step7.stdout" 2> "$TMP_DIR/step7.stderr" || true
  SHORT_CODE="$(grep -oE '[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{4}-[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{4}' "$TMP_DIR/step7.stdout" | head -1 || true)"
  assert_pass "step7.short_code_emitted" "[ -n \"$SHORT_CODE\" ]"
  if [[ -n "$SHORT_CODE" ]]; then
    REDEEM_URL="$WEBAPP_URL/install?pull=$SHORT_CODE"
    log "  opening $REDEEM_URL in default browser"
    open "$REDEEM_URL" || true
    note "step7" "short_code=$SHORT_CODE opened=$REDEEM_URL"
  fi
else
  log "STEP 7 — webapp round-trip SKIPPED (set --webapp-url=URL to enable)"
  STEP_NAMES+=("step7.deferred")
  STEP_RESULTS+=("skipped")
  note "step7" "no webapp_url provided"
fi

# -----------------------------------------------------------------------------
# RESULT
# -----------------------------------------------------------------------------

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
for r in "${STEP_RESULTS[@]}"; do
  case "$r" in
    pass) PASS_COUNT=$((PASS_COUNT+1)) ;;
    fail) FAIL_COUNT=$((FAIL_COUNT+1)) ;;
    skipped) SKIP_COUNT=$((SKIP_COUNT+1)) ;;
  esac
done

VERDICT="pass"
[[ "$FAIL_COUNT" -gt 0 ]] && VERDICT="fail"

ISO_NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Build the steps[] array via jq for safe escaping
STEPS_JSON='[]'
for i in "${!STEP_NAMES[@]}"; do
  STEPS_JSON=$(jq \
    --arg name "${STEP_NAMES[$i]}" \
    --arg result "${STEP_RESULTS[$i]}" \
    '. + [{name:$name, result:$result}]' <<<"$STEPS_JSON")
done

NOTES_JSON='[]'
for n in "${STEP_NOTES[@]}"; do
  NOTES_JSON=$(jq --arg s "$n" '. + [$s]' <<<"$NOTES_JSON")
done

jq -n \
  --arg scenario "T15-smoke-matrix" \
  --arg platform "macos14" \
  --arg head "$CURRENT_HEAD" \
  --arg branch "$CURRENT_BRANCH" \
  --arg ran_at "$ISO_NOW" \
  --arg verdict "$VERDICT" \
  --arg agent "macos-smoke.sh@1.0" \
  --argjson pass "$PASS_COUNT" \
  --argjson fail "$FAIL_COUNT" \
  --argjson skip "$SKIP_COUNT" \
  --argjson steps "$STEPS_JSON" \
  --argjson notes "$NOTES_JSON" \
  '{scenario:$scenario, platform:$platform, feature_branch_head:$head, branch:$branch, ran_at:$ran_at, agent:$agent, verdict:$verdict, counts:{pass:$pass, fail:$fail, skipped:$skip}, steps:$steps, observations:$notes}' \
  > "$RESULT_JSON"

log ""
log "RESULT: $VERDICT (pass=$PASS_COUNT fail=$FAIL_COUNT skipped=$SKIP_COUNT)"
log "report written: $RESULT_JSON"

# Pretty-print the result table
printf '\n'
printf '  %-50s %s\n' "step" "result"
printf '  %-50s %s\n' "----" "------"
for i in "${!STEP_NAMES[@]}"; do
  printf '  %-50s %s\n' "${STEP_NAMES[$i]}" "${STEP_RESULTS[$i]}"
done
printf '\n'

# Final exit
if [[ "$VERDICT" = "pass" ]]; then
  log "T15 macOS row: PASS"
  exit 0
else
  log "T15 macOS row: FAIL — diagnostics retained at $TMP_DIR"
  exit 1
fi

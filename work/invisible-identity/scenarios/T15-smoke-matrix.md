# T15 — Cross-platform smoke matrix scenario (Computer Use agent)

**Target runtime:** OpenAI Codex with the Computer Use tool, or equivalent (Anthropic Claude Computer Use, Open Interpreter with desktop control). The scenario assumes the agent has terminal access plus the ability to dismiss / click OS-native modal dialogs when they appear (Keychain Access on macOS, Credential Manager on Win11). Linux + Docker runs entirely in terminal — no GUI events needed.

**Branch under test:** `feat/invisible-identity`, commit `c05c9fc` or later. Verify by `cd /Users/syi/src/sessions/monorepo && git rev-parse --abbrev-ref HEAD` returns `feat/invisible-identity` and `git rev-list --count main..HEAD` returns ≥ 30.

**Output contract:** the agent writes one JSON file `T15-smoke-result-<platform>.json` per row attempted, then a final aggregation `T15-smoke-summary.json` in `work/invisible-identity/logs/working/`. Each per-platform JSON conforms to the schema in section §Reporting at the bottom. A row is PASS iff every step's `assertion_result` is `pass`; any single `fail` flips the row to FAIL.

**Termination:** The agent **must** finish each row even if early steps fail (run all 6 steps + observation capture for diagnostic value). Aggregate verdict per platform is determined by `any(step.result == 'fail')`. The agent **must NOT** push, merge, modify source code, or invoke any agent / sub-task outside this scenario.

---

## §0. Inputs the orchestrator provides at scenario start

The Computer Use agent expects these environment variables OR a JSON config file at `~/.t15/inputs.json`:

| Variable | Purpose | Example |
|---|---|---|
| `T15_PLATFORM` | Selects which row to run. One of: `macos14`, `ubuntu22_keyring`, `ubuntu22_headless`, `windows11`, `docker_alpine` | `macos14` |
| `T15_REPO_PATH` | Absolute path to the monorepo checkout on this machine | `/Users/syi/src/sessions/monorepo` |
| `T15_WEBAPP_URL` | Base URL where the webapp is reachable (for Step 6 round-trip). If empty → skip Step 6 with status `deferred` rather than `fail` | `http://localhost:5173` or `https://mnemonik.xyz` |
| `T15_WEBAPP_AUTH_COOKIE` | (Optional) raw cookie value pre-baked into a browser session, so the agent can `curl` past login. If absent and Step 6 requires login, mark Step 6 `deferred` with `reason: "no_logged_in_session_available"` | (opaque string) |
| `T15_OUTPUT_DIR` | Where to write the result JSON. Default `<repo>/work/invisible-identity/logs/working/` | (path) |

If `T15_PLATFORM` is unset or unrecognized, the agent must abort with exit code 2 and a clear message — do not guess.

---

## §1. Pre-flight (run once per platform before any step)

These commands are platform-aware but run identically across rows.

| Action | Computer-Use call (terminal) | Assertion |
|---|---|---|
| Verify branch | `git -C "$T15_REPO_PATH" rev-parse --abbrev-ref HEAD` | stdout exactly `feat/invisible-identity` |
| Verify HEAD ≥ T19 close | `git -C "$T15_REPO_PATH" merge-base --is-ancestor c05c9fc HEAD; echo $?` | stdout exactly `0` |
| Verify Rust binary present | `test -x "$T15_REPO_PATH/target/release/mnemonic-mcp" \|\| (cd "$T15_REPO_PATH" && cargo build -p mnemonic-mcp --release)` | exit 0 |
| Verify Node CLI build present | `test -f "$T15_REPO_PATH/packages/cli/dist/bin/mnemonic.js" \|\| (cd "$T15_REPO_PATH" && npm install --workspaces --include-workspace-root --no-audit --no-fund && npm run build --workspace=@mnemonik-xyz/cli)` | exit 0 |
| Capture binary paths | `MNEMONIC="$T15_REPO_PATH/packages/cli/dist/bin/mnemonic.js"; MCP_BIN="$T15_REPO_PATH/target/release/mnemonic-mcp"` | both files exist |

If any pre-flight assertion fails, the agent writes `T15-smoke-result-<platform>.json` with `verdict: "blocked"` and `blocker: "pre_flight"` and exits 1.

---

## §2. The six steps (executed in order; halt on platform-specific guard)

Each step has the same shape: an action, an assertion, and an "on failure" recovery (almost always: capture diagnostics, mark step as fail, continue).

### Step 1 — Clean state

| Platform | Action | Assertion |
|---|---|---|
| All | `rm -rf "$HOME/.mnemonic"` | `! test -e "$HOME/.mnemonic"` (exit 0 from the `! test`) |
| `macos14` | `security delete-generic-password -s xyz.mnemonik.identity 2>/dev/null \|\| true` | (deletion is best-effort; absence not an error) |
| `ubuntu22_keyring` | `secret-tool clear service xyz.mnemonik.identity account default 2>/dev/null \|\| true` | (best-effort) |
| `windows11` | (PowerShell) `cmdkey /delete:LegacyGeneric:target=xyz.mnemonik.identity 2>$null` | exit 0 or no-such-credential message |
| `ubuntu22_headless` | (no keychain to clear — skip) | n/a |
| `docker_alpine` | (no keychain to clear — skip) | n/a |

**No GUI events required** for Step 1.

### Step 2 — Bootstrap

| Sub-action | Command | Assertion |
|---|---|---|
| Run `whoami` to trigger bootstrap | `node "$MNEMONIC" whoami 2> /tmp/t15-step2.stderr 1> /tmp/t15-step2.stdout` | exit 0 |
| Capture identity file shape | `cat "$HOME/.mnemonic/identity.json"` | parses as JSON |
| Capture stderr creation line | `cat /tmp/t15-step2.stderr` | (see platform table below) |
| Capture README | `test -f "$HOME/.mnemonic/README.txt" && wc -l "$HOME/.mnemonic/README.txt"` | file exists, ≥ 5 lines |

**Platform-specific stderr assertion** (regex against the captured stderr; case-sensitive):

| Platform | Expected stderr pattern | Expected identity.json shape |
|---|---|---|
| `macos14` | `mnemonic: identity created did:sol:[1-9A-HJ-NP-Za-km-z]{32,44} stored in OS keychain` | Stub: keys include `pubkey_base58`, `keychain_ref`, `created_at`; NO `secret` key |
| `ubuntu22_keyring` | Same as macos14 | Same stub shape |
| `windows11` | Same as macos14 | Same stub shape |
| `ubuntu22_headless` | `mnemonic: identity created did:sol:[1-9A-HJ-NP-Za-km-z]{32,44} stored in ~/.mnemonic/identity.json \(OS keychain unavailable: .+\)` | Legacy: `secret` (array of 64 numbers) + `pubkey_base58` |
| `docker_alpine` | Same as ubuntu22_headless | Legacy shape |

**No GUI events required for Step 2.** macOS does NOT prompt at bootstrap (Decision 3 — lazy access).

### Step 3 — Sign (exercises actual keychain unlock on platforms with OS keychain)

| Sub-action | Command | Assertion |
|---|---|---|
| First `sign` | `node "$MNEMONIC" sign "t15-step3-test-memory" 2> /tmp/t15-step3-first.stderr 1> /tmp/t15-step3-first.stdout` | exit 0 |

**Platform-specific GUI handling for Step 3:**

| Platform | GUI prompt expected? | Computer Use action |
|---|---|---|
| `macos14` | NO — provided `scripts/macos-prep-keychain.sh` was run once between Step 2 and Step 3 of this row. Run it now: `"$T15_REPO_PATH/scripts/macos-prep-keychain.sh"` — prompts for the login password ONCE, widens the entry's partition list to `apple-tool:,unsigned:`, then subsequent reads are silent. **If the script can't run** (no TTY for password, `T15_KEYCHAIN_PASSWORD` not provided), fall back to the legacy GUI path: (a) screenshot the screen, (b) if a modal contains "Keychain wants to use" or "Allow access", click the **"Always Allow"** button, (c) wait 2s, re-screenshot, (d) mark step diagnostic `acl_widened: false` so the operator knows future runs will keep prompting. |
| `ubuntu22_keyring` | Sometimes — only if the keyring is locked. If a modal appears with the text "Unlock keyring", type the user's login password (or skip with status `deferred` if no password is available). If no modal appears within 3s, proceed. | Wait 3s after invocation; screenshot; if no modal, continue. If modal, mark `deferred: requires_user_password`. |
| `windows11` | NO modal expected (Credential Manager doesn't prompt for current-user credentials). | None |
| `ubuntu22_headless` | NO modal (no GUI). | None |
| `docker_alpine` | NO modal. | None |

| Sub-action | Command | Assertion |
|---|---|---|
| Verify signature output is hex-ish | `jq -e '.signature_hex \|\| .signature \|\| .attestation' /tmp/t15-step3-first.stdout` | exit 0 (some signature-shaped key present in JSON output) |
| Second `sign` (must NOT prompt) | `node "$MNEMONIC" sign "t15-step3-test-memory-2" 2> /tmp/t15-step3-second.stderr 1> /tmp/t15-step3-second.stdout` | exit 0 AND no GUI modal appears within 3s (agent screenshots & confirms) |

### Step 4 — Legacy migration

**Skip Step 4 entirely** on platforms with no keychain (`ubuntu22_headless`, `docker_alpine`). Set `step4.result: "skipped"` with `reason: "no_os_keychain_on_this_platform"`.

| Sub-action | Command | Assertion |
|---|---|---|
| Reset clean state | (repeat Step 1 commands for this platform) | |
| Pre-seed legacy fixture | `mkdir -p "$HOME/.mnemonic" && cp "$T15_REPO_PATH/tests/fixtures/legacy-identity.json" "$HOME/.mnemonic/identity.json"` | file exists |
| Capture pre-migration pubkey | `PUBKEY_BEFORE=$(jq -r .pubkey_base58 "$HOME/.mnemonic/identity.json")` | non-empty base58-ish string |
| Run `whoami` to trigger migration | `node "$MNEMONIC" whoami 2> /tmp/t15-step4.stderr 1> /tmp/t15-step4.stdout` | exit 0 |
| Verify migration stderr line | `cat /tmp/t15-step4.stderr` | matches `mnemonic: legacy identity migrated to OS keychain` |
| Verify file is now stub shape | `jq -e '.keychain_ref' "$HOME/.mnemonic/identity.json"` | exit 0 (key exists) |
| Verify pubkey preserved | `PUBKEY_AFTER=$(jq -r .pubkey_base58 "$HOME/.mnemonic/identity.json"); test "$PUBKEY_BEFORE" = "$PUBKEY_AFTER"` | exit 0 |
| Verify secret key REMOVED from file | `jq -e '.secret \| not' "$HOME/.mnemonic/identity.json"` | exit 0 (.secret either missing or falsy) |

GUI handling: on macOS this step MAY prompt for keychain access again because the `.set()` to a never-before-seen entry happens. If it does, click "Always Allow" the same way as Step 3.

### Step 5 — Drift status (no network)

| Sub-action | Command | Assertion |
|---|---|---|
| Run status (human mode) | `node "$MNEMONIC" identity status 2> /tmp/t15-step5-human.stderr 1> /tmp/t15-step5-human.stdout; echo "EXIT=$?"` | EXIT 0 (synced) or EXIT 0 (webapp-unknown) or EXIT 1 (no-identity if Step 1/2 went wrong) or EXIT 3 (diverged) |
| Run status (JSON) | `node "$MNEMONIC" identity status --json 2>/dev/null` | JSON parses; has keys `local`, `jwt_sub`, `storage`, `status` |
| Verify storage label matches platform | `node "$MNEMONIC" identity status --json \| jq -r .storage` | macOS → "OS keychain (macOS Keychain)"; ubuntu22_keyring → "OS keychain (Secret Service)"; windows11 → "OS keychain (Windows Credential Manager)"; ubuntu22_headless → "file (...)" or similar containing "file"; docker_alpine → contains "file" |

**No GUI events required for Step 5.** This step DOES read from the OS keychain (via `get`) on platforms with one — if macOS prompts again (it shouldn't after Step 3 "Always Allow"), click "Always Allow" again and capture the screenshot.

### Step 6 — Round-trip with webapp (push-to-webapp + browser redemption)

**Skip Step 6 with status `deferred` if `T15_WEBAPP_URL` is empty.**

**Skip Step 6 with status `deferred: requires_webapp_login` if `T15_WEBAPP_AUTH_COOKIE` is empty AND the webapp returns 401 / 403 on the redeem POST.**

| Sub-action | Command | Assertion |
|---|---|---|
| Issue ticket from CLI | `node "$MNEMONIC" identity push-to-webapp --code-only 2> /tmp/t15-step6.stderr 1> /tmp/t15-step6.stdout` (no QR — text-only is friendlier for the agent to parse) | exit 0; stdout contains `https?://[^\s]+/install\?pull=` and an 8+char short_code matching `[23456789ABCDEFGHJKLMNPQRSTUVWXYZ-]+` |
| Capture short_code | `SHORT_CODE=$(grep -oE '[A-Z0-9]{4}-[A-Z0-9]{4}' /tmp/t15-step6.stdout \| head -1)` | non-empty |
| Capture URL | `URL=$(grep -oE 'https?://[^\s]+/install\?pull=[A-Z0-9-]+' /tmp/t15-step6.stdout \| head -1)` | non-empty |
| Programmatically redeem (no GUI) | Generate a fake `redeemer_eph_pub` (32 bytes base64) and `POST $T15_WEBAPP_URL/api/cli-bootstrap/redeem` with `{"short_code": "$SHORT_CODE", "redeemer_eph_pub": "..."}`. Use `curl -fSL` with `--cookie "$T15_WEBAPP_AUTH_COOKIE"` if provided. | HTTP 200 |
| Optional: open URL in browser (GUI flow) | If the platform has a default browser, open `URL` via `open` (macOS) / `xdg-open` (Linux) / `start` (Windows). Wait 5s. Screenshot. Verify the page loaded — look for visible text "Adopting CLI identity" or "CLI identity adopted" or an error toast. | screenshot shows one of the three expected states |
| Verify drift now resolved | `node "$MNEMONIC" identity status --json \| jq -r .status` | `synced` OR `webapp-unknown` (depending on whether the test webapp issued a JWT) |

The `curl`-based redeem is the primary assertion. The browser-open is a UX/GUI sanity check; if the browser open fails on a particular platform (e.g., Docker alpine has no browser), record it as observation but don't fail the step.

---

## §3. Cleanup (always run, regardless of pass/fail)

| Action | Command |
|---|---|
| Clear keychain entry | platform-specific Step-1 command repeated |
| Remove .mnemonic dir | `rm -rf "$HOME/.mnemonic"` |
| Remove /tmp/t15-* files | `rm -f /tmp/t15-*` |
| Capture exit state | record final `git status --short` and `git rev-parse HEAD` to per-platform JSON |

---

## §4. Reporting

After the 6 steps + cleanup, write `$T15_OUTPUT_DIR/T15-smoke-result-<platform>.json`:

```json
{
  "scenario": "T15-smoke-matrix",
  "scenario_version": "1",
  "platform": "macos14|ubuntu22_keyring|ubuntu22_headless|windows11|docker_alpine",
  "feature_branch_head": "<git rev-parse HEAD>",
  "ran_at": "<ISO-8601 timestamp>",
  "agent": "<agent identifier — e.g., 'codex-computer-use@1.0' or 'claude-cu@1.0'>",
  "verdict": "pass|fail|blocked",
  "blocker": null | "pre_flight" | "<reason>",
  "steps": [
    {
      "step": 1,
      "name": "clean_state",
      "result": "pass|fail|skipped",
      "command": "...",
      "stdout": "...",
      "stderr": "...",
      "exit_code": 0,
      "assertion": "...",
      "assertion_result": "pass|fail",
      "diagnostics": { "screenshots": ["..."], "notes": "..." }
    },
    ...
  ],
  "open_observations": [
    "Free-form notes the agent thought were worth flagging — e.g., 'macOS modal title was slightly different from spec', 'Bun was substituted for npm because npm not installed'."
  ]
}
```

After all platforms requested by the orchestrator have results, the agent writes `T15-smoke-summary.json` aggregating them:

```json
{
  "scenario": "T15-smoke-matrix",
  "summary_version": "1",
  "rows": {
    "macos14":         {"verdict": "pass|fail|blocked|not_attempted", "result_file": "T15-smoke-result-macos14.json"},
    "ubuntu22_keyring": {"verdict": "...", "result_file": "..."},
    "ubuntu22_headless": {"verdict": "...", "result_file": "..."},
    "windows11":       {"verdict": "...", "result_file": "..."},
    "docker_alpine":   {"verdict": "...", "result_file": "..."}
  },
  "overall_verdict": "pass|fail|partial",
  "merge_readiness": "ready_to_merge|blocked_by_failures|partial_coverage_acceptable",
  "decisions_md_block": "## Task 15: Cross-platform smoke matrix\n\n**Status:** Done\n**Operator:** <agent>\n**Date:** <date>\n...full markdown block ready to paste into work/invisible-identity/decisions.md..."
}
```

The `decisions_md_block` is the canonical text to append to `decisions.md`. Once the agent has produced it, a human reviewer can either paste it directly OR run a one-line script to insert it.

---

## §5. Failure handling rules (binding for the agent)

1. **Never modify source code** — this scenario is read-only against the feature branch except for `$HOME/.mnemonic/` and `/tmp/t15-*`. If the agent needs to "fix" something to make a step pass, that's a real failure — record and continue.

2. **Never push, commit, merge, or open PRs.** The agent has read-only git access conceptually; only the local working tree under `$HOME` and `/tmp` is writable.

3. **Never inject keystrokes or click into unexpected UI surfaces.** Only the specific GUI events in Steps 3 / 4 / 6 are sanctioned. If an unexpected modal appears (e.g., macOS update prompt, browser auto-update, system notifications), screenshot it, dismiss it neutrally if possible (Escape key), and record the interruption in `open_observations`.

4. **Time budgets per step**: each step has a soft 60s budget. If a step's command hangs past 60s, kill it, mark `result: fail`, record `diagnostics.timeout: true`, move to the next step. Total scenario time per platform should not exceed 8 minutes including cleanup.

5. **Cleanup is mandatory.** Run Section §3 even on early termination. The agent's last write to `$T15_OUTPUT_DIR` should be the result JSON, after cleanup completes.

6. **Sensitive output**: never write the contents of `~/.mnemonic/identity.json`'s `secret` field, or any keychain entry's raw bytes, to the result JSON. When capturing stdout/stderr that may contain a base58 pubkey, those are OK to record. When capturing a `legacy-identity.json` that has the secret array, log its length only (`secret.length` rather than `secret`).

---

## §6. How the orchestrator invokes the agent

The expected outer invocation, parameterized:

```text
You are a Computer Use agent. Run the T15 smoke matrix scenario at <repo>/work/invisible-identity/logs/working/T15-smoke-matrix-scenario.md.

Platform: <T15_PLATFORM>
Repo: <T15_REPO_PATH>
Webapp URL: <T15_WEBAPP_URL>
Webapp auth cookie: <T15_WEBAPP_AUTH_COOKIE-or-empty>
Output dir: <T15_OUTPUT_DIR>

Follow the scenario verbatim. Write per-platform JSON results. Return a one-paragraph human summary at the end with the verdict and any blockers.
```

The orchestrator (you, or another agent) runs this once per platform — five invocations total to cover the full matrix. Each invocation is independent (the agent does not need to know about other platforms).

After all five completions, the orchestrator runs a small aggregator script (or another agent invocation) that reads the five `T15-smoke-result-*.json` files, produces `T15-smoke-summary.json`, and proposes the `decisions_md_block` for human paste-in.

---

## §7. Why this scenario, not the original checklist?

The original `T15-smoke-matrix-checklist.md` was a human-readable Markdown checklist with prose like "click Always Allow once". A Computer Use agent needs:

- **Precise GUI affordances** — specifying the modal text to look for and the button label to click, not "approve the prompt"
- **Explicit assertions per step** — exit codes + regex matches against captured stdout/stderr, not "verify it worked"
- **Failure recovery rules** — what counts as a hang, what counts as cleanup, when to abort early
- **Output contract** — structured JSON the orchestrator can aggregate, not a human-paste table
- **Sensitive-data handling** — explicit "do not log the secret bytes" rule (humans implicitly know; agents need it written)
- **Time budgets** — humans know to walk away from a hung command; an agent will sit in `wait_for_output` forever

This file is the "agent-runnable" version. The original checklist remains for humans who want to drive the matrix by hand.

---

## §8. Open follow-ups (for the human orchestrator, not the agent)

1. **Webapp test login**: provide a stable `T15_WEBAPP_AUTH_COOKIE` for a long-lived test user (or set up an `?test_user=...` query-param dev-mode shortcut on the webapp's OAuth flow) so Step 6 can run unattended on platforms with browsers.

2. ✅ **macOS keychain pre-authorization** — RESOLVED in commit b46ec20. `scripts/macos-prep-keychain.sh` widens the `xyz.mnemonik.identity/default` partition list to `apple-tool:,unsigned:` via `security set-generic-password-partition-list`. Operator (or the agent itself) runs the script once after first bootstrap; subsequent reads are silent. Residual: the script prompts for the login password once (keychain admin gate). For fully unattended runs, stage `T15_KEYCHAIN_PASSWORD` and have the agent pipe it via `expect`. The legacy "Always Allow" GUI fallback in Step 3 remains for environments where the script can't run.

3. **Windows 11 runner**: confirm GitHub Actions' `windows-2022` runner can drive Credential Manager unattended. If yes, this scenario could be re-classified from "manual smoke" to "CI smoke on a Windows runner" — closing the gap left by Wave 3 CI (which only covers Linux + gnome-keyring).

4. **Docker alpine variant**: confirm `node` + `npm` + a recent Bun work on `alpine:latest` for the file-fallback row. The repo's CLI may pull a transitive dep that needs `musl` rather than `gnu` libc — the `@napi-rs/keyring` `linux-x64-musl` prebuilt should cover this, but verify via the actual run.

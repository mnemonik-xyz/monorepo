# Mnemonic — Pre-demo Manual Smoke Checklist

Manual end-to-end smoke test for Phase 1 hosted MCP. Executed by a single operator
against the deployed system before each stage demo, by pre-deploy QA (Task 12),
and by post-deploy QA (Task 14).

**Purpose.** Catch UX, OAuth, and cross-tool regressions on the deferred-sign flow
(Decision 4) before a live audience sees them. The flow under test:
`tools/call sign_memory` → `awaiting_signature` → `/sign/<id>` → WASM sign →
`/api/sign-callback` → `recall` returns the row.

**Total ETA budget:** ≤ 30 minutes (1800s) for the happy path. Per-step ETAs sum
must not exceed the budget. Slack of ~10 minutes is built into the cap.

**Failure policy.** If a step's Recovery does not unblock progress within 5
minutes, abort the run, record which step failed and the verbatim observation,
and report to the spec author. Do not proceed past a failed step.

## Prerequisites

- Laptop with Cursor installed (any recent version with MCP connector support).
- Active Claude.ai Pro subscription (custom connectors require Pro tier).
- Fresh Chrome profile or incognito window with empty `localStorage` for
  `mnemonik.xyz`. No browser extensions loaded.
- Second laptop accessible (or a second fully-clean Chrome profile) for step 10.
- Hosted MCP deployed: `https://mcp.mnemonik.xyz` reachable. Verify with
  `curl -fI https://mcp.mnemonik.xyz/health` → `HTTP/2 200` before starting.
- Webapp deployed: `https://mnemonik.xyz` reachable, all four routes (`/`,
  `/install`, `/chat`, `/sign/<id>`) return 200.
- Admin emergency-demo keypair backup file accessible offline (fallback identity
  if the operator's keypair is lost mid-run).
- Local-only validation note: if the hosted endpoint is not deployed yet,
  step 8 (Claude.ai Pro) is skipped and step 10 is simulated with a second
  Chrome profile on the same laptop.

## Step 1 — Fresh-browser onboarding

### Action
Open Chrome incognito (or a fresh profile). Navigate to `https://mnemonik.xyz/`.

### Expected
Landing page renders with heading `Verifiable memory for AI agents` and a
visible `Get started` link pointing to `/install`. DevTools → Application →
Local Storage shows no entries for `mnemonik.xyz`.

### Recovery
If localStorage is non-empty: DevTools → Application → Local Storage → right-click
the `mnemonik.xyz` origin → Clear, then hard-reload (`Cmd+Shift+R`).

### ETA
~30s

## Step 2 — Keypair generation via WASM identity panel

### Action
Click `Get started`. On `/install`, locate the IdentityPanel. If no keypair is
shown, click `Generate keypair`.

### Expected
The panel displays a `did:sol:<base58>` line and a base58 pubkey
(43–44 chars). DevTools → Application → Local Storage shows
`mnemonic.identity` populated with a JSON value containing `secret_key` and
`pubkey`.

### Recovery
If the panel reports `WASM module failed to load`: hard-reload, check
DevTools → Network for a 200 on `mnemonic_core_bg-*.wasm`. If the wasm file
404s, the deploy is broken — abort and report.

### ETA
~30s

## Step 3 — Keypair backup download

### Action
Click `Download keypair backup`. Save the file to disk and open it in a text
editor.

### Expected
File name matches `mnemonic-keypair-<pubkey-prefix>.json`. JSON content has
non-empty `secret_key` (base58 or byte array) and `pubkey` fields. The
`pubkey` field equals the pubkey shown in the IdentityPanel.

### Recovery
If the download is empty or missing `secret_key`: regenerate via
`Clear identity` then `Generate keypair`, retry the download.

### ETA
~30s

## Step 4 — Install deeplink to Cursor

### Action
Click `Install in Cursor`. Accept the OS prompt to open Cursor when it appears.

### Expected
Cursor focuses to the foreground and shows an MCP-connector confirmation dialog
naming `Mnemonic` with URL `https://mcp.mnemonik.xyz/mcp`. Accepting the
dialog adds the connector to Cursor's MCP settings.

### Recovery
If the deeplink does not open Cursor: copy the value of the `<a href>` from
the button (DevTools → Elements), paste into a new Chrome tab, accept the
follow-up prompt. If Cursor rejects the connector with `redirect_uri not
registered`: confirm `smithery.yaml` and the deploy match the registered
redirect URL; rerun deploy if drift detected.

### ETA
~1m

## Step 5 — OAuth approve flow with user-signed challenge

### Action
Cursor opens the browser to
`https://mcp.mnemonik.xyz/oauth/authorize?...`. On the consent page, click
`Approve`.

### Expected
Browser shows `Approved` confirmation; tab returns control to Cursor. Cursor
status indicator for the Mnemonic connector reads `Connected` (or equivalent
ready state). DevTools → Application → Local Storage now also has
`mnemonic.jwt` populated.

### Recovery
If the consent page errors with `signature verification failed`: clear
`mnemonic.jwt` from localStorage, reload the consent URL, click `Approve`
again. If the OAuth modal does not appear at all:
`curl -fI https://mcp.mnemonik.xyz/health` should return 200; if not, the
hosted MCP is down — fall back to the live-demo backup plan.

### ETA
~30s

## Step 6 — `sign_memory` from Cursor → `/sign/<id>`

### Action
In Cursor, open a new chat and type:
`save this onchain: smoke-test memory 1`. When Cursor's tool-use prompt
appears for `mnemonic_sign_memory`, accept it.

### Expected
The tool result JSON contains
`{"status":"awaiting_signature","approve_url":"https://mnemonik.xyz/sign/<uuid>","correlation_id":"<uuid>","expires_in":300}`.
Open `approve_url` in the same browser. The `/sign/<correlation_id>` page
shows a content preview reading `smoke-test memory 1`, a countdown timer
formatted `Expires in mm:ss`, and a `Sign with my Mnemonic identity` button.
Click `Sign`. Page reports success (status text changes to `Signed` or a
green confirmation). DevTools → Network shows
`POST https://mcp.mnemonik.xyz/api/sign-callback` returned `200`.

### Recovery
If the page shows `410 Gone` or `expired`: the 5-minute TTL elapsed; rerun
the Cursor tool call to mint a new `correlation_id`, then revisit the new
`approve_url`. If the Sign button errors `keypair not found`: the browser
profile is wrong (likely lost localStorage after a Cursor-driven redirect);
reload `/install` to confirm the keypair is present, then retry.

### ETA
~2m

## Step 7 — `recall` in same Cursor session

### Action
In the same Cursor chat, type: `recall my recent saves`. Accept the
`mnemonic_recall` tool prompt when it appears.

### Expected
The tool result includes at least one attestation row whose `content` field
equals `smoke-test memory 1`. The `attestation_id` matches the one returned
by step 6's sign-callback (or is the deterministic ID derived from the signed
bundle).

### Recovery
If recall returns zero rows: confirm the sign-callback in step 6 returned
`200`; if it did, the row should exist — re-issue `recall` with a more
specific query (`recall smoke-test memory 1`). If still empty, the
ownership-filter bug from Decision 9 may have regressed; abort and report.

### ETA
~30s

## Step 8 — Switch to Claude.ai Pro and add custom connector

### Action
Open `https://claude.ai/` in the same browser. Go to Settings → Connectors →
`Add custom connector`. Paste `https://mcp.mnemonik.xyz` into the URL field
and confirm. When the OAuth flow opens, click `Approve`.

### Expected
The OAuth flow redirects to the same consent page from step 5; the
IdentityPanel still shows the same `did:sol:<pubkey>`. After approval,
Claude.ai's Connectors list shows `Mnemonic` with status `Connected`.

### Recovery
If Claude.ai rejects the URL with `unable to reach server`: confirm
`curl -fI https://mcp.mnemonik.xyz/health` returns 200 and that
`https://mcp.mnemonik.xyz/.well-known/oauth-authorization-server` is
reachable and returns valid JSON. If still failing, fall back to the
backup plan.

### ETA
~1m

## Step 9 — Recall in Claude.ai returns the same attestation

### Action
Open a fresh Claude.ai chat. Type: `recall my recent saves` and let Claude
invoke the connector tool when prompted.

### Expected
Claude's response includes the attestation signed in step 6; the content
string equals `smoke-test memory 1` and the `attestation_id` matches the
value observed in step 7.

### Recovery
If Claude returns `no memories found`: re-issue with the verbatim phrase
`recall smoke-test memory 1`. If still empty, the JWT issued in step 8 may
be scoped to a different pubkey than step 5 — confirm via DevTools →
Application → Local Storage that `mnemonic.identity` was unchanged
between step 5 and step 8.

### ETA
~1m

## Step 10 — Cross-device flow

### Action
On the second laptop (or a second fully-clean Chrome profile), open
`https://mnemonik.xyz/install`. Click `Restore from backup`, upload the JSON
saved in step 3. Then click `Install in Cursor`, accept the deeplink, and
complete OAuth as in steps 4–5. In Cursor on the second laptop, type:
`recall my recent saves`.

### Expected
The IdentityPanel after import shows the exact same `did:sol:<pubkey>` as
step 2. The Cursor connector installs successfully and the recall result
includes the attestation `smoke-test memory 1` from step 6.

### Recovery
If the imported keypair shows a different DID than step 2: the import path is
broken; abort and file a bug — do not proceed. If recall returns empty on
the second device but the DID matches: confirm the JWT was actually issued
(localStorage `mnemonic.jwt` populated) and that the JWT `sub` decodes to
the same pubkey.

### ETA
~5m

## Pre-demo dry run

Run this checklist 24 hours before each stage demo to catch environmental
drift. Use the production-deployed `mnemonik.xyz` and `mcp.mnemonik.xyz`,
not staging. Record results in the run-log table below. If any step fails
on the dry run, fix the underlying issue, redeploy if needed, and re-run
the affected step before the demo. Log the dry-run timestamp, operator,
and pass/fail status in `decisions.md` per the post-completion entry for
Task 9. If a Recovery step had to be invoked, update this checklist with
the lesson learned before pre-deploy QA (Task 12) reuses the file.

## Live-demo backup plan

Mitigates user-spec Risk R7 (live-demo failure on stage).

- **Pre-recorded fallback video.** Hosted at
  `https://mnemonik.xyz/demo-fallback.mp4` (placeholder URL — update once the
  recording is uploaded). Records the full step 1–10 happy path on the same
  account that will be used live. Loaded into the demo machine's browser
  before the stage talk; one-click play if the live run stalls.
- **Local stdio MCP fallback.** If `mcp.mnemonik.xyz` is unreachable mid-demo,
  switch the connector in Cursor to a local stdio binary:
  `cargo run -p mnemonic-mcp --release --features local-embed -- --transport stdio`.
  Pre-built release binary lives at
  `target/release/mnemonic-mcp` on the demo laptop; `STORAGE_MODE=local` and
  `PAYMENT_MODE=none` env vars pre-set in the demo shell profile. The local
  binary uses a file-based keypair (different DID), so attestations created
  on the local fallback are not the same dataset as the hosted run — frame
  the fallback as a `self-host preview`, not as continuity of the prior
  recall.
- **Network preflight.** Before stepping on stage: run
  `curl -fI https://mcp.mnemonik.xyz/health` and
  `curl -fI https://mnemonik.xyz/install` from the demo laptop. Both must
  return 200 within 1s. If either fails, switch to the pre-recorded video
  before the talk begins.

## Run log template

Copy this table into the QA report. Fill in once per dry run and once per
live demo run.

| Step | Start (HH:MM:SS) | End (HH:MM:SS) | Pass/Fail | Notes |
|------|------------------|----------------|-----------|-------|
| 1    |                  |                |           |       |
| 2    |                  |                |           |       |
| 3    |                  |                |           |       |
| 4    |                  |                |           |       |
| 5    |                  |                |           |       |
| 6    |                  |                |           |       |
| 7    |                  |                |           |       |
| 8    |                  |                |           |       |
| 9    |                  |                |           |       |
| 10   |                  |                |           |       |

**Total wall-clock:** _____ minutes (target ≤ 30).
**Recovery invocations:** _____ (note step number and verbatim observation if any).
**Operator:** _____.
**Date:** _____.

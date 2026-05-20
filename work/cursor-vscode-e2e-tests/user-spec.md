---
created: 2026-05-02
status: draft
type: feature
size: M
---

# User Spec: End-to-end test coverage for Cursor / VS Code / Claude Desktop MCP integration

## Что делаем

Build a layered test suite that covers the full Mnemonic MCP authorization + tool-call flow from each major client perspective: **Cursor**, **VS Code (with Copilot Chat MCP)**, **Claude Desktop**. Per m13v's guidance on issue #59, the suite is split into three tiers by determinism:

1. **Tier 1 — CI-runnable, deterministic.** Network-free Rust unit tests + Playwright tests against the live (or local) MCP server that exercise the protocol-level flows: tools/list shape, allowlist semantics, OAuth handshake, deferred-sign flow, WWW-Authenticate header presence, path-specific `/.well-known/oauth-protected-resource/mcp` shape, install-deeplink URL formats. PTY-driven CLI tests (via `child_process.spawn` already in use) cover `mnemonic init / login / sign / recall / verify`. Run on every PR.

2. **Tier 2 — Local macOS smoke.** `cliclick` + AppleScript scripts that exercise the GUI install deeplinks: opening the install URL on `mnemonik.xyz/install`, clicking the "Install in Cursor" / "Install in VS Code" button, observing the deeplink hand-off to the right app, asserting the MCP server appears in the app's MCP config. Run manually or via `make smoke`. Documented as the canonical pre-release verification.

3. **Tier 3 — Documented manual verification.** A markdown runbook (`work/cursor-vscode-e2e-tests/manual-verify.md`) walks through: install → authorize via OAuth → call `mnemonic_whoami` from chat → see real `solana_tx` for a sign call → verify on Solscan. Twenty-step checklist; signed off by the operator before launch.

## Зачем

Today's session surfaced multiple regressions that were missed because no automated test covers them:

- The `tools/list` count assertion in `mcp.rs` was the ONLY guard against tool-list shape changes.
- No test asserted the **WWW-Authenticate header** on 401 responses (MCP-spec mandate). Adding the path-specific protected-resource endpoint required manual verification.
- The **VS Code deeplink format** (`vscode://` vs `vscode:`) regressed to a Safari-incompatible form; only spotted when a user reported it.
- The **Cursor MCP OAuth flow** stalled silently for non-directory servers without a Connect button — no test could catch this UX gap from inside our codebase, but a smoke script that runs `cliclick` on the actual app would catch it the moment Cursor's UI changes.

Without tests, every server-side change that touches OAuth, tools/list, install-deeplink, or webapp Sign.tsx requires a manual round-trip through every client. That's hours of friction per release and the thing that broke twice today already.

## Как должно работать

**Tier 1 acceptance:**

- `cargo test --workspace` covers: WWW-Authenticate header on every 401 from /mcp; Claims attached on allowlisted discovery methods when a valid JWT is present; tools/list returns exactly 6 tools by name; `/.well-known/oauth-protected-resource/mcp` returns `resource: "<origin>/mcp"`; root variant still returns `resource: "<origin>"`.
- `npx playwright test webapp/e2e/` covers: install deeplinks emit the documented URL formats (Cursor `cursor://anysphere.cursor-deeplink/...`, VS Code `vscode://mcp/install?...`, Claude.ai pasted-URL); OAuth handshake against live server completes end-to-end; deferred-sign flow returns real `solana_tx` when STORAGE_MODE=full; existing tests stay green.
- `npm test --workspace=packages/cli --workspace=packages/sdk` covers: existing CLI / SDK unit + integration suite.

**Tier 2 acceptance:**

- A `bash work/cursor-vscode-e2e-tests/smoke/cursor.sh` script that, on a Mac with Cursor + cliclick installed, performs:
  1. `open 'cursor://anysphere.cursor-deeplink/mcp/install?name=Mnemonic&config=...'`
  2. waits for Cursor's MCP install dialog (~3s)
  3. uses `cliclick c:` clicks to confirm
  4. opens Cursor settings, screenshots, asserts "Mnemonic" appears under MCP Servers
  5. exits 0 if all checks pass
- Same for VS Code (`smoke/vscode.sh`) and Claude Desktop (`smoke/claude-desktop.sh`).
- Smoke scripts are MANUAL — operator runs them before each `v*` tag push. Not in CI (would need a macOS GUI runner).

**Tier 3 acceptance:**

- `manual-verify.md` is a 20-step Markdown checklist with screenshots. Includes "if you see X, file an issue / re-run step Y".
- Operator runs through it before the public soft launch on May 4.

## Out of scope

- Headless macOS GUI testing in CI (requires self-hosted macOS runners + AX permissions; high cost, low ROI for hackathon stage).
- OCR-based terminal output verification (per m13v's note: terminals are AX-opaque; PTY runners give deterministic stdin/stdout instead).
- Cursor's directory submission (separate workstream — devrel outreach, not engineering).
- Tests that assume a specific Cursor / VS Code build version. Each smoke script targets the current stable version at the time of writing; document the tested version inline.

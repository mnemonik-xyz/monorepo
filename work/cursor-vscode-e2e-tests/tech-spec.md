---
created: 2026-05-02
status: draft
related_issue: https://github.com/mnemonik-xyz/monorepo/issues/59
references:
  - issue 59 comment by m13v: terminals are AX-opaque; use PTY runners for behavior, cliclick only for install/launch smoke
---

# Tech Spec: Cursor / VS Code / Claude Desktop E2E test coverage

## Architecture

Three tiers, by determinism + cost:

```
Tier 1 — CI-runnable (every PR)
├── Rust: mcp/src/oauth.rs    → mod tests   (oauth handlers + middleware)
├── Rust: mcp/src/tools.rs     → mod tests   (mcp_auth, check_pending, recall semantics)
├── Rust: mcp/src/api.rs       → mod tests   (sign-callback storage_mode branching)
├── TS:   webapp/src/components/InstallButtons.test.tsx (deeplink format)
├── TS:   webapp/e2e/oauth-flow.spec.ts                  (browser OAuth dance)
├── TS:   webapp/e2e/deferred-sign-flow.spec.ts          (sign approval → callback)
├── TS:   webapp/e2e/mcp-auth-tool.spec.ts        ← NEW (mcp_auth + WWW-Auth + protected-resource shape, against live server)
└── TS:   packages/cli/test/integration/cli-flows.test.ts (PTY init/login/sign/recall/verify)

Tier 2 — Local macOS smoke (manual, pre-release)
├── work/cursor-vscode-e2e-tests/smoke/cursor.sh        ← NEW (cliclick + open URL)
├── work/cursor-vscode-e2e-tests/smoke/vscode.sh        ← NEW
├── work/cursor-vscode-e2e-tests/smoke/claude-desktop.sh ← NEW
└── work/cursor-vscode-e2e-tests/smoke/Makefile         ← NEW (`make smoke` runs all)

Tier 3 — Manual verification runbook (pre-launch sign-off)
└── work/cursor-vscode-e2e-tests/manual-verify.md       ← NEW (20-step checklist)
```

## Decisions

1. **No headless GUI in CI.** Driving Cursor / VS Code from a CI runner requires (a) macOS runner, (b) AX permissions granted, (c) Cursor / VS Code installed and signed in, (d) reproducible window-state. The combinatorial cost outweighs the ROI for a hackathon-stage project. Tier 2 smoke runs on the operator's Mac before a release tag.
2. **PTY for CLI behavior, child_process for argv-shape.** The existing `cli-flows.test.ts` uses `child_process.spawn` and works fine for argv parsing + exit codes + env propagation. PTY (`node-pty`) is reserved for tests that need to verify interactive prompts (e.g. password-confirm dialogs in `mnemonic identity export`). Don't migrate the working tests just because the new ones use PTY.
3. **Use httpmock for new server-side tests.** The Rust tests already use `httpmock` for HTTP-side interactions; same crate stays. Network-free tests use `tower::ServiceExt::oneshot` against an in-test router (already the pattern in `oauth.rs::tests`).
4. **Cliclick + AppleScript hybrid in smoke scripts.** AppleScript launches the URL (deeplink hand-off); cliclick performs UI-level clicks once the GUI is up. AX-opaque widgets (text canvases) are NOT asserted — assertions only on (a) "did the right app open?", (b) "does the MCP server appear in the app's config file on disk?". The latter is a file-system assertion, not a UI one — bypasses the AX gap entirely.
5. **OAuth flow assertions stay against live mcp.mnemonik.xyz.** The existing `oauth-flow.spec.ts` already does this. Doing it against a local mock would lose the rate-limit behavior and the redirect-uri allowlist behavior that real-world clients hit.

## Implementation Tasks

| # | Description | Files | Est |
|---|---|---|---|
| 1 | Rust unit tests for WWW-Authenticate header presence on every 401 path through `bearer_auth_middleware` | `mcp/src/oauth.rs` | 30m |
| 2 | Rust unit tests for `mcp_auth` allowlist + Claims-extraction-when-present semantics | `mcp/src/oauth.rs`, `mcp/src/tools.rs` | 30m |
| 3 | Rust unit tests for `/.well-known/oauth-protected-resource/mcp` shape + root variant continues to work | `mcp/src/oauth.rs` | 15m |
| 4 | Rust unit tests for tools/list count + names (replaces fragile `tools.len() == N` assertion) | `mcp/src/mcp.rs` | 15m |
| 5 | New Playwright spec `webapp/e2e/mcp-auth-tool.spec.ts` — exercises mcp_auth + WWW-Auth + protected-resource against live server | `webapp/e2e/mcp-auth-tool.spec.ts` | 45m |
| 6 | Cursor install smoke script (cliclick + AppleScript + file-system assertion on `~/.cursor/mcp.json`) | `work/cursor-vscode-e2e-tests/smoke/cursor.sh` | 45m |
| 7 | VS Code install smoke script (analogous, asserts `~/.vscode/extensions/.../mcp_servers.json` or platform-equivalent) | `work/cursor-vscode-e2e-tests/smoke/vscode.sh` | 45m |
| 8 | Claude Desktop install smoke script (no deeplink — config file edit + restart; asserts via `claude_desktop_config.json`) | `work/cursor-vscode-e2e-tests/smoke/claude-desktop.sh` | 30m |
| 9 | Manual verification runbook — 20-step checklist with screenshots | `work/cursor-vscode-e2e-tests/manual-verify.md` | 45m |
| 10 | Makefile + README that ties it together | `work/cursor-vscode-e2e-tests/smoke/Makefile`, `README.md` | 20m |

**Total: ~5h.** Tasks 1-5 are CI-blocking and ship first (next 2h). 6-10 are manual / docs and ship as time permits before May 4.

## Verification Plan

After all tasks land:

1. `cargo test --workspace` → 100% pass (including the 9 currently-broken oauth tests we should also try to fix as part of this — separate consideration).
2. `cd webapp && npx playwright test` → 100% pass against live mcp.mnemonik.xyz.
3. Operator runs `make smoke` (Tier 2) on their Mac before tagging the soft-launch release. Smoke output captured as PR artifact.
4. Operator walks through `manual-verify.md` and ticks every step.

## Risks

- **Tier 2 smoke flake from Cursor / VS Code UI changes** between releases. Mitigation: pin the tested app versions in the script header; bump on regression.
- **AX permissions prompt blocks cliclick on first run.** Mitigation: README documents the one-time grant in System Settings → Privacy → Accessibility.
- **Live-server tests rate-limited.** `oauth-flow.spec.ts` already runs serial-with-retries-disabled to stay under the 5/min/IP cap; new tests inherit this.
- **No test for the deepest issue (Cursor doesn't show Connect button)** — that's a Cursor product gap, not testable from our side. The test we CAN add is "the install deeplink URL is well-formed and the OAuth metadata is spec-compliant" — Tier 1 covers both.

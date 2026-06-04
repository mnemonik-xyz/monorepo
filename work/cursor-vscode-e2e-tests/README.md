# cursor-vscode-e2e-tests

End-to-end test coverage for Cursor / VS Code / Claude Desktop MCP integration. Three tiers by determinism + cost.

## Tier 1 — CI-runnable (every PR)

Runs on every push via existing CI:

- **Rust unit tests** — `cargo test -p mnemonic-mcp --lib --features test-support oauth::tests` covers the WWW-Authenticate header, claims-on-allowlisted, and path-specific `oauth-protected-resource/mcp`.
- **CLI integration** — `cd packages/cli && npm test` covers the `mnemonic init / login / sign / recall / verify` ladder against the SDK's in-process mock server.
- **InstallButtons unit test** — `cd webapp && npx vitest run src/components/InstallButtons.test.tsx` validates the deeplink URL formats (`cursor://`, `vscode://`, Claude.ai pasted URL).

## Tier 2 — Local macOS smoke (manual, pre-release)

Runs on the operator's Mac before tagging a release:

```bash
cd work/cursor-vscode-e2e-tests/smoke
make all     # cursor + vscode + claude-desktop
# or one at a time:
make cursor
make vscode
make claude-desktop
```

Each script:

1. Triggers the install deeplink for that app
2. Uses `cliclick` to click through the MCP install dialog
3. Asserts (file-system level — NOT AX-opaque UI scraping) that the app's MCP config file gained the Mnemonic entry

Per [m13v's note on issue #59](https://github.com/mnemonik-xyz/monorepo/issues/59): we deliberately do NOT scrape terminal output via AppleScript / OCR — that's flaky. The MCP config file IS persisted and is the deterministic source of truth.

### Prerequisites

- macOS with the apps installed:
  - Cursor at `/Applications/Cursor.app`
  - Visual Studio Code at `/Applications/Visual Studio Code.app` (Insiders also OK)
  - Claude Desktop at `/Applications/Claude.app`
- `brew install cliclick`
- macOS Accessibility permission granted to your shell host (System Settings → Privacy & Security → Accessibility → enable Terminal / iTerm / Ghostty / whatever you use). One-time grant, then `cliclick` works.
- `python3` (already on macOS).

## Tier 3 — Manual verification runbook

Walk through `manual-verify.md` step by step before any public release. ~25 minutes. Sign off in `work/<feature>/decisions.md` with a timestamp.

The runbook covers what Tier 1 + 2 can't: real OAuth flow with a real browser, real JWT lands, real tool calls succeed end-to-end.

## When to run what

| Trigger | Tier 1 | Tier 2 | Tier 3 |
|---|---|---|---|
| Every PR | ✓ | | |
| Pre-release (any `v*` tag) | ✓ | ✓ | ✓ |
| Touched `mcp/src/oauth.rs` or bearer-middleware | ✓ | ✓ | ✓ |
| Touched `InstallButtons.tsx` or any deeplink format | ✓ | ✓ | ✓ |
| Touched `tools/list` (added/removed a tool) | ✓ | ✓ | |
| Major Cursor / VS Code / Claude Desktop update | | ✓ | ✓ |

## Why no headless GUI in CI

Driving Cursor / VS Code from a CI runner requires:
- a macOS runner (~5x cost of Linux)
- AX permissions granted in a non-interactive way (impossible without re-imaging)
- the apps installed + signed in
- reproducible window state

The combinatorial cost outweighs the ROI for a hackathon-stage project. Tier 2 smoke runs on the operator's Mac.

## What we explicitly DON'T test (and why)

| Not tested | Why |
|---|---|
| Cursor's "Connect" button appearance | Cursor's UI is closed-source; behavior depends on whether the server is in their curated directory. We can't assert from outside. |
| Browser auto-pop on 401 in Cursor 3.2.16 | Cursor-side UX gap in old versions. Cursor ≥0.45 handles spec OAuth correctly; older builds require manual install via `mnemonik.xyz/install`. |
| Terminal output of `mnemonic` CLI via AppleScript | Terminals are AX-opaque. Use the `child_process.spawn`-based integration tests instead (already in `packages/cli/test/integration/cli-flows.test.ts`). |
| Cursor / VS Code / Claude Desktop versions outside the matrix | Pin tested versions in the script header. Bump on regression. |

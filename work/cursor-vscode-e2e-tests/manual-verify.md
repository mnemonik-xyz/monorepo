# Manual Verification Runbook — pre-launch sign-off

Walk through every step. Tick the checkbox. If anything fails, file a GitHub issue with the step number and the symptom.

> **When to run:** before tagging a public release (any `v*.*.*` tag), or after any change that touches OAuth (`mcp/src/oauth.rs`), the bearer middleware, the `/api/sign-callback` flow, or the install deeplinks (`webapp/src/components/InstallButtons.tsx`).

> **Tested clients (May 2026):** Cursor 3.2.16, Visual Studio Code 1.95 + GitHub Copilot Chat 0.22, Claude Desktop 0.7.x. Bump these as the team migrates.

---

## A. Server-side spec compliance (5 min)

- [ ] **A1.** `curl -i https://mcp.mnemonik.xyz/health` → `HTTP/2 200`, body `{"status":"ok"}`.
- [ ] **A2.** `curl -sS https://mcp.mnemonik.xyz/.well-known/oauth-authorization-server | python3 -m json.tool` → JSON includes `issuer`, `authorization_endpoint`, `token_endpoint`, `code_challenge_methods_supported: ["S256"]`, `scopes_supported: ["mcp"]`.
- [ ] **A3.** `curl -sS https://mcp.mnemonik.xyz/.well-known/oauth-protected-resource` → JSON has `resource: "https://mcp.mnemonik.xyz"`.
- [ ] **A4.** `curl -sS https://mcp.mnemonik.xyz/.well-known/oauth-protected-resource/mcp` → JSON has `resource: "https://mcp.mnemonik.xyz/mcp"` (path-specific, RFC 9728 §3.1).
- [ ] **A5.** `curl -i -X POST https://mcp.mnemonik.xyz/mcp -H 'content-type: application/json' -d '{"jsonrpc":"2.0","method":"tools/call","id":1,"params":{"name":"mnemonic_recall","arguments":{"query":"x"}}}'` → `HTTP/2 401` AND a `www-authenticate: Bearer realm="..."` header with `resource_metadata=...`.
- [ ] **A6.** `curl -sS -X POST https://mcp.mnemonik.xyz/mcp -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d["result"]["tools"]))'` → prints `6`.

## B. CLI install flow (3 min)

- [ ] **B1.** `npx @mnemonik-xyz/cli --version` → prints latest `0.1.x` version.
- [ ] **B2.** `MNEMONIC_CONFIG_DIR=/tmp/manual-verify-$$/ npx @mnemonik-xyz/cli init` → "identity created" message + a base58 pubkey shown.
- [ ] **B3.** `MNEMONIC_CONFIG_DIR=/tmp/manual-verify-$$/ npx @mnemonik-xyz/cli login` → opens browser to `mcp.mnemonik.xyz/oauth/authorize?...`. Approve in browser. Terminal shows "login OK" + a `sub:` line.
- [ ] **B4.** `MNEMONIC_CONFIG_DIR=/tmp/manual-verify-$$/ npx @mnemonik-xyz/cli sign "manual verify ${USER} $(date -u +%FT%TZ)"` → returns `attestation_id`, `solana_tx`, `arweave_tx`. (NOTE: if server is `STORAGE_MODE=local`, the tx fields are synthetic `local:...` — that's expected for non-anchor-mode demos.)
- [ ] **B5.** `MNEMONIC_CONFIG_DIR=/tmp/manual-verify-$$/ npx @mnemonik-xyz/cli recall "manual verify"` → returns the just-signed attestation as the top hit.
- [ ] **B6.** `rm -rf /tmp/manual-verify-$$/` (cleanup).

## C. Webapp install + OAuth + browser-mediated sign (5 min)

- [ ] **C1.** Open `https://mnemonik.xyz/install` in a fresh incognito browser. Page loads, hero shows "Install" + 4 connector cards.
- [ ] **C2.** Click "Generate new" under Agent identity. Pubkey + DID render below the buttons.
- [ ] **C3.** Click "Send to CLI". Code-block appears: `mnemonic identity import --ticket <uuid>`. Copy button works.
- [ ] **C4.** Open `https://mnemonik.xyz/sign/00000000-0000-0000-0000-000000000000` (deliberately invalid UUID). Page renders an "Invalid sign URL" / expired error — does NOT crash.
- [ ] **C5.** OAuth via the webapp's own `/oauth/consent` redirect chain: open Cursor / VS Code with the install deeplink (steps D1 / E1 below) and let the OAuth flow complete in the browser. The post-sign approval page renders with `solana_tx`, `arweave_tx`, "View on Solscan" + "View on Arweave" buttons (only with `STORAGE_MODE=full`; in local mode the buttons hide).

## D. Cursor install + OAuth + tool-call (5 min)

- [ ] **D1.** Open `https://mnemonik.xyz/install`. Click "Install in Cursor". macOS routes the `cursor://anysphere.cursor-deeplink/...` URL to Cursor.
- [ ] **D2.** Cursor's MCP install dialog appears. Confirm. Cursor adds the entry to `~/.cursor/mcp.json` (verify with `cat ~/.cursor/mcp.json`).
- [ ] **D3.** Open Cursor's MCP settings panel. Mnemonic appears as a server.
- [ ] **D4.** Trigger any Mnemonic tool from chat (e.g. ask the agent to call `mnemonic_whoami`). One of three things happens:
  - **Best:** browser pops up to `mcp.mnemonik.xyz/oauth/consent`, approve, JWT lands, tool returns server identity. ✓
  - **Documented Cursor 3.2.16 UX gap:** browser does NOT pop. Workaround: `mnemonic login` in CLI, copy JWT from `~/.mnemonic/token.json`, paste into `~/.cursor/mcp.json` as `"headers": {"Authorization": "Bearer <jwt>"}`. Restart Cursor. Retry.
  - **Failure:** anything else — file a GitHub issue with the Cursor app log (`~/Library/Logs/Cursor/`).
- [ ] **D5.** After auth, call `mnemonic_sign_memory` from Cursor chat with content "manual D5". Tool returns `awaiting_signature` + a `next_step` hint. Click the `approve_url`. Webapp signs. Agent calls `mnemonic_check_pending(correlation_id)` → returns `solana_tx` + `arweave_tx`.

## E. VS Code install + OAuth + tool-call (5 min)

- [ ] **E1.** Open `https://mnemonik.xyz/install`. Click "Install in VS Code". macOS routes the `vscode://mcp/install?...` URL to VS Code (note: with `//`, double-slash — see InstallButtons.tsx note about Safari rejecting opaque URIs).
- [ ] **E2.** VS Code prompts to install MCP server. Approve.
- [ ] **E3.** Mnemonic appears in VS Code's GitHub Copilot Chat MCP server list.
- [ ] **E4.** Trigger any Mnemonic tool from Copilot Chat. OAuth flow in browser → JWT → tool succeeds.

## F. Claude Desktop install + OAuth + tool-call (5 min)

- [ ] **F1.** Edit `~/Library/Application Support/Claude/claude_desktop_config.json` to add (preserve any existing `mcpServers`):
  ```json
  {
    "mcpServers": {
      "Mnemonic": {
        "url": "https://mcp.mnemonik.xyz/mcp",
        "type": "http"
      }
    }
  }
  ```
- [ ] **F2.** Restart Claude Desktop.
- [ ] **F3.** First Mnemonic tool call from Claude Desktop chat triggers a browser pop-up to `mcp.mnemonik.xyz/oauth/consent`. Approve. JWT lands. Tool returns server identity. (Per the 2026-05-02 user report, Claude Desktop's MCP OAuth UI works without the friction Cursor has.)

## G. Send-to-CLI keypair alignment (5 min)

> **Why this section matters:** the deferred-sign / `mnemonic_sign_memory`
> flow is governed by *two* keypairs that MUST match: the JWT.sub presented
> by the IDE on `/mcp tools/call`, and the webapp's localStorage keypair
> that signs the COSE envelope on `/api/sign-callback`. If they don't
> match, the server returns "pending bundle owner mismatch" / 403 even when
> both sides are individually valid.
>
> The Send-to-CLI flow (Decision 7 in `work/mnemonic-cli/tech-spec.md`)
> exists specifically to align them.

- [ ] **G1.** Verify mismatch reproduces. With Cursor configured against
  a JWT minted from CLI's separately-generated identity, ask the agent to
  sign a memory. Webapp opens, you click Approve, error: `Sign failed:
  pending bundle owner mismatch`. ✓ (mismatch is the bug; this confirms
  the test setup).
- [ ] **G2.** Open `https://mnemonik.xyz/install`. Note the displayed
  pubkey under "Agent identity".
- [ ] **G3.** Click **"Send to CLI"**. A code block appears:
  `mnemonic identity import --ticket <uuid>`. Copy it.
- [ ] **G4.** Run in terminal: `MNEMONIC_CONFIG_DIR=/tmp/aligned-${USER}/
  npx @mnemonik-xyz/cli identity import --ticket <uuid>` (use a fresh
  config dir to avoid clobbering existing identity). CLI prints "identity
  imported" with the same pubkey from G2.
- [ ] **G5.** `MNEMONIC_CONFIG_DIR=/tmp/aligned-${USER}/ npx
  @mnemonik-xyz/cli login` → OAuth completes; JWT.sub equals the webapp
  pubkey from G2.
- [ ] **G6.** Extract the JWT: `cat /tmp/aligned-${USER}/token.json |
  python3 -c 'import sys,json; print(json.load(sys.stdin)["jwt"])'`. Copy.
- [ ] **G7.** Edit `~/.cursor/mcp.json`. Replace the Mnemonic entry's
  `headers.Authorization` with `Bearer <jwt-from-G6>`. Save. Restart
  Cursor.
- [ ] **G8.** From Cursor chat, ask the agent to sign a memory. Webapp
  opens, click Approve. Server completes the on-chain anchor (real
  `solana_tx` if `STORAGE_MODE=full`; synthetic `local:` if `local`).
  Webapp success page shows tx + Solscan / Arweave buttons. ✓
- [ ] **G9.** Back in Cursor, ask the agent to call
  `mnemonic_check_pending` with the correlation_id from the awaiting
  response. Returns `{status: "signed", solana_tx, arweave_tx, ...}`. ✓

## H. Stage A → Stage B sign-off (1 min)

- [ ] **H1.** All A–G boxes ticked.
- [ ] **H2.** Output of `cargo test -p mnemonic-mcp --lib oauth::tests --features test-support` → all `oauth::tests::test_*` from today's batch pass (the 8 pre-existing failing oauth tests are tracked separately in a backlog item — do NOT block release on them, do NOT ignore the new ones).
- [ ] **H3.** Sign off in `decisions.md` with timestamp + your handle.

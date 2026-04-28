# Backlog — mnemonic-cli

Items deliberately out of scope for Phase 1 (≤5 dev-days, hackathon MVP). Architecture must remain open for these — no decisions in Phase 1 should foreclose any of them.

---

## TOP PRIORITY — On-chain storage + billing (Phase 1.5)

Flip `STORAGE_MODE=local` → `STORAGE_MODE=full` (Arweave + Solana anchoring) AND `PAYMENT_MODE=none` → `PAYMENT_MODE=balance` (USDC top-up). These two are inseparable economically — see [`.claude/skills/project-knowledge/references/economics.md`](../../.claude/skills/project-knowledge/references/economics.md) for full cost analysis and open economic questions.

**Why this is #1:**
- Currently all attestations live in SQLite on the VPS only. If `/home/claude/data/attestations.db` is wiped, all memories of all users are lost. There is no recovery — that's the protocol's central value gap right now.
- Without on-chain anchoring, `mnemonic verify` returns synthetic `local:` IDs. The protocol's headline pitch (verifiable, cross-node, third-party-checkable memory) is invisible until this flips.
- Without billing, flipping storage to `full` creates an unbounded cost burn — operator funds every user's writes at ~$0.003/sign.
- Both surfaces touch the CLI: new `mnemonic balance`, `mnemonic top-up`, low-balance warnings; new `verify` output showing real `arweave_tx` + `solana_tx`.

**Approximate scope:**
- Server config flip + funding: ~½ dev-day + $50 capital outlay (SOL + Irys credits).
- Async write path (write to SQLite immediately, anchor in background): ~1 dev-day.
- `PAYMENT_MODE=balance` user surface: top-up flow on webapp, balance display, refund-on-error UX, CLI commands: ~3–5 dev-days.
- Operational monitoring: SOL/Irys balance alerts, graceful degradation, treasury management policy.

**Open economic questions (need proper deliberation before flipping):**
- Pricing surface — per-call cost surfaced to user vs flat-rate tier?
- Free tier shape — first N attestations free, free if private, free for specific identity tiers?
- Refund-on-error semantics for partial failures.
- Treasury — where USDC collected goes, multisig vs single-sig, auto-swap.
- KYC threshold for high-spending identities.

**Recommended order:** spin up a separate user-spec / tech-spec for "on-chain + billing" after `mnemonic-cli` Phase 1 ships. It's not a one-day flip — the economics deserve their own deliberation.

---

## Auth & Identity

- **TurnkeySigner** — drop-in `Signer` impl that delegates to Turnkey MPC API. Same pubkey survives migration; user moves keypair into Turnkey custody without changing SDK API. Phase 1.5.
- **WebAuthnSigner** — passkey-based `Signer` (Touch ID / Yubikey / Windows Hello). User auth via biometric, key never leaves secure enclave. Phase 2.
- **OAuth refresh tokens** — currently JWT TTL is 1h, after expiry user re-runs `mnemonic login`. Refresh token endpoint + auto-refresh in SDK. Phase 1.5.
- **OAuth device-code flow** — for headless environments where `mnemonic login --token <jwt>` is too cumbersome (e.g. SSH session with no browser). Phase 2.
- **Multi-account profiles** — `~/.mnemonic/profiles/{work,personal}/identity.json`, CLI `--profile work` flag. Phase 2.

## Distribution

- **Browser bundle of SDK** — currently SDK is ESM that works in browsers, but no published browser-optimized build. Add `dist/browser.js` via esbuild for Chrome-extension consumption. Phase 1.5 (trivial).
- **Homebrew tap** — `brew install mnemonik-xyz/tap/mnemonic`. Wraps the npm package. Phase 1.5.
- **Standalone binary via `bun build --compile`** — single-file Mac/Linux/Windows binary, no Node/Bun required. Phase 2.
- **Docker image** — `ghcr.io/mnemonik-xyz/cli:latest` for CI use without npm install. Phase 2.
- **Migration to `@mnemonik` scope** — if/when org name becomes available on npm. Right now `@mnemonik-xyz` is the available scope.

## CLI UX

- **REPL / TUI mode** — `mnemonic repl` interactive shell with command history, autocomplete, multi-line input. Phase 2.
- **Agent loop** — `mnemonic chat` runs LLM-driven REPL with auto tool-calls, similar to `claude` / `codex` CLIs. Requires LLM provider integration (Anthropic API key or local Ollama). Lower priority — Cursor/Claude.ai already cover this UX. Phase 2.
- **Plugin system** — third parties register their own MCP tools accessible via `mnemonic plugins/<name>`. Phase 2.
- **Self-host commands** — `mnemonic serve` to spawn a local `mnemonic-mcp` instance, `mnemonic init-server` to set up env vars + keypair file. Phase 1.5.
- **Git-hook integration** — `mnemonic precommit-sign` auto-signs commit metadata into Mnemonic. Phase 2.
- **Auto-tagging from context** — `mnemonic sign` infers tags from current git branch, dir name, package.json. Phase 2.
- **`mnemonic export` / `mnemonic import`** — bulk export/import of attestations as JSONL for backup. Phase 1.5.
- **`mnemonic search`** — server-side full-text search over content (currently only embedding-based recall). Phase 2.
- **Keyring storage for secrets** — `~/.mnemonic/identity.json` is plaintext (mode 0600). Move to OS keyring (macOS Keychain / Linux Secret Service / Windows Credential Manager) via `keytar` or similar. Phase 1.5.

## SDK / Library

- **Bundle size optimization** — swap `@mnemonic/core` WASM (442KB) for `@noble/curves` Ed25519 + custom canonical CBOR (~30KB). Public API does not change. Validate byte-for-byte against WASM via golden test. Phase 1.5 if bundle size complaints arrive.
- **Streaming recall** — current API returns full result array; expose AsyncIterable for large result sets. Phase 2.
- **WebSocket / SSE transport** — currently HTTP request/response only; add streaming for long-running tool calls. Phase 2.
- **TypeScript declaration bundle** — single-file `.d.ts` for easier consumption in mixed-tool codebases. Phase 1.5 (trivial).
- **Browser-mediated signing path in CLI** — for cases where CLI runs without local key (e.g. future Turnkey-only environment). CLI opens browser via `open` package, polls for callback. Phase 1.5 alongside TurnkeySigner.
- **MCP client extras** — currently SDK exposes 5 Mnemonic tools. Could expose generic MCP-client primitives (`callTool`, `listTools`) for using SDK against any MCP server, not just Mnemonic. Phase 2.

## Chrome Extension (Phase 2 product, depends on SDK)

The whole reason SDK exists separately from CLI. Open product surface:
- Capture page selection / clipboard / chat transcript → `signMemory`.
- Right-click context menu integration.
- Recall popup that surfaces relevant past attestations on visited pages.
- OAuth via `chrome.identity.launchWebAuthFlow` (different code path from CLI's loopback server, but same SDK + same `Signer` interface).

## Agent framework integration (Phase 2)

- LangChain/LangGraph: publish `@mnemonik-xyz/langchain` adapter that exposes Mnemonic tools as LangChain `Tool` objects.
- AutoGen / CrewAI / PydanticAI: similar adapters in their respective ecosystems.
- These all consume the same `@mnemonik-xyz/sdk`.

## Testing

- **Cross-runtime CI matrix** — currently Phase 1 only tests Node + Bun in CI. Add Deno + Cloudflare Workers to matrix. Phase 1.5.
- **macOS UI automation tests for CLI** — drive a real terminal session through `mnemonic init / login / sign / recall` via `cliclick` or AppleScript. Phase 2.
- **Property-based fuzzing** — fuzz CBOR/COSE round-trip between SDK and server. Phase 2.

## Out of scope forever (architectural NO)

- Storing private keys in cloud / unencrypted — TurnkeySigner is the cloud path, all others local.
- Implementing custom MCP transport (anything other than streamable HTTP per MCP spec 2025).
- Bundling Anthropic API key or any other LLM credentials in CLI defaults.
- Replacing or competing with `claude` / `codex` / `qwen` CLIs as a chat client — not the goal.

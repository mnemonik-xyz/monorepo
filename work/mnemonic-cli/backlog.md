# Backlog — mnemonic-cli

Items deliberately out of scope for Phase 1 (≤5 dev-days, hackathon MVP). Architecture must remain open for these — no decisions in Phase 1 should foreclose any of them.

---

## TOP PRIORITY 2 — Crypto-flexibility (decouple from Solana / Ed25519 lock-in)

**Status (Phase 1):** Identity is hard-pinned to Solana Ed25519 because the on-chain anchor uses Solana SPL Memo (Ed25519-required by SVM) and the same keypair is reused for the off-chain attestation envelope. Webapp localStorage shape, server keypair file format, DID format `did:sol:`, all WASM exports — every layer assumes Ed25519.

### Why this matters

- **WebAuthn / passkeys** use ES256 (secp256r1) or RS256, not Ed25519 — Touch ID / Yubikey can't sign attestations today.
- **Hardware wallets** (Ledger, Trezor) are mostly secp256k1 — same problem.
- **HSMs / KMS** (AWS, GCP, Cloudflare) support various algs but not all support Ed25519.
- **Corporate / SAML identities** are typically RSA or ECDSA-P256.
- **Post-quantum migration** (ML-DSA / Falcon) — being locked to Ed25519 forecloses the upgrade path.

### Architectural decoupling needed

Today: `Identity == Anchor ID` (must be Ed25519 because anchor is Solana).

Target: **two separate signers** per attestation, recorded on the row:
- **Off-chain envelope signer** (alg-pluggable: Ed25519, secp256k1, ES256, RS256, future PQ algs). COSE_Sign1 supports this natively via the alg field.
- **Anchor signer** (chain-pluggable: Solana, Ethereum, Bitcoin, ICP, Arweave-only, none). Stays alg-bound to whatever the chain requires.

User picks the combination. Default = Solana Ed25519 for both (current behavior). Verification checks both independently.

### Cost estimate

| Scope | Days |
|---|---|
| **Option A — full multi-alg (off-chain + on-chain pluggable)** | 10–12 dev-days |
| **Option B — off-chain pluggable only (anchor stays Solana Ed25519)** | 6–8 dev-days |

Option B is the right starting point — unblocks WebAuthn / KMS users without touching the on-chain story.

Touchpoints (Option B):
- `core/src/identity/mod.rs` — generic `Signer` trait + Ed25519/secp256k1/ES256 implementations
- `core/src/codec/sign.rs` — set COSE alg field from signer; verify any registered alg
- `core/src/storage/sqlite.rs` — add `signer_alg TEXT NOT NULL DEFAULT 'EdDSA'` column (idempotent migration)
- `core/src/wasm/mod.rs` — generic sign/verify exports parameterized by alg
- `packages/sdk/src/signer.ts` — already abstract via `Signer` interface (Phase 1 prep), drop-in for new impls
- Webapp localStorage shape — version flag + new shape `{alg, secret_*, pubkey_*}`
- Migration: legacy NULL `signer_alg` rows assumed `EdDSA` (Ed25519)

Phase 1's `Signer` interface in SDK was deliberately abstract precisely to enable this. The blocker is server-side + WASM exports + storage schema + migration UX.

### Recommended sequencing

Phase 2: Option B — off-chain crypto-flex. Pulls in passkey-based identity. Anchor stays Solana for now.
Phase 3: Option A — full chain-flex. Anchor adapter pattern (Ethereum, Bitcoin, etc.). Frees the protocol from the SVM dependency.

---

## TOP PRIORITY 3 — Bundle-size optimization for SDK consumers

**Status (Phase 1, post-publish):** SDK packs to 254KB (most of it the 458KB WASM unpacked). For CLI users `npm install -g` once = fine. For programmatic SDK consumers (Chrome extension, Cloudflare Workers, LangChain agents) loading 442KB on every cold start is a real cost.

### Concrete actions

| Action | Estimated saving | Effort | Notes |
|---|---|---|---|
| `wasm-opt -O4` + `wasm-strip` | 30–50KB | 1 hour | **Done in Phase 1 pre-publish if possible** — no architectural change |
| Lean `core-wasm` Rust crate (split from `core/`, drop full `solana-sdk` dep, keep only what WASM needs) | 200–250KB | 1–2 days | Loses some shared code with server; needs careful boundary |
| Single `wasm-pack` build target (currently 2: `pkg/` + `pkg-web/`) | minor (build pipeline simplification) | 0.5 day | Doesn't reduce shipped size — code-hygiene win |
| Pure-JS swap (`@noble/curves` Ed25519 + custom canonical CBOR) — replaces WASM entirely | drops WASM 458KB → ~50KB JS | 2–3 days | Risk: drift from server's Rust CBOR encoder unless golden fixture covers exhaustively |
| Dual publish: `@mnemonik-xyz/sdk` (full WASM) + `@mnemonik-xyz/sdk-light` (pure JS, edge-friendly) | n/a | 1–2 days on top of pure-JS work | Best UX — consumer picks per-use-case |

### Bundle analyzer first

Before any of these — actually profile the WASM. `wasm-objdump -h` + `twiggy top` to identify the 250KB of `solana-sdk` transitive bloat. ~30 minutes. Otherwise we're guessing.

---

## TOP PRIORITY 1 — On-chain storage + billing (Phase 1.5)

(Already documented earlier — kept here for ordering reference; see [.claude/skills/project-knowledge/references/economics.md](../../.claude/skills/project-knowledge/references/economics.md))

Flip `STORAGE_MODE=local` → `STORAGE_MODE=full` AND `PAYMENT_MODE=none` → `PAYMENT_MODE=balance`. Inseparable economically. Without on-chain anchoring, `mnemonic verify` returns synthetic `local:` IDs — protocol's headline value invisible.

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

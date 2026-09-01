# Mnemonic Protocol

> **Verifiable, persistent memory for AI agents — signed, anchored on Solana, exposed over MCP.**

[![CI](https://github.com/mnemonik-xyz/monorepo/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/mnemonik-xyz/monorepo/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)
[![npm: cli](https://img.shields.io/npm/v/%40mnemonik-xyz%2Fcli.svg?label=%40mnemonik-xyz%2Fcli)](https://www.npmjs.com/package/@mnemonik-xyz/cli)
[![npm: sdk](https://img.shields.io/npm/v/%40mnemonik-xyz%2Fsdk.svg?label=%40mnemonik-xyz%2Fsdk)](https://www.npmjs.com/package/@mnemonik-xyz/sdk)

**Live:** [mnemonik.xyz](https://mnemonik.xyz) · **Hosted MCP:** `https://mcp.mnemonik.xyz/mcp` · **Discord:** [discord.gg/ws6wruJj](https://discord.gg/ws6wruJj)

**Docs:** [Quickstart](./docs/QUICKSTART.md) · [Tool reference](./docs/tools.md) · [Whitepaper](./docs/WHITEPAPER.md) · [How it works](./docs/how-it-works.md) · [Comparisons](./docs/comparisons.md) · [AGENTS.md](./AGENTS.md)

```bash
# Recommended: pair with the webapp (open mnemonik.xyz/install, click
# "Send to CLI", paste the ticket UUID below):
npx @mnemonik-xyz/cli init --ticket <uuid> && npx @mnemonik-xyz/cli login && npx @mnemonik-xyz/cli sign "first memory"

# Or standalone (CLI-only, no webapp pairing):
npx @mnemonik-xyz/cli init --standalone && npx @mnemonik-xyz/cli login && npx @mnemonik-xyz/cli sign "first memory"
```

Mnemonic gives an AI agent a persistent and verifiable artifact/memory layer: signed memories that can be semantically recalled, independently verified, and optionally anchored on-chain.

---

## Introduction

AI agents forget. Conversations, decisions, and learned context vanish between sessions, and when they do survive, there is no way for anyone else to verify what the agent actually remembered or claimed.

**Mnemonic Protocol** is a verifiable memory layer for AI agents. An agent writes a memory; anyone can later check *who* wrote it, *that it has not changed*, and *when it existed* — without trusting the agent, or us.

### What happens to a memory

Every memory an agent saves is:

- **Semantically embedded** so it can be recalled by meaning, not by keyword.
- **Compressed** with TurboQuant, so a portable form of the embedding travels inside the artifact itself.
- **Canonicalized** to deterministic CBOR and hashed with blake3, so the same content always produces the same fingerprint.
- **Signed** as a COSE_Sign1 artifact under an Ed25519 identity, so authorship is cryptographically provable.
- **Optionally anchored** on Arweave (durable storage) and Solana (timestamped anchor), so third parties can independently verify the memory existed at a point in time.

### You keep the key

Signing is **non-custodial**. The private key lives with the client — your CLI, your browser, your agent — and the server never holds it.

Over HTTP the server builds the canonical bundle and hands it back unsigned; your client signs it locally and posts the signature. The operator's own key signs only memories the operator itself authored. Anything else is refused outright, in code:

```
refusing to operator-sign a memory owned by a different identity;
remote writes must be client-signed via the deferred path
```

This is what makes "agent X claimed Y" checkable by a third party. If we could sign on your behalf, we could forge your memories, and the signature would prove nothing.

### You choose what goes on-chain

Every write carries an explicit intent, chosen per request — not a server-wide setting:

| | `local` | `participate` |
|---|---|---|
| Where it lives | The node's own SQLite | Arweave bytes + a Solana SPL Memo, plus SQLite |
| Cost | Free, always | Priced by the operator |
| Who can verify | You, from the signature and hash | Anyone, against public chains |

`local` is the default and is free by construction, not by configuration. A `participate` write counts as delivered only after the anchored bytes pass a recall-and-verify round-trip; if that fails the memory is demoted to `local` and **you are not charged**. Both kinds coexist in one database, and recall spans them.

### How you reach it

The protocol is exposed through the [Model Context Protocol](https://modelcontextprotocol.io/) (MCP), so any MCP-compatible client — Claude, Cursor, VS Code, Windsurf, custom agents — can use it as a drop-in memory backend over HTTP or stdio. There is also a [CLI](./packages/cli/), a [TypeScript SDK](./packages/sdk/), and a browser extension.

### Why it matters

- **Persistent memory across sessions and models.** A memory signed by one agent is readable and verifiable by any other.
- **Verifiable claims.** When an agent says "I remembered X on date Y," that claim can be checked against an on-chain anchor and a signed artifact — not just taken on faith.
- **Portable.** Artifacts are self-describing (typed schema, canonical encoding, embedded compression metadata) and can be rehydrated anywhere.
- **Offline-first.** `local` writes need no chain, no payment, and no network beyond the embedder — good for development, demos, and memories that were never meant to leave the machine.

New here? [**Quickstart**](./docs/QUICKSTART.md) gets you a signed memory in three commands; [**docs/tools.md**](./docs/tools.md) is the full tool reference.

### Repository layout

A Cargo workspace with two crates, plus the TypeScript clients and the webapp:

```
core/      # mnemonic-core — library: codec, identity, embed, compress, storage, solana, arweave, lineage
mcp/       # mnemonic-mcp  — binary: MCP server (HTTP + stdio), payment gate, pricing engine
packages/  # npm clients: cli, sdk, mcp (shim), extension
webapp/    # mnemonik.xyz — install / approve / blog surfaces
docs/      # protocol docs: quickstart, tool reference, whitepaper, specs, research
work/      # active features / bugs (spec-driven work); completed/ is archived
.claude/
└── skills/
    └── project-knowledge/   # architecture, patterns, deployment docs for AI agents
CLAUDE.md
Cargo.toml
```

The MCP server is the user-facing entrypoint. The core library is where the protocol primitives live.

---

## Foundational research

Mnemonic builds on the [Mnemonic Protocol Foundational Paper](docs/research/paper.pdf), which motivates the project's core thesis: agent memory must be semantic, attributable, and operationally cheap. Deeper protocol design notes live in [docs/research/](docs/research/).

---

## Quick start

Requires Rust stable.

```bash
# build
cargo build --release

# run the MCP server over HTTP (local storage mode, no blockchain, no payment)
STORAGE_MODE=local PAYMENT_MODE=none \
  ./target/release/mnemonic-mcp --transport http --port 3000

# or over stdio (for local MCP clients)
./target/release/mnemonic-mcp --transport stdio
```

Health check:

```bash
curl http://localhost:3000/health
```

Test MCP handshake:

```bash
curl -s http://localhost:3000/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize"}'
```

Run the test suite:

```bash
cargo test --workspace
```

Enable local ONNX embeddings (`fastembed`) when you want real semantic recall:

```bash
cargo build --release --features local-embed
```

---

## MCP tools

The server exposes **8 tools** over JSON-RPC at `POST /mcp` (and stdio):

| Tool | Purpose |
|---|---|
| `mnemonic_whoami` | Server identity (Ed25519 pubkey, DIDs, storage mode, attestation count) plus the capability envelope (`supported_modes`, `default_mode`, `participate_cost`) |
| `mnemonic_sign_memory` | Embed + compress + canonicalize (CBOR) + hash (blake3) + sign (COSE_Sign1) + persist. Takes an optional per-request `mode: "local" \| "participate"` |
| `mnemonic_check_pending` | Resolve a deferred-sign `correlation_id` to its final on-chain state |
| `mnemonic_recall` | Semantic search over stored embeddings (SQLite) |
| `mnemonic_verify` | Verify a memory by `solana_tx` and/or `arweave_tx` (version-aware) |
| `mnemonic_prove_identity` | Sign an arbitrary challenge with the server key |
| `mnemonic_publish_post` | Publish a signed public blog post (agent-native publishing) |
| `request_public_write_confirmation` | Internal ceremony gate before a public on-chain write (not user-facing) |

Three further tools — `mnemonic_attest_step`, `mnemonic_attest_verdict`, and
`mnemonic_verify_trajectory` — are **experimental** and compiled in only with
`--features trajectory-experimental` (not in `default`). Enumerate any server's
live surface with a `tools/list` call.

**→ Full reference with inputs, outputs, auth, and the write-mode howto: [docs/tools.md](./docs/tools.md).**

Current artifact format: **canonical CBOR + COSE_Sign1, blake3 hashing**. Older SHA-256/JSON artifacts are still verifiable via a legacy fallback path.

### The deferred-signing flow

The mechanics behind [You keep the key](#you-keep-the-key). A JWT write owned by
a remote user returns `{status: "awaiting_signature", correlation_id,
approve_url, ...}`; the client signs the canonical bundle locally and posts it
back to `/api/sign-callback`, and only then is anything persisted or anchored.
`mnemonic_check_pending` resolves the `correlation_id` to the final state.
Bundles expire after 300 seconds. Inline server-side signing happens only when
the writer *is* the operator (the stdio / single-tenant path). Full detail in
[docs/tools.md](./docs/tools.md#mnemonic_sign_memory).

---

## Programmatic access

Two npm packages let you drive the same hosted MCP server without writing your own JSON-RPC client. Both reuse the OAuth 2.1 + PKCE handshake and the COSE_Sign1 signing substrate that the Cursor / VS Code / Claude.ai connectors and the webapp use — only the renderer differs.

- [`@mnemonik-xyz/cli`](packages/cli/) — `mnemonic` binary for terminal use. Recommended setup: open `mnemonik.xyz/install` → click "Send to CLI" → `mnemonic init --ticket <uuid> && mnemonic login && mnemonic sign "hello"`. Standalone mode (`mnemonic init --standalone`) is also available for CLI-only use.
- [`@mnemonik-xyz/sdk`](packages/sdk/) — runtime-agnostic TypeScript SDK (`MnemonicClient`, `LocalSigner`, `Keypair`, OAuth helpers). Pure ESM; runs on Node 20+, Bun, Deno, and modern browsers.

---

## Storage modes

`STORAGE_MODE` sets what an operator *can* do and what it defaults to. It is not
the same knob as the per-request `mode` a caller picks on each write (see
[You choose what goes on-chain](#you-choose-what-goes-on-chain)) — a `local`
request stays free even on a `full` deployment.

- `local` (default) — SQLite only. No Solana / Arweave writes, no payment gate. Synthetic tx ids (`local:...`). Ideal for dev, demos, and UX testing.
- `full` — signed COSE bytes written to Arweave, anchor memo written to Solana, searchable embeddings kept in SQLite. Payment gate applies on HTTP when enabled.

`full` deployments also select an anchoring environment with
`ANCHORING_NETWORK`:

- `mainnet` (default) — uses the production Irys upload endpoint and the
  operator-selected mainnet-compatible read gateway/RPC.
- `devnet` — test-only anchoring. MCP permits only
  `SOLANA_RPC_URL=https://api.devnet.solana.com` and
  `IRYS_GATEWAY_URL=https://devnet.irys.xyz`, then selects Irys Devnet’s
  upload endpoint internally. A production or custom endpoint causes startup
  to fail rather than risk a billable upload.

Irys Devnet artifacts are disposable test data; do not use this mode for
durable user memory.

---

## Payment modes (HTTP only, `full` mode only)

`PAYMENT_MODE` ∈ `none` | `balance` | `x402` | `both`.

- `balance` — `Authorization: Bearer mnm_<key>`, balance checked against the live pricing engine quote and reserved before execution.
- `x402` — first request returns HTTP 402, retry with `X-Payment: {"tx_sig":"...","network":"solana-mainnet"}`.

Only `mnemonic_sign_memory` is paid. Deposits are validated against the treasury pubkey + USDC mint + signer ownership on the tx.

---

## Configuration

All configuration is env-driven (`mcp/src/config.rs`). The most relevant variables:

| Variable | Default | Purpose |
|---|---|---|
| `MCP_TRANSPORT` | `http` | `http` or `stdio` |
| `MCP_HTTP_HOST` / `MCP_HTTP_PORT` | `0.0.0.0` / `3000` | HTTP listener |
| `STORAGE_MODE` | `local` | `local` or `full` |
| `MNEMONIC_KEYPAIR_PATH` | `~/.mnemonic/id.json` | Server Ed25519 identity |
| `DATABASE_PATH` | `~/.mnemonic/attestations.db` | SQLite path |
| `EMBED_PROVIDER` | `fastembed` | `fastembed` \| `openai` \| `hash` (tests only) |
| `OPENAI_API_KEY` / `OPENAI_EMBED_MODEL` | — | When using OpenAI embeddings |
| `TURBO_BITS` | `4` | TurboQuant bit width (2/3/4) |
| `ANCHORING_NETWORK` | `mainnet` | `mainnet` or fail-closed test-only `devnet` |
| `SOLANA_RPC_URL` / `IRYS_GATEWAY_URL` | localhost | External anchoring endpoints (`full` mode); `ARWEAVE_URL` is a legacy fallback for the gateway only |
| `PAYMENT_MODE` | `none` | `none` \| `balance` \| `x402` \| `both` |
| `TREASURY_PUBKEY` / `USDC_MINT` | — / mainnet USDC | Payment routing |
| `SIGN_MEMORY_COST_MICRO_USDC` | `1000` | Floor price for sign-memory |
| `PRICE_REFRESH_SECS` / `PRICING_MARGIN_BPS` | `1800` / `2000` | Dynamic pricing engine |

Copy `.env.example` to `.env` to start.

---

## Development workflow

This project uses a **spec-driven** flow with AI agents:

1. **User Spec** — what and why (in `work/<feature>/user-spec.md`)
2. **Tech Spec** — how (architecture, decisions, testing)
3. **Tasks** — atomic decomposition of the tech-spec
4. **Implementation** — agent-executed, reviewed per wave

Active work lives in `work/`. Completed features are archived under `work/completed/`.

Agent guidance and project knowledge for this repo live in `.claude/skills/project-knowledge/` and `CLAUDE.md`.

Default branch: `main`. Feature branches are cut from `main` and PR'd back to it; tagged releases (`v*`) are cut from `main`.

---

## Further reading

All protocol documentation lives in this repository. Start at the top of
whichever column matches what you are here to do.

### Use it

- [`docs/QUICKSTART.md`](./docs/QUICKSTART.md) — install, identity, first signed memory in three commands
- [`docs/tools.md`](./docs/tools.md) — MCP tool reference: every tool's inputs, outputs, auth, and the `local` vs `participate` write-mode howto
- [`packages/cli/README.md`](./packages/cli/README.md) — `@mnemonik-xyz/cli`: every command, flag, and exit code
- [`packages/sdk/README.md`](./packages/sdk/README.md) — `@mnemonik-xyz/sdk`: TypeScript client, OAuth helpers, COSE verification
- [`packages/mcp/README.md`](./packages/mcp/README.md) — `@mnemonik-xyz/mcp`: one-command install into Claude Desktop, Claude Code, and Cursor
- [`packages/extension/README.md`](./packages/extension/README.md) — the MV3 browser extension

### Understand it

- [`docs/how-it-works.md`](./docs/how-it-works.md) — sign / recall / verify walked through the actual modules
- [`docs/WHITEPAPER.md`](./docs/WHITEPAPER.md) ([RU](./docs/WHITEPAPER_RU.md)) — protocol design, artifact model, trust model
- [`docs/spec/memory-composition.md`](./docs/spec/memory-composition.md) — cognitive typing, capability tokens, rehydration pipelines
- [`docs/research/`](./docs/research/) — the foundational paper and the TurboQuant analysis behind the compression choices

### Run it

- [`Dockerfile`](./Dockerfile) and [`docker-compose.yml`](./docker-compose.yml) — containerised server, optional local Ollama embedder, nginx + certbot TLS
- [`smithery.yaml`](./smithery.yaml) — Smithery MCP-registry manifest
- [`.claude/skills/project-knowledge/references/deployment.md`](./.claude/skills/project-knowledge/references/deployment.md) — CI/CD, secrets, environments, rollback

### Situate it

- [`docs/usecases.md`](./docs/usecases.md) — ten agent-memory use cases, one paragraph each (start here), with deep-dives in [`docs/usecases/`](./docs/usecases/)
- [`docs/comparisons.md`](./docs/comparisons.md) — how Mnemonic relates to adjacent memory and RAG systems
- [`docs/competitive-landscape/`](./docs/competitive-landscape/) — decentralized RAG, zkTAM, V3DB, and neighbouring directions
- [`docs/ROADMAP.md`](./docs/ROADMAP.md) — what is shipped and what is next
- [`docs/problems/`](./docs/problems/) — open questions we have not solved

### For agents and machines

- [`AGENTS.md`](./AGENTS.md) — the agent-facing service card: endpoints, tool surface, identity and verification models
- [`/.well-known/agent.json`](https://mnemonik.xyz/.well-known/agent.json) — machine-readable discovery
- [`/llms.txt`](https://mnemonik.xyz/llms.txt) — condensed summary for LLM crawlers
- [`CLAUDE.md`](./CLAUDE.md) and [`.claude/skills/project-knowledge/`](./.claude/skills/project-knowledge/) — repo conventions and architecture notes for AI coding agents

---

## Community

- **GitHub Discussions** — long-form Q&A and design proposals.
- **Discord** — [discord.gg/ws6wruJj](https://discord.gg/ws6wruJj)
- **Telegram** — [@mnemonikprotocol](https://t.me/mnemonikprotocol) · announcements: [@mnemonik_xyz_announcements](https://t.me/mnemonik_xyz_announcements)
- **Issues** — file bugs at [github.com/mnemonik-xyz/monorepo/issues](https://github.com/mnemonik-xyz/monorepo/issues). For security reports see [`SECURITY.md`](./SECURITY.md) — do **not** file public issues for vulnerabilities.

Before contributing, please read [`CONTRIBUTING.md`](./CONTRIBUTING.md) and [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md).

## License

Apache License 2.0 — see [`LICENSE`](./LICENSE) for the full text. By contributing you agree your contribution is licensed under the same terms (inbound = outbound). No CLA required.

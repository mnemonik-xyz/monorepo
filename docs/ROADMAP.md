# Roadmap

This is what's in production, what's actively landing, and what's drafted but
not started. Each entry maps to one feature folder under `work/<feature>/`,
where the full tech spec, tasks, and decisions live.

## Shipped

### docs-actualization

Promote 11 evergreen documents (9 `.md` + 2 PDF) recovered from
`sivo4kin/mnemonic-protocol@docs/usecases` into the public `docs/` tree with
surgical de-staling, expand `WHITEPAPER.md §9` to cover all 10 use cases,
anchor PK references to the new tree, and seed a follow-up roadmap in
`decisions.md`.

> Source: [`work/docs-actualization/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/docs-actualization)

### mnemonic-integrations — Phase 1 (Hackathon MVP)

Ship a public `mcp.mnemonik.xyz` endpoint that any MCP-capable AI tool
(Cursor, VS Code, Claude.ai Pro, Perplexity Pro) can install as a remote
connector. Identity AND attestation signing are both client-side: the
server never sees a private key.

> Source: [`work/completed/mnemonic-integrations/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/completed/mnemonic-integrations)

### mnemonic-webapp — MVP Protocol Chatbot

Build a React webapp backed by the existing MCP HTTP server. Adds two
endpoints to the MCP server: `POST /chat` (RAG chatbot) and
`GET /download-knowledge` (pre-built artifact). A startup seeding routine
indexes the protocol docs so the chatbot can answer questions about
Mnemonic itself.

> Source: [`work/mnemonic-webapp/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/mnemonic-webapp)

## In progress

### mnemonic-cli — Phase 1 (SDK + CLI)

Two pure-ESM npm packages under the `@mnemonik-xyz` scope: a runtime-agnostic
`@mnemonik-xyz/sdk` wrapping the public MCP HTTP surface (5 tool methods,
OAuth 2.1 + PKCE, pluggable `Signer`), and `@mnemonik-xyz/cli` — a Node-only
binary built on top of it with 7 commands plus identity bootstrap.

> Source: [`work/mnemonic-cli/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/mnemonic-cli)

### mnemonic-core — library extraction

Extract all domain logic from the monolithic MCP server into a standalone
Rust library crate `mnemonic-core`. The MCP server becomes a thin wrapper
that depends on core as a Cargo workspace member. Native-only — no WASM,
no axum, no clap in core.

> Source: [`work/mnemonic-core/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/mnemonic-core)

## Planned

### A2A Bridge

Three layers, deployable independently: new CBOR schemas in `mnemonic-core`
(`A2A_TASK_V1`, `A2A_MESSAGE_V1`, `A2A_ARTIFACT_V1`), a pure-Rust adapter
crate `mnemonic-a2a`, and surface layers (MCP tools, a reference axum
sidecar, and SDK helpers).

> Source: [`work/a2a-bridge/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/a2a-bridge)

### Cursor / VS Code / Claude Desktop — E2E test coverage

Three tiers by determinism + cost: CI-runnable Rust + TS specs on every
PR, PTY-driven CLI flows in nightly, and macOS-only AX-driven smoke tests
for the actual Cursor / VS Code / Claude Desktop install dialogs.

> Source: [`work/cursor-vscode-e2e-tests/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/cursor-vscode-e2e-tests)

### Keypair Sync — eliminate localStorage ↔ identity.json drift

Make the CLI and the webapp converge on a single Ed25519 identity per user
without the silent drift that today causes "wrong-signer" failures on
deferred-sign. Out of scope: multi-tenant keypairs and migrating
pre-existing synthetic-tx attestations to a new keypair.

> Source: [`work/keypair-sync/`](https://github.com/mnemonik-xyz/monorepo/tree/dev/work/keypair-sync)

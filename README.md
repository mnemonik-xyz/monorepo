# Mnemonic Protocol

**Signed memory that agents can keep, verify, and recover beyond one service.**

[![CI](https://github.com/mnemonik-xyz/monorepo/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/mnemonik-xyz/monorepo/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)
[![npm: cli](https://img.shields.io/npm/v/%40mnemonik-xyz%2Fcli.svg?label=cli)](https://www.npmjs.com/package/@mnemonik-xyz/cli)
[![npm: sdk](https://img.shields.io/npm/v/%40mnemonik-xyz%2Fsdk.svg?label=sdk)](https://www.npmjs.com/package/@mnemonik-xyz/sdk)

[Website](https://mnemonik.xyz) · [Install and connect](https://mnemonik.xyz/install) · [CLI](./packages/cli/README.md) · [SDK](./packages/sdk/README.md) · [Discord](https://discord.gg/ws6wruJj)

Agents change runtimes. Services change operators. A useful memory should survive both.
Mnemonic makes memory a signed artifact that a client can retain, copy, and verify independently.
The operator provides delivery and discovery services; its database is not the authority for the artifact's author or contents.

The goal is practical: save private memory, recover it in a fresh client, and continue with another operator.
Recovery requires the original bytes, the necessary keys, and independently trusted authors and checkpoints.
It restores recorded memory, not a running process, tool credentials, or unfinished actions.

## What we build around

- **Privacy at preparation.** Clients encrypt and sign sealed memories before sending them for storage. The prepared-artifact path does not send plaintext to the operator.
- **Verification without an account.** Retained signed bytes can be checked against a trusted author's key without the original operator or its SQL records.
- **A local copy you control.** Local sealed CLI writes need no hosted service. Clients can retain original artifacts and encrypted recovery backups.
- **An exit path.** Storage migration copies original signed bytes. Authenticated locator manifests describe new locations without changing authorship or signed parent references.
- **Open implementation.** Artifact formats, verification code, client libraries, and recovery tools live in this Apache-2.0 repository.

A signature proves authorship and integrity under the trusted key. It does not prove that a claim is true or provide a trusted timestamp alone.
Encryption protects content; public storage can still expose metadata. Lost decryption keys cannot be reconstructed from signatures.

## Start locally

This README describes the current source. Published packages and hosted deployments can lag behind it.
See the [implementation evidence](./work/protocol-product/implementation-evidence.md) for tested behavior and remaining acceptance work.
The [capability matrix](./docs/source-capabilities.md), [local research handoff demo](./docs/research-handoff-demo.md), and [pilot runbook](./work/protocol-product/pilot-runbook.md) describe source support, repeatable evidence, and the remaining release gates.

Build the CLI from this checkout with Node.js 20+, Rust, and `wasm-pack` installed:

```bash
npm install
npm run build --workspace @mnemonik-xyz/sdk
npm run build --workspace @mnemonik-xyz/cli

# Create a standalone identity, then save a sealed memory locally.
node packages/cli/dist/bin/mnemonic.js init --standalone
node packages/cli/dist/bin/mnemonic.js sign "Remember why we chose this design"

# Use the memory_hash printed by sign.
node packages/cli/dist/bin/mnemonic.js open <memory_hash>
```

Standalone initialization creates a file-backed identity with restricted file permissions. Existing identities are preserved unless you explicitly replace them.
The current offline CLI write requires a file-backed key; it refuses an identity stored only in the OS keychain.
The CLI saves the original signed ciphertext locally. Keep your identity and backups safe; another key cannot open an owner-only memory.
The SDK's default local cache lasts only for the client session. Applications must supply durable storage or export a backup.

For a published CLI, use `npx @mnemonik-xyz/cli` and consult its version's command help.
To use a webapp identity, open [Install](https://mnemonik.xyz/install), choose **Send to CLI**, then run `init --ticket <uuid>`.

External delivery is explicit and can require authentication and payment:

```bash
node packages/cli/dist/bin/mnemonic.js login

# Encrypt locally, retain the original, then request external delivery.
node packages/cli/dist/bin/mnemonic.js sign "Private project context" --anchor

# Explicit public plaintext publication.
node packages/cli/dist/bin/mnemonic.js sign "A public claim" --public --anchor
```

Public plaintext writes use the legacy server-prepared flow, which sends content to the operator.
Current external delivery uses Arweave/Irys. New writes do not require a Solana memo; historical memo verification and discovery remain available.

## Recovery and portability

A recovery checkpoint records the trusted scope, expected heads, and artifact locations under an independently pinned owner's signature.
An encrypted backup can carry that checkpoint and explicitly supplied key material.
Clients fetch original bytes, verify them, open permitted content, and rebuild a local index.

Recovery reports missing parents, unavailable sources, invalid artifacts, and competing branches.
Completion is relative to trusted expected heads. Discovery cannot prove that an unknown newer branch does not exist.

| Source capability | Scope and limits |
|---|---|
| Client-prepared memory | Local encryption and signing; exact-byte external delivery. Hosted private recall and the old sealed-session store are retired |
| Local recall | SDK support with a caller-supplied local embedder. General sealed recall is not yet exposed by the CLI |
| Authenticated recovery | Checkpoint verification, encrypted backups, and verified memory/A2A recovery APIs. Keys and accessible artifact bytes remain necessary |
| A2A recovery and continuation | Signed parent validation, recipient grants, and completed encrypted streams. Tested with real cryptography and mocked external services |
| A2A storage migration | Arweave fetch adapter and configured HTTP object-storage adapter; signed locator manifests preserve original bytes. Use the standalone migration/restore APIs |
| Delivery and payment | Separate delivery and settlement states, bounded retries, and same-byte resubmission. Payment acceptance alone does not establish delivery |

Local tests exercise SQL loss, storage-source shutdown, and continuation through another operator.
These tests do not establish live provider availability, index freshness, or release readiness.
The [release evidence](./work/protocol-product/release-evidence.md) records current drills and live observations; the [acceptance reconciliation](./work/sealed-memories/a2a-acceptance-reconciliation.md) tracks the remaining gates.

## Integrate or run an operator

Use the [TypeScript SDK](./packages/sdk/README.md) for client preparation, verification, local indexing, and recovery.
Use the [CLI](./packages/cli/README.md) for identity and local artifact persistence.
The [MCP launcher](./packages/mcp/README.md) connects supported agent clients; the hosted endpoint is `https://mcp.mnemonik.xyz/mcp`.

Inspect each deployment's advertised capabilities before using it.
MCP includes historical server-prepared tools whose privacy boundaries differ from the client-prepared API.
See the [tool reference](./docs/tools.md), [agent guide](./AGENTS.md), and [hosted service card](https://www.mnemonik.xyz/.well-known/agent.json). The [card source](./webapp/public/.well-known/agent.json) defines the repository version; inspect the deployed response separately.

The focused implementation guides describe current boundaries:

- [Client-prepared memory](./docs/client-prepared-memory.md): preparation, persistence, ingestion, and recall.
- [Recovery checkpoints](./docs/recovery-checkpoints.md): trust pins, backups, key recovery, and verified restore.
- [Sealed A2A](./docs/sealed-a2a.md): grants, parent validation, discovery, and continuation.
- [Storage portability](./docs/storage-portability.md): adapter capabilities, manifests, migration, and destination recovery.
- [Payment and delivery retries](./docs/artifact-payment-retries.md): durable receipts and interrupted operations.

For operator setup, see [Dockerfile](./Dockerfile), [Compose](./docker-compose.yml), and the [deployment guide](./.claude/skills/project-knowledge/references/deployment.md).
Configuration is defined in [mcp/src/config.rs](./mcp/src/config.rs).

## Develop and contribute

| Path | Purpose |
|---|---|
| [core/](./core/) | Rust artifact formats, signing, encryption, verification, storage access, and recovery |
| [mcp/](./mcp/) | HTTP and stdio service, ingestion, delivery, authentication, and payments |
| [mnemonic-a2a/](./mnemonic-a2a/) and [bridge-a2a/](./bridge-a2a/) | A2A models and integration |
| [packages/](./packages/) | TypeScript SDK, CLI, MCP launcher, and browser extension |
| [webapp/](./webapp/) | Website, installation, and approval interfaces |
| [docs/](./docs/) and [work/](./work/) | Guides, specifications, task contracts, and acceptance evidence |

After building the SDK, run the relevant checks:

```bash
npm test --workspace @mnemonik-xyz/sdk
npm test --workspace @mnemonik-xyz/cli
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support
```

See [CONTRIBUTING.md](./CONTRIBUTING.md), [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md), and [CLAUDE.md](./CLAUDE.md) for repository conventions.
Work follows written user specifications, technical designs, and atomic tasks.
The [product specification](./work/protocol-product/user-spec.md) explains the recovery promise and its boundaries.
The [whitepaper](./docs/WHITEPAPER.md) and [yellow paper](./docs/YELLOWPAPER.md) provide broader design background.

Discuss ideas on [Discord](https://discord.gg/ws6wruJj) or [Telegram](https://t.me/mnemonikprotocol).
Report bugs in [GitHub issues](https://github.com/mnemonik-xyz/monorepo/issues).
For vulnerabilities, follow [SECURITY.md](./SECURITY.md).

Apache License 2.0. See [LICENSE](./LICENSE). Contributions use the same license; no CLA is required.

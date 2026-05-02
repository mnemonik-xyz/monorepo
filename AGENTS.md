# AGENTS.md — Mnemonic Protocol

> This file describes how AI agents can discover and use the Mnemonic Protocol service. It is the human-readable companion to [`/.well-known/agent.json`](https://mnemonik.xyz/.well-known/agent.json) (machine-readable card served from the site root).

Mnemonic Protocol is itself an *agent service*: an MCP server that any AI agent can call to give itself persistent, verifiable memory. This document tells other agents (and the humans configuring them) how to plug in.

## TL;DR

```bash
# Terminal
npx @mnemonik-xyz/cli init
npx @mnemonik-xyz/cli login
npx @mnemonik-xyz/cli sign "first memory"
```

For Claude / Cursor / VS Code / Windsurf — install from [mnemonik.xyz/install](https://mnemonik.xyz/install) (one-click connector). HTTP MCP endpoint: `https://mcp.mnemonik.xyz/mcp`. OAuth 2.1 + PKCE.

## Service surface

| Surface | Endpoint | Auth | Notes |
|---|---|---|---|
| MCP (HTTP) | `https://mcp.mnemonik.xyz/mcp` | OAuth 2.1 + PKCE | Production. JSON-RPC 2.0. |
| MCP (stdio) | `npx @mnemonik-xyz/cli mcp` | local keypair | For agents that prefer stdio transport. |
| Agent card | `https://mnemonik.xyz/.well-known/agent.json` | none | Discovery. |
| OAuth metadata | `https://mcp.mnemonik.xyz/.well-known/oauth-authorization-server` | none | RFC 8414. |
| Health | `https://mcp.mnemonik.xyz/health` | none | `{"status":"ok"}` |

## Tools exposed (MCP)

| Tool | Inputs | Returns |
|---|---|---|
| `mnemonic_whoami` | — | server pubkey, DIDs, storage mode, attestation count |
| `mnemonic_sign_memory` | `{ content: string, tags?: string[] }` | `correlation_id` for the deferred-sign / sign-callback flow (SDK handles the COSE-sign step locally) |
| `mnemonic_recall` | `{ query: string, limit?: number, tags?: string[] }` | top-k semantically similar attestations |
| `mnemonic_verify` | `{ solana_tx?: string, arweave_tx?: string, attestation_id?: string }` | verification result with the recovered envelope and chain-of-trust |
| `mnemonic_prove_identity` | `{ challenge: string }` | server-signed challenge bytes |

The signing flow is intentionally split: the server returns a canonical CBOR bundle that the client signs locally with COSE_Sign1, then posts back. This means Mnemonic never holds the user's private key.

## Identity model

- **Ed25519** keypair per agent (or per user — same mechanism, different operational policy).
- DIDs:
  - `did:sol:<base58 pubkey>` — Solana-native DID resolver (default).
  - `did:key:z6Mk...` — multibase-encoded Ed25519 (interop).
- For multi-agent setups, **one keypair per agent** is the recommended pattern; lineage is preserved through CBOR `prev_id` references.

## Verification model

A consumer of a memory can independently verify it without trusting Mnemonic:

1. Fetch `arweave_tx` from any Arweave gateway → raw COSE_Sign1 bytes.
2. Recompute `blake3(canonical_cbor_payload)` → 32-byte hash.
3. Verify `cose_signature` against the writer's Ed25519 pubkey embedded in the envelope.
4. Fetch the Solana transaction by `solana_tx` (any RPC) → SPL Memo program data field equals the same hash.
5. If all four match, the memory is authentic, content-addressed, and timestamped.

Reference verifier code is in `core/src/codec/verify.rs` (Rust) and `packages/sdk/src/verify.ts` (TypeScript).

## Composability with other agent protocols

| Protocol | Status | Notes |
|---|---|---|
| **MCP** (Anthropic) | shipping | Mnemonic is itself an MCP server. Native. |
| **A2A** (Google) | bridge planned | `work/a2a-bridge/` — adapter crate `mnemonic-a2a` will turn `Task` / `Message` / `Artifact` into signed Mnemonic attestations. |
| **ERC-8004** (Ethereum) | planned | Mnemonic registration as a `signed-memory-attestation` validator in the Validation Registry. |
| **ACP** (IBM/BeeAI) | watch | Adapter once protocol stabilizes. |

## License + governance

- Apache-2.0 (see [`LICENSE`](./LICENSE)).
- No CLA. Inbound = outbound (see [`CONTRIBUTING.md`](./CONTRIBUTING.md)).
- Spec-driven workflow: every feature lives in `work/<feature>/` with `user-spec.md`, `tech-spec.md`, and atomic task files.

## Contact

- Discord: https://discord.gg/ws6wruJj
- Issues: https://github.com/mnemonik-xyz/monorepo/issues
- Security: see [`SECURITY.md`](./SECURITY.md) — `dev@mnemonik.xyz`, responsible disclosure window 90 days.

## For LLM crawlers

A simplified, machine-friendly summary of this site is at [`/llms.txt`](https://mnemonik.xyz/llms.txt). The canonical agent-discovery surface is [`/.well-known/agent.json`](https://mnemonik.xyz/.well-known/agent.json).

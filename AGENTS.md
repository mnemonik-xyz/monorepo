# AGENTS.md — Mnemonic Protocol

> This file describes how AI agents can discover and use the Mnemonic Protocol service. It is the human-readable companion to [`/.well-known/agent.json`](https://www.mnemonik.xyz/.well-known/agent.json) (machine-readable card served from the site root).

Mnemonic Protocol is itself an *agent service*: an MCP server that any AI agent can call to give itself persistent, verifiable memory. This document tells other agents (and the humans configuring them) how to plug in.

## TL;DR

```bash
# Terminal
# Pair with the webapp identity (open mnemonik.xyz/install, click Send to CLI):
npx @mnemonik-xyz/cli init --ticket <uuid>
npx @mnemonik-xyz/cli login
npx @mnemonik-xyz/cli sign "first memory"            # local sealed ciphertext; file-backed identity required
npx @mnemonik-xyz/cli sign "public claim" --public --anchor # explicit plaintext publication

# Or standalone (CLI-only): npx @mnemonik-xyz/cli init --standalone
# The CLI renews an expired session automatically (refresh token); log in one time.
```

A plain local memory stays in the agent's own store and is not encrypted.
Explicit HTTP `mode: "local"` is rejected. The SDK's `sealMemory` prepares
signed ciphertext locally; `mode: "store"` retains it in a session-only client
cache, and `mode: "anchor"` delivers original bytes to external storage.
The caller must persist returned bytes and back up keys for restart recovery.
Public anchored artifacts are plaintext at external storage.

**Keychain rule.** Plain local legacy writes need no identity key. Sealed
operations and client-signed artifacts need the identity key, including SDK
sealed local writes. Hosted recall does not accept recall keys or decrypt content.

The [client-prepared memory guide](./docs/client-prepared-memory.md) describes
source-level migration and retired hosted blob/grant routes. This document does
not establish that those changes are deployed or published to npm.

For Claude / Cursor / VS Code / Windsurf — install from [mnemonik.xyz/install](https://mnemonik.xyz/install) (one-click connector). HTTP MCP endpoint: `https://mcp.mnemonik.xyz/mcp`. OAuth 2.1 + PKCE.

## Service surface

| Surface | Endpoint | Auth | Notes |
|---|---|---|---|
| MCP (HTTP) | `https://mcp.mnemonik.xyz/mcp` | OAuth 2.1 + PKCE | Production. JSON-RPC 2.0. |
| MCP (stdio) | `npx @mnemonik-xyz/cli mcp` | local keypair | For agents that prefer stdio transport. |
| Agent card | `https://www.mnemonik.xyz/.well-known/agent.json` | none | Discovery. |
| OAuth metadata | `https://mcp.mnemonik.xyz/.well-known/oauth-authorization-server` | none | RFC 8414. |
| Health | `https://mcp.mnemonik.xyz/health` | none | `{"status":"ok"}` |

## Tools exposed (MCP)

Eleven tools are advertised by the default source build. Full reference — outputs, auth, error shapes — in [`docs/tools.md`](./docs/tools.md).

| Tool | Inputs | Returns |
|---|---|---|
| `mnemonic_whoami` | — | server pubkey, DIDs, storage mode, attestation count, and the capability envelope (`supported_modes`, `default_mode`, `anchored_cost`); on a paid (`x402`) deploy also `free_anchors`: your free daily quota of anchored writes (only for a key linked to a Google account; per account, per IP address, up to 16 KiB per write), or the `reason` you must pay |
| `mnemonic_sign_memory` | `{ content: string, tags?: string[], mode?: "local" \| "anchored" }` | over HTTP: `{ status: "awaiting_signature", correlation_id, approve_url, content_hash, expires_in }` for the deferred-sign / sign-callback flow (the SDK handles the COSE-sign step locally). Explicit HTTP `mode: "local"` is refused; plain stdio local writes return an unsigned row |
| `mnemonic_check_pending` | `{ correlation_id: string }` | `{ status: "signed", attestation_id, solana_tx, arweave_tx, ... }`, or `awaiting_signature` / `not_found` |
| `mnemonic_recall` | `{ query: string, limit?: number }` | top-k semantically similar attestations. Authenticated → your own corpus; anonymous → the cross-owner public pool only |
| `mnemonic_verify` | `{ solana_tx?: string, arweave_tx?: string }` (supply at least one) | verification result with the recovered envelope and chain-of-trust |
| `mnemonic_prove_identity` | `{ challenge: string }` | server-signed challenge bytes |
| `mnemonic_publish_post` | `{ title: string, body_markdown: string, tags?: string[], author?: string }` | the created post; requires auth |
| `mnemonic_share` | `{ memory_hash: string, reader: string }` | legacy UI handoff only; not evidence of grant delivery. New hosted general-memory grant storage is retired |
| `mnemonic_attest_a2a` | signed envelope, kind, context, optional parent locator | verified external locator and metadata receipt; validation precedes shared payment/delivery coordination |
| `mnemonic_recall_a2a` | context and optional filters | authorized external receipt metadata; clients fetch and open locally |
| `request_public_write_confirmation` | `{ content_hash: string }` | internal public-write ceremony gate (not user-facing) |

Three more — `mnemonic_attest_step`, `mnemonic_attest_verdict`, `mnemonic_verify_trajectory` — cover hash-linked agent trajectories with independent judge verdicts. They are **experimental**, compiled in only with `--features trajectory-experimental`, and are not advertised by default builds or the hosted server. Call `tools/list` to see what a given endpoint actually exposes.

Legacy `mnemonic_sign_memory` sends plaintext to the operator, which prepares
the canonical CBOR bundle for the client to sign. Keeping the signing key local
does not make artifact preparation private. Client-prepared sealed ingestion
avoids that plaintext boundary. The operator signs memory inline only for its
own stdio identity. Explicit HTTP local writes are rejected; omitted legacy
modes still follow the operator default.

## Identity model

- **Ed25519** keypair per agent (or per user — same mechanism, different operational policy).
- DIDs:
  - `did:sol:<base58 pubkey>` — Solana-native DID resolver (default).
  - `did:key:z6Mk...` — multibase-encoded Ed25519 (interop).
- For multi-agent setups, **one keypair per agent** is the recommended pattern; lineage is preserved through the CBOR `parents` field (up to 16 parent references per artifact).

## Verification model

A consumer of a memory can independently verify it without trusting Mnemonic:

1. Fetch `arweave_tx` from any Arweave gateway → raw COSE_Sign1 bytes.
2. Recompute `blake3(canonical_cbor_payload)` → 32-byte hash.
3. Verify `cose_signature` against the writer's Ed25519 pubkey embedded in the envelope.
4. For legacy artifacts with a Solana transaction, verify that its memo hash matches. New deliveries do not require a Solana memo.
5. Matching signature and hash establish authorship and content identity. A successful fetch proves availability at that observation time; it does not prove completeness or sustained availability.

Reference verifier code is in `core/src/codec/sign.rs` (`verify_artifact`) and `core/src/codec/hash.rs` (`verify_hash`).

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

A simplified, machine-friendly summary of this site is at [`/llms.txt`](https://mnemonik.xyz/llms.txt). The canonical agent-discovery surface is [`/.well-known/agent.json`](https://www.mnemonik.xyz/.well-known/agent.json).

# Mnemonik × Hermes Agent — Integration Proposal

**Date:** May 2026
**Status:** Draft v0.1
**Author:** Mnemonik team
**Audience:** Internal review · Nous Research collaboration brief

---

## 1. Executive summary

Hermes Agent (Nous Research) is a multi-platform agent runtime with a pluggable architecture: MCP tool servers, seven memory providers, an OpenAI-compatible API server, an ACP editor bridge, an RL trajectory exporter, and a plugin system. Mnemonik already ships the primitives Hermes needs at every one of those layers — a hosted MCP server, a TypeScript SDK, a `mnemonic` CLI, COSE_Sign1 + Solana/Arweave attestation, and an x402 payment gate.

This document outlines six integration surfaces, ordered by effort, and proposes a four-step rollout. The single highest-leverage move is **landing Mnemonik as Hermes' 8th Memory Provider** — it would be the only provider that delivers cryptographically verifiable memory, which directly satisfies Hermes' "persistent memory across sessions and models" goal that none of the existing seven providers can prove.

---

## 2. Context

### 2.1 What Hermes exposes

From the official integrations page:

- **MCP servers** — stdio + SSE transport, per-server tool filtering
- **Memory Providers** — pluggable; current backends: Honcho, OpenViking, Mem0, Hindsight, Holographic, RetainDB, ByteRover
- **API server** — OpenAI-compatible HTTP endpoint
- **Plugin system** — tools, lifecycle hooks, CLI commands from `~/.hermes/plugins/`
- **RL training & batch processing** — ShareGPT trajectory export, Atropos environments
- **15+ messaging gateways** — Telegram, Discord, Slack, etc.

### 2.2 What Mnemonik ships

From `mnemonik-xyz/monorepo`:

- **Hosted MCP server** at `https://mcp.mnemonik.xyz/mcp` (HTTP + stdio)
- **Five MCP tools** — `whoami`, `sign_memory`, `recall`, `verify`, `prove_identity`
- **`@mnemonik-xyz/cli`** — `mnemonic init / login / sign / …` (and reportedly `ask`)
- **`@mnemonik-xyz/sdk`** — runtime-agnostic TypeScript SDK with `MnemonicClient`, `LocalSigner`, OAuth helpers; ESM, runs on Node 20+, Bun, Deno, browsers
- **Cryptographic substrate** — canonical CBOR + blake3 + COSE_Sign1 over Ed25519 identity (DID-sol / DID-key)
- **Storage modes** — `local` (SQLite-only) and `full` (Arweave durable storage + Solana SPL Memo anchor)
- **Payment modes** — `none`, `balance`, `x402`, `both`; only `sign_memory` is paid
- **Auth** — OAuth 2.1 + PKCE, identical handshake across Cursor / VS Code / Claude.ai / webapp

---

## 3. Integration surfaces

### 3.1 MCP server registration *(effort: hours)*

Hermes' MCP layer accepts any compliant server. Adding Mnemonik to `config.yaml` makes all five tools callable from any Hermes agent immediately, with no upstream PR.

```yaml
mcp_servers:
  mnemonik:
    transport: http
    url: https://mcp.mnemonik.xyz/mcp
    auth: oauth2_pkce
    tools_allowed: [mnemonic_sign_memory, mnemonic_recall, mnemonic_verify]
```

Per-server tool filtering means agents see only the operational tools; `verify` and `prove_identity` can be reserved for ops or exposed selectively.

**Risk:** none — pure additive integration.

### 3.2 First-class Memory Provider *(effort: 1–2 weeks; PR upstream)*

This is the strategic move. Hermes' Memory Providers interface is the canonical extension point for persistence backends. Mnemonik fits the contract directly: the SDK already provides `MnemonicClient.write()` / `recall()` semantics that map to a `MemoryProvider` trait.

What Mnemonik uniquely contributes vs the existing seven:

| Property | Existing 7 | Mnemonik |
|----------|------------|----------|
| Semantic recall | ✓ | ✓ |
| Cross-runtime portability | partial | ✓ (DID-bound) |
| Cryptographic provenance | ✗ | ✓ (COSE_Sign1) |
| Tamper detection | ✗ | ✓ (blake3 over CBOR) |
| Independent timestamp | ✗ | ✓ (Solana anchor) |
| Durable off-host storage | varies | ✓ (Arweave) |

This positioning matches Hermes' own "memory survives sessions and providers" claim — Mnemonik is the only provider that lets a third party *prove* it.

### 3.3 `hermes-mnemonik` plugin *(effort: 1 week)*

Wraps the shipped CLI as native Hermes subcommands and adds lifecycle hooks:

- `hermes mnemonic sign` — re-export of `mnemonic sign`
- `hermes mnemonic ask` — re-export of `mnemonic ask` *(pending confirmation of CLI surface)*
- `on_message_complete` hook — auto-attest assistant turns when `--attest` flag is set
- `on_session_start` hook — `recall` recent project memories into the system prompt

Re-uses the existing OAuth/PKCE flow; no new credential surface for users.

### 3.4 RL trajectory attestation *(effort: 1–2 weeks; biggest collab story)*

Hermes generates ShareGPT-format trajectories for Atropos / RL fine-tuning. Piping each turn through `mnemonic_sign_memory` produces training data where every sample carries:

- A Solana tx ID (provable timestamp)
- An Arweave tx ID (durable artifact)
- A producing-agent DID (authorship)
- A blake3 hash over canonical CBOR (integrity)

This is a unique pitch: **verifiable RL datasets**. Aligns with Nous's open-training ethos and gives downstream consumers a way to audit dataset provenance — something neither HuggingFace datasets nor existing trajectory exporters provide.

Suggested artifact: a joint blog post + reference dataset (signed Hermes trajectories on a public benchmark).

### 3.5 API-server middleware *(effort: 2–3 days)*

Hermes exposes an OpenAI-compatible HTTP endpoint. A small middleware:

- Honors `X-Mnemonic-Attest: true` on incoming requests
- Calls `sign_memory` on the assistant response
- Returns `X-Mnemonic-Solana-Tx` and `X-Mnemonic-Arweave-Tx` headers

Every downstream client (Open WebUI, LibreChat, NextChat, ChatBox) inherits attestation transparently with no client changes.

### 3.6 x402 as agent-billing primitive *(effort: TBD; longer-horizon)*

Mnemonik already implements x402 + USDC settlement. Hermes has no comparable primitive for paid tool calls. Separate proposal: Mnemonik as the reference x402 gateway for paid Hermes tools — agents paying agents in stablecoins, with payment-gated tool execution and per-request settlement on Solana.

---

## 4. Recommended sequencing

| Phase | Effort | Output |
|-------|--------|--------|
| **1. MCP registration** | days | Mnemonik usable from any Hermes agent today; no PR required |
| **2. RL attestation demo** | 1–2 wks | Joint blog post + signed-trajectory dataset with Nous; the headline collab |
| **3. Memory Provider PR** | 1–2 wks | Upstream PR to Hermes; Mnemonik becomes the 8th first-class provider |
| **4. Plugin + middleware** | 1 wk | Bundle CLI ergonomics + API-server attestation for end users |

Phase 6 (x402 billing) is held separate as a longer-horizon protocol conversation.

---

## 5. Risks & open questions

- **CLI surface.** README documents `init / login / sign`; `ask` is referenced but not yet visible in public docs. Plugin scope (3.3) depends on confirming `ask` is the recall frontend.
- **Memory Provider PR acceptance.** Nous may prefer a curated provider list. Mitigation: ship the plugin (3.3) first as a community provider; PR upstream once adoption is demonstrated.
- **Payment gate UX.** `sign_memory` in `full` mode is paid. For Hermes' RL trajectory attestation (3.4), this is a per-turn cost — needs either bulk pricing or a `local`-mode default with opt-in `full` for high-value runs.
- **OAuth alignment.** Hermes' credential pool system and Mnemonik's OAuth 2.1+PKCE need a confirmed integration path; SDK helpers should make this routine but worth a spike.
- **Solana mainnet dependency.** Hermes runs in air-gapped contexts (some enterprise deployments). `local` mode addresses this but loses the verifiability story; document the trade-off clearly.

---

## 6. Next actions

1. Confirm `mnemonic ask` CLI surface and document in this report
2. Open a draft issue on `NousResearch/hermes-agent` proposing MCP registration in their docs
3. Draft RL trajectory attestation demo as a 1-page pitch for Nous
4. Spike a `hermes-mnemonik` plugin against the SDK to validate the lifecycle-hook surface

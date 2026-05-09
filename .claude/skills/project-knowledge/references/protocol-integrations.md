# Protocol Integrations

Status of Mnemonic's integration with external multi-agent protocols. Source of truth for which bindings exist, which are planned, and which are deliberately deferred.

---

## Positioning

The protocol-integration roadmap exists to deliver one positioning, locked in 2026-05-01:

> **Mnemonic is verifiable memory for trustless agents.**

The trustless-agent stack as it exists in May 2026 (A2A v1.0.0-rc + ERC-8004 mainnet + TEE / crypto-economic validators) is missing the layer that proves what an agent claims to remember. Mnemonic fills that gap. Full rationale, gap analysis, three-regime decision matrix, and what the positioning forecloses live in [`work/a2a-bridge/research/positioning-trustless-agents.md`](../../../../work/a2a-bridge/research/positioning-trustless-agents.md). That document is the strategic charter for everything in the table below.

---

## TL;DR

Today the only wire surface is Mnemonic's own MCP tools (5 of them). No protocol bindings ship yet. The active backlog item is the **A2A bridge** (`work/a2a-bridge/`); the next is the **ERC-8004 follow-on** detailed in `work/a2a-bridge/backlog.md`. Both are required to land the positioning above; they are sequenced together because they share substrate.

---

## Snapshot

| Protocol | Status | Folder | Schema family |
|---|---|---|---|
| **A2A (Agent2Agent)** | Backlog → V1 in flight | `work/a2a-bridge/` | `A2A_TASK_V1`, `A2A_MESSAGE_V1`, `A2A_ARTIFACT_V1` |
| **ERC-8004 (Trustless Agents, Ethereum mainnet 2026-01-29)** | Backlog — V1 plan locked, anchor pluggability co-required | `work/a2a-bridge/backlog.md` (Phase 2 of the bridge stack) | reuses A2A schemas + new `MNEMONIC_FEEDBACK_V1` for Reputation Registry payloads |
| **MCP-to-MCP delegation** | Backlog | `work/a2a-bridge/backlog.md` | `MCP_DELEGATION_V1` (planned) |
| **ACP (IBM/BeeAI)** | Backlog | `work/a2a-bridge/backlog.md` | `ACP_RUN_V1`, `ACP_MESSAGE_V1`, `ACP_AWAIT_V1` (planned) |
| **AGNTCY (Cisco)** | Watch & wait | `work/a2a-bridge/backlog.md` | TBD post-AGNTCY-v1 |
| **Hermes Agent runtime (Nous Research)** | Backlog — proposal drafted, near-term reference deployment of the trustless-agent stack positioning | `.claude/skills/project-knowledge/recovered/research/mnemonik-hermes-integration.md` | reuses `MEMORY_V1`; no new schemas required |
| **Hindsight memory architecture (Latimer et al., arXiv:2512.12818)** | Backlog — analysis + cost model drafted; adapter design includes Merkle-batch anchoring | `.claude/skills/project-knowledge/recovered/research/hindsight-mnemonik-analysis.md` | reuses `MEMORY_V1` per Hindsight network (W/B/O/S) |
| **LangGraph / AutoGen / CrewAI** | Frameworks, not protocols | `work/a2a-bridge/backlog.md` | reuse A2A / ACP / MCP-delegation envelopes |

---

## A2A bridge (active)

A2A is Google's Agent2Agent protocol — JSON-RPC + SSE over HTTP, currently v1.0.0-rc, with `Task` / `Message` / `Part` / `Artifact` / `AgentCard` as core types and AgentCard JWS signing for producer identity. The protocol explicitly defers memory ("agents collaborate without needing access to each other's internal state, memory, or tools"), which is exactly the gap Mnemonic fills.

**Why it matters for Mnemonic.** Six of eleven `docs/usecases/*.md` are A2A-shaped (`task-memory-ledger`, `shared-memory-layer`, `shared-project-memory-namespace`, `artifact-attestation-service`, `provenance-attestation-layer`, `reliability-oracle-for-orchestration`). Without a bridge, each integrator hand-rolls the mapping from A2A wire types to `MEMORY_V1` and maintains it across A2A versions. A first-class bridge converts the whitepaper's positioning sentence ("A2A makes agents interoperable in motion; Mnemonic makes them coherent over time") from prose into a working integration surface, and creates a defensible double moat — signed memory + native multi-agent protocol binding — that no current memory competitor (letta / zep / mem0 / cognee) holds.

**Surface (planned).** Three layers:
- Schemas in `mnemonic-core` (`core/src/codec/a2a/`).
- Adapter crate `mnemonic-a2a/` (pure-Rust functions over `mnemonic-core`).
- Three deployment shapes: MCP tools (`mnemonic_attest_a2a`, `mnemonic_recall_a2a`), reference sidecar (`bridge-a2a/`), SDK helpers in `@mnemonik-xyz/sdk`.

**Identity binding.** AgentCard `x-mnemonic` extension publishes the agent's Ed25519 pubkey; A2A's existing JWS over the AgentCard authenticates the binding. No DID, no new identity crypto.

**Conformance.** Golden vectors published as `@mnemonik-xyz/conformance` so any third-party implementation can prove byte-for-byte parity. Ties into the missing `references/conformance.md` doc that this work also creates.

See `work/a2a-bridge/{user-spec.md, tech-spec.md, tasks/, decisions.md, backlog.md}` for the full plan and 8-task wave breakdown.

---

## MCP-to-MCP delegation (planned)

When MCP server A delegates a tool call to MCP server B on the user's behalf, today there is no signed record of the chain. This is structurally identical to A2A task delegation but on the MCP transport.

**Schema sketch.** `MCP_DELEGATION_V1 = {caller_pubkey, callee_pubkey, tool_name, request_hash, response_hash, started_at, completed_at, status}`. Adapter pattern reuses `mnemonic-a2a` 70/30 (most of the lineage / canonicalization plumbing transfers).

**Why it matters.** Closes the audit gap for "agent toolchains where one MCP server forwards calls to another" — currently only the outermost server's logs exist. Mnemonic itself becomes a first-class delegation target ("agent X called `mnemonic_sign_memory` Y times this session"). Differentiates against generic MCP registries (smithery etc.): they list servers, we attest interactions between them.

---

## ACP — Agent Communication Protocol (planned)

IBM / Linux Foundation / BeeAI's protocol — REST-based (not JSON-RPC), messages-as-first-class-objects, async-by-default with explicit `Run.await` semantics for human-in-the-loop. Different surface from A2A, same underlying memory gap.

**Schema sketch.** `ACP_RUN_V1` (matches ACP's `Run` lifecycle: `created → in-progress → awaiting → completed/failed/cancelled`), `ACP_MESSAGE_V1`, `ACP_AWAIT_V1`. The await semantics are particularly attestable — these are the long-running runs where signed audit trail matters most for compliance.

**Why two protocol bindings.** A2A + ACP together makes "vendor-neutral attestation layer" a real claim instead of an aspirational one. They cover the two leading open multi-agent ecosystems (Google-led and IBM-led). Subsequent bindings cost less because the adapter pattern is established.

---

## ERC-8004 — Trustless Agents (Ethereum mainnet, live since 2026-01-29)

ERC-8004 is the on-chain trust layer designed to extend A2A. Three registries on Ethereum mainnet:

- **Identity Registry** — ERC-721 NFT per agent. `agentId = tokenId`. `tokenURI` resolves to off-chain JSON registration file with `services[]`, `supportedTrust[]`, cross-chain `registrations[]`.
- **Reputation Registry** — `giveFeedback` / `appendResponse` / `revokeFeedback` / `readAllFeedback` / `getSummary`; on-chain commits a hash, off-chain JSON carries the rich payload.
- **Validation Registry** — `validationRequest` / `validationResponse` with `responseURI` + `responseHash` for off-chain attestations. Spec note: still under active update with the TEE community.

**Why Mnemonic plugs in cleanly.** The Validation Registry is *literally designed* for off-chain signed attestations like Mnemonic's COSE_Sign1-over-deterministic-CBOR. Two existing validator categories are filling fast (TEE — Phala / Marlin; crypto-economic — staking-based). Mnemonic occupies a third, distinct trust category — **signed-memory** — that no current ERC-8004 participant holds. First-mover window is months, not years.

**Four integration paths** (full detail in `work/a2a-bridge/backlog.md` § "ERC-8004 — Phase 2 of the bridge stack"):

1. Mnemonic as a registered validator on the Validation Registry (validator-as-a-service).
2. Mnemonic declared in agents' own registration files (minimal binding).
3. Mnemonic-attested entries in the Reputation Registry (long-lived signing identity behind every feedback).
4. Three-way identity reconciliation via `did:mnemonic:` — closes the chain `tokenId → registration file → AgentCard URL → x-mnemonic.ed25519_pubkey`.

**Hard prerequisite — Solana decoupling.** Path-b ("ship ERC-8004 while keeping Solana SPL Memo as the only anchor") is rejected because it deepens the SVM dependency the protocol is trying to escape (`work/mnemonic-cli/backlog.md` Phase 3). The chain-pluggable anchor work — narrowed to "Phase 3α", anchor-only, off-chain envelope alg unchanged — must land *during or before* erc8004-1. This is treated as a sequencing constraint, not a "maybe", and is recorded in `work/a2a-bridge/decisions.md`.

**Scope.** ~20 dev-days across six tasks (`erc8004-0` anchor pluggability through `erc8004-5` Ethereum anchor end-to-end), riding entirely on the A2A bridge substrate.

---

## AGNTCY (watch & wait)

Cisco / Outshift's Agent Connect initiative — broader than a single wire protocol; standardizes agent identity, discovery, and message-passing with their own AgentCard-like spec (`agp`). Less mature than A2A or ACP. Their identity layer is closer to DIDs than A2A's JWS model — the integration that would force `did:mnemonic:` design is this one.

Re-evaluate when AGNTCY tags v1. Premature today: schema would churn.

---

## Hermes Agent runtime — Nous Research (concrete deployment of the positioning)

Not a protocol. A multi-platform agent runtime with a pluggable architecture: MCP tool servers, seven memory providers (Honcho, OpenViking, Mem0, Hindsight, Holographic, RetainDB, ByteRover), an OpenAI-compatible API server, an ACP editor bridge, RL trajectory exporter, and plugin system. Hermes is the **near-term reference deployment** of the "verifiable memory for trustless agents" positioning — a specific runtime where Mnemonik can land as the only cryptographically verifiable memory provider in a registry that already takes new entries.

Six integration surfaces, ordered by effort, in the proposal:

1. **MCP server registration** (hours) — pure additive `config.yaml` line.
2. **First-class Memory Provider** (1–2 wks, upstream PR) — the strategic move; Mnemonik becomes the 8th provider, the only one with COSE_Sign1 + Solana anchor + Arweave durability.
3. **`hermes-mnemonik` plugin** (1 wk) — CLI re-exports + lifecycle hooks (`on_message_complete` auto-attest, `on_session_start` recall).
4. **RL trajectory attestation** (1–2 wks) — ShareGPT export with Solana tx + Arweave tx + producing-agent DID per turn; the headline collab — "verifiable RL datasets" as a dataset-provenance pitch for Nous's open-training ethos.
5. **API-server middleware** (2–3 days) — `X-Mnemonic-Attest` header on the OpenAI-compatible endpoint; every downstream client (Open WebUI, LibreChat, NextChat, ChatBox) inherits attestation transparently.
6. **x402 as Hermes' agent-billing primitive** (longer horizon) — separate proposal; agents paying agents in stablecoins with payment-gated tool execution.

Recommended sequencing in the proposal: MCP registration → RL attestation demo with Nous → upstream Memory Provider PR → plugin + middleware bundle.

Full proposal: [`recovered/research/mnemonik-hermes-integration.md`](../recovered/research/mnemonik-hermes-integration.md).

---

## Hindsight × Mnemonik — composition with a cognitive memory architecture

Not a protocol or runtime. **Hindsight** (Latimer et al., *arXiv:2512.12818*, Dec 2025; Vectorize.io + Virginia Tech) is a memory architecture that treats agent memory as a first-class reasoning substrate rather than a retrieval layer. Four logical networks — World (W), Experience (B), Opinion (O), Observation (S) — and three operations (Retain / Recall / Reflect) via TEMPR + CARA. Already a first-class Hermes memory provider; benchmark numbers on LongMemEval (39 → 83.6 % with a 20B backbone) and LoCoMo (89.6 %).

The integration thesis is **composition, not competition**: Hindsight is the cognitive layer (how memory is organized and reasoned over); Mnemonik is the trust layer (how memory becomes verifiable across instances and time). The four Hindsight networks map cleanly onto Mnemonik's existing schema registry; TEMPR's narrative-fact extraction is the natural insertion point for `mnemonic_sign_memory`; CARA's reflect output co-signs alongside the retrieved memory set.

Six contradictions are flagged and reconciled in the analysis: mutability vs. immutability (treat reinforcement as append-only deltas), async observation regeneration (versioned attestations rather than overwrites), missing agent identity in Hindsight (extend bank profile with DID + pubkey), latency (sync sign, async anchor), unverified LLM extraction (Mnemonik attests *that* extraction happened, not that facts are true), and closed evaluation (opening for Mnemonik to define a provenance benchmark).

Cost model: per-attestation ~ $0.0003–$0.0005 today; naive "sign everything" integration of a Hindsight pipeline lands around $5,000–$7,500/mo per 1,000 heavy users; five mitigations (local-mode default, **Merkle batching ~1000× reduction**, selective per-network policy, sync-sign-async-anchor, x402 cost passthrough) drop the bill ~10×.

Full analysis + cost model: [`recovered/research/hindsight-mnemonik-analysis.md`](../recovered/research/hindsight-mnemonik-analysis.md).

---

## Frameworks vs. protocols

LangGraph, AutoGen, CrewAI, Agno, etc. are **frameworks**, not protocols. Do **not** define new core schemas per framework. Instead provide framework-specific adapters in `@mnemonik-xyz/sdk` that translate framework events into one of the protocol envelopes (A2A / ACP / MCP-delegation) and route through the appropriate bridge.

This preserves the one-way dependency graph in `core/`, keeps the schema count bounded, and lets each framework pick whichever protocol binding most naturally fits its call shape.

---

## Anchor-layer integrations

These do not change the off-chain envelope and so do not belong in this doc beyond pointer:

- **Solana** — current default anchor (SPL Memo). See `core/src/solana/`.
- **Arweave** — current default durable storage. See `core/src/arweave/`.
- **Chain-pluggable anchor** (Ethereum / Bitcoin / ICP / Arweave-only / none) — Phase 3 of `mnemonic-cli`. The narrowed subset "Phase 3α" (anchor-only, off-chain envelope alg unchanged) is upgraded to **prerequisite or co-requisite of ERC-8004 V1** — see `work/a2a-bridge/backlog.md` and the cross-link in `work/mnemonic-cli/backlog.md` "TOP PRIORITY 2".
- **ERC-8004 on-chain agent identity** — see the dedicated section above. No longer foreclosed.

---

## When to update this doc

Add a row to the snapshot table whenever:
- A new protocol-binding folder appears under `work/`.
- A backlog binding gets a tech-spec.
- A binding ships V1 (status flips to "shipped").
- A protocol enters or leaves "watch & wait" — usually because the protocol itself tagged a stable release.

Source of truth for the *active* binding is its `work/<name>/` folder; this doc is the index.

# Protocol Integrations

Status of Mnemonic's integration with external multi-agent protocols. Source of truth for which bindings exist, which are planned, and which are deliberately deferred.

---

## TL;DR

Mnemonic's whitepaper positions the protocol underneath multi-agent coordination layers as durable, signed memory. Today the only wire surface is Mnemonic's own MCP tools (5 of them). No protocol bindings ship yet. The active backlog item is the **A2A bridge** — see `work/a2a-bridge/`.

---

## Snapshot

| Protocol | Status | Folder | Schema family |
|---|---|---|---|
| **A2A (Agent2Agent)** | Backlog → V1 in flight | `work/a2a-bridge/` | `A2A_TASK_V1`, `A2A_MESSAGE_V1`, `A2A_ARTIFACT_V1` |
| **MCP-to-MCP delegation** | Backlog | `work/a2a-bridge/backlog.md` | `MCP_DELEGATION_V1` (planned) |
| **ACP (IBM/BeeAI)** | Backlog | `work/a2a-bridge/backlog.md` | `ACP_RUN_V1`, `ACP_MESSAGE_V1`, `ACP_AWAIT_V1` (planned) |
| **AGNTCY (Cisco)** | Watch & wait | `work/a2a-bridge/backlog.md` | TBD post-AGNTCY-v1 |
| **ERC-8004 / on-chain identity** | Foreclosed pre-Phase-3 | `work/mnemonic-cli/backlog.md` | n/a (anchor-layer) |
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

## AGNTCY (watch & wait)

Cisco / Outshift's Agent Connect initiative — broader than a single wire protocol; standardizes agent identity, discovery, and message-passing with their own AgentCard-like spec (`agp`). Less mature than A2A or ACP. Their identity layer is closer to DIDs than A2A's JWS model — the integration that would force `did:mnemonic:` design is this one.

Re-evaluate when AGNTCY tags v1. Premature today: schema would churn.

---

## Frameworks vs. protocols

LangGraph, AutoGen, CrewAI, Agno, etc. are **frameworks**, not protocols. Do **not** define new core schemas per framework. Instead provide framework-specific adapters in `@mnemonik-xyz/sdk` that translate framework events into one of the protocol envelopes (A2A / ACP / MCP-delegation) and route through the appropriate bridge.

This preserves the one-way dependency graph in `core/`, keeps the schema count bounded, and lets each framework pick whichever protocol binding most naturally fits its call shape.

---

## Anchor-layer integrations

These do not change the off-chain envelope and so do not belong in this doc beyond pointer:

- **Solana** — current default anchor (SPL Memo). See `core/src/solana/`.
- **Arweave** — current default durable storage. See `core/src/arweave/`.
- **Chain-pluggable anchor** (Ethereum / Bitcoin / ICP / Arweave-only / none) — Phase 3 of `mnemonic-cli`. See `work/mnemonic-cli/backlog.md`.
- **ERC-8004 on-chain agent identity** — foreclosed until chain-pluggable anchor lands. Re-evaluate post-Phase-3.

---

## When to update this doc

Add a row to the snapshot table whenever:
- A new protocol-binding folder appears under `work/`.
- A backlog binding gets a tech-spec.
- A binding ships V1 (status flips to "shipped").
- A protocol enters or leaves "watch & wait" — usually because the protocol itself tagged a stable release.

Source of truth for the *active* binding is its `work/<name>/` folder; this doc is the index.

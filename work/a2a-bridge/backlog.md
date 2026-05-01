# Backlog — a2a-bridge and adjacent protocol integrations

Items deliberately out of A2A-V1 scope. Architecture must remain open for them — no V1 decision should foreclose any.

---

## A2A — V2 / future scope

- **Streaming chunk attestation (`A2A_STREAM_CHUNK_V1`)** — per-SSE-chunk attestations for `SendStreamingMessage` / `SubscribeToTask`. V1 only fixes the final task state. Cost: high write-amplification; needs batched / Merkle-root strategy.
- **Push-notification config attestation** — A2A's `CreateTaskPushNotificationConfig` registers webhooks; attesting these closes a side-channel where a malicious agent could exfiltrate task results.
- **Cross-A2A-server lineage** — V1's `prev_id` lineage is per-Mnemonic-store. Cross-server lineage needs a portable handle (e.g. `did:mnemonic:<pubkey>/<attestation_id>`). Belongs together with the Phase 3 chain-pluggable anchor (see `work/mnemonic-cli/backlog.md`).
- **AgentCard discovery indexing** — store + recall AgentCards themselves (with their JWS signatures) so a Mnemonic store becomes a queryable agent registry.

---

## MCP-to-MCP delegation

When an MCP server (A) delegates a tool call to another MCP server (B) on behalf of a user, today there is no signed record of "A asked B to run tool T with args X, B returned Y". This is the same shape as A2A's task delegation but on the MCP transport.

**What a binding would do:**

- Define `MCP_DELEGATION_V1` schema: `{caller_pubkey, callee_pubkey, tool_name, request_hash, response_hash, started_at, completed_at, status}`.
- Reuse the `mnemonic-a2a` adapter pattern: `attest_delegation(...)`, `recall_delegations(caller, callee?, tool?)`.
- Sidecar deployment is straightforward — MCP transport is JSON-RPC over HTTP/stdio, same shape as A2A.

**Value:**

- Closes the audit gap for "agent toolchains where an MCP server forwards calls to another MCP server" — currently the user only sees the outermost server's logs.
- Mnemonic itself becomes a first-class delegation target (you can prove "agent X called mnemonic_sign_memory Y times in this session").
- Differentiates Mnemonic against generic MCP-server registries (smithery, etc.) — they list servers, we attest interactions between them.

**Cost:** ~70% reuse of A2A adapter; ~30% new schema + sidecar variant. Single mid-sized feature folder.

---

## ACP (Agent Communication Protocol — IBM/BeeAI)

ACP is IBM/Linux Foundation's agent protocol — REST-based (not JSON-RPC), messages-as-first-class-objects, async-by-default, framework-agnostic. Different surface, same problem.

**What a binding would do:**

- `ACP_RUN_V1`, `ACP_MESSAGE_V1`, `ACP_AWAIT_V1` schemas — distinct from A2A because ACP's `Run` lifecycle (`created → in-progress → awaiting → completed/failed/cancelled`) and message-array shape do not 1:1 map to A2A Task/Message.
- Adapter: `mnemonic-acp` crate, same one-way dependency rule.
- Sidecar variant for ACP servers running on `acp-sdk`.

**Value:**

- Captures the IBM / Watsonx / open-source-research-pipeline ecosystem that ACP targets, distinct from A2A's Google-led one.
- Two protocol bindings (A2A + ACP) makes "vendor-neutral attestation layer" a real claim, not an aspirational one.
- ACP's `Run.await` semantics (long-running, human-in-the-loop) make signed audit-trail genuinely valuable — these are the runs that matter most for compliance.

**Cost:** Same shape as A2A bridge; estimate ~80% of A2A effort because the adapter pattern is now established.

---

## AGNTCY (Agent Connect Protocol)

AGNTCY is Cisco / Outshift's open-source initiative — broader than a single wire protocol; aims to standardize agent identity, discovery, and message-passing, with their own `AgentCard`-like spec (`agp`). Less mature than A2A or ACP but well-funded.

**What a binding would do:**

- Watch and wait until protocol surface stabilizes. Adapter would mirror A2A: identity binding via AGNTCY's own descriptor, message/task schemas as `AGNTCY_*_V1`.
- AGNTCY identity layer is closer to DIDs than A2A's JWS model; this is the integration that would force `did:mnemonic:` design (a Phase 3 question, not Phase 1).

**Cost:** Premature today — schema would churn. Re-evaluate when AGNTCY tags v1.

---

## LangGraph / AutoGen / CrewAI direct integrations

Not protocols — frameworks. Each has its own message and run abstractions.

**Approach:** Do NOT define new core schemas per framework. Instead provide framework-specific *adapters in `@mnemonik-xyz/sdk`* that translate framework events → A2A/ACP/MCP_DELEGATION envelope and route through the appropriate bridge. Frameworks are users, not protocols.

**Why:** preserves the one-way dependency graph in `core/`; keeps schema count bounded; lets us pick whichever protocol binding (A2A vs ACP vs MCP-delegation) most naturally fits each framework's call shape.

---

## ERC-8004 / on-chain agent identity

Ethereum L2-native agent identity registries (ERC-8004 draft, plus AGNTCY-on-chain experiments). When + if they stabilize, Mnemonic attestations could register the producer pubkey there directly, giving a single verifiable identity from a Web3 audience without changing the off-chain envelope.

**Cost:** Trivial once the chain-pluggable anchor (Phase 3 of `mnemonic-cli`) lands. Pre-Phase-3: foreclosed.

---

## Cross-cutting: shared `mnemonic-protocols/` crate

If we build A2A + MCP-delegation + ACP bindings, the `attest_*` / `recall_by_context` skeleton repeats. Refactor target: extract the common adapter pattern into `mnemonic-protocols/` with per-protocol modules. Premature for V1 (one binding); revisit at V2.

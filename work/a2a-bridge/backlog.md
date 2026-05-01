# Backlog — a2a-bridge and adjacent protocol integrations

Items deliberately out of A2A-V1 scope. Architecture must remain open for them — no V1 decision should foreclose any.

---

## Positioning (locked-in 2026-05-01)

This entire body of work — A2A bridge V1, ERC-8004 follow-on, MCP-to-MCP delegation, ACP, AGNTCY, framework adapters — exists to deliver a single positioning:

> **Mnemonic is verifiable memory for trustless agents.**

Not "memory for AI agents" (head-to-head with letta / zep / mem0 / cognee on retrieval quality, where we lose). Not "agent identity" (head-to-head with ERC-8004 / DIDs, where we compose rather than compete). Not "execution attestation" (TEE territory). The defensible niche is **cryptographic provenance over content the agent itself claims to remember**, composed underneath the trustless-agent stack (A2A + ERC-8004).

Full gap analysis, what we explicitly do not close, three-regime decision rationale, and what the positioning forecloses: see [`research/positioning-trustless-agents.md`](research/positioning-trustless-agents.md). Anyone touching this backlog should read that document first — every sequencing decision below derives from it.

Decision log entry: `decisions.md` → "2026-05-01 — Positioning lock-in: verifiable memory for trustless agents".

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

## ERC-8004 — Phase 2 of the bridge stack (post A2A V1)

**Status as of 2026-05-01:** ERC-8004 ("Trustless Agents") shipped on Ethereum mainnet on **2026-01-29**. It is *literally designed* to extend A2A with a trust layer via three on-chain registries (Identity / Reputation / Validation), each pointing at off-chain content-addressed documents. Mnemonic's signed-CBOR-with-Ed25519 envelope is one of the cleanest off-chain attestation shapes that fits the spec's "responseURI + responseHash" pattern. First-mover window is closing — TEE-attestation validators (Phala, Marlin) are already moving in. Mnemonic occupies a third, distinct trust category (signed-memory) that no current ERC-8004 participant holds.

**Hard prerequisite — break the Solana lock during or before this stage.** Path-b ("validator-as-a-service that still anchors via Solana memo and only *posts* to ERC-8004 off-chain") is explicitly REJECTED. Reason: it ships ERC-8004 integration while *deepening* the SVM dependency the protocol is trying to escape (Phase 3 of `work/mnemonic-cli/backlog.md`). We do not double-anchor. The chain-pluggable anchor work either (a) lands first and ERC-8004 inherits Ethereum as one of the supported anchors, or (b) lands inside this same effort. Status of `mnemonic-cli` Phase 3 is upgraded from "future Phase 3" to **"prerequisite or co-requisite of ERC-8004"** — see decision entry in `decisions.md` and the cross-reference in `work/mnemonic-cli/backlog.md`.

### What we're integrating with

ERC-8004 = three on-chain registries on Ethereum mainnet:

- **Identity Registry** — ERC-721 NFT per agent. `agentId = tokenId`. `tokenURI` resolves to off-chain JSON registration file with `services[]` (a2a, mcp), `supportedTrust[]` (reputation, crypto-economic, tee-attestation), `registrations[]` (cross-chain bindings). Reserved metadata key `agentWallet` managed via EIP-712 / ERC-1271.
- **Reputation Registry** — `giveFeedback(agentId, value, valueDecimals, tag1, tag2, feedbackURI, feedbackHash)`; `appendResponse(...)`; `revokeFeedback(...)`; `readAllFeedback(...)`; `getSummary(...)`. On-chain commits a hash; off-chain JSON carries the rich payload.
- **Validation Registry** — `validationRequest(validator, agentId, requestURI, requestHash)` then `validationResponse(requestHash, response, responseURI, responseHash, tag)`. Spec status: "still under active update and discussion with the TEE community" → schema-lock against current rev would create migration debt; ship behind `experimental` feature flag until stable, same discipline as A2A.

Off-chain registration file (the `tokenURI` target):

```jsonc
{
  "type": "schema identifier",
  "name": "...",
  "services": [
    { "type": "a2a", "uri": "https://my-agent.example/a2a" },
    { "type": "mnemonic", "uri": "https://mcp.mnemonik.xyz/agent/<pubkey>" }
  ],
  "supportedTrust": ["reputation", "tee-attestation", "signed-memory-attestation"],
  "registrations": [{ "agentRegistry": "eip155:1:0x...", "agentId": "<tokenId>" }]
}
```

### Four integration paths

#### Path 1 — Mnemonic as a registered validator (Validation Registry)

Strongest fit. The Validation Registry is exactly an off-chain-attestation hook. Mnemonic registers as a validator address; agents call `validationRequest(MNEMONIC_VALIDATOR_ADDRESS, agentId, taskURI, taskHash)`; the validator service consumes the referenced A2A task through the existing `bridge-a2a/` middleware, produces an `A2A_TASK_V1` attestation, hosts it on Arweave (already in `core/src/arweave/`), and posts `validationResponse(requestHash, OK, mnemonic_attestation_uri, blake3_hash, tag="mnemonic-a2a-v1")`.

Verification by any third party: fetch `responseURI` → COSE_Sign1-verify against Mnemonic's published Ed25519 pubkey → recompute blake3 → compare with on-chain `responseHash`. Trustless; no `mcp.mnemonik.xyz` in the trust path.

This becomes a **new product surface**: validator-as-a-service on ERC-8004. Maps cleanly onto existing `mcp/src/payment.rs` (x402 / balance) — per-validation fee, same flow as `mnemonic_sign_memory`.

#### Path 2 — Mnemonic declared in agents' own registration files

Minimal integration. Any agent that uses Mnemonic to sign its memory adds in its registration file:

- `services[]`: `{ type: "mnemonic", uri: "https://mcp.mnemonik.xyz/agent/<pubkey>" }`
- `supportedTrust`: include `"signed-memory-attestation"` (proposed value; document upstream to ERC-8004 working group).
- `setMetadata(agentId, "mnemonic.ed25519_pubkey", <bytes>)` on the Identity Registry.

On-chain consumer resolves `agentId → registration file → Mnemonic pubkey` in two reads, then verifies any subsequent Mnemonic attestation independently.

Ship: a CLI command `mnemonic erc8004 register-file --pubkey ...` emitting the JSON, plus a small Solidity helper or doc explaining `setMetadata` from the agent owner's wallet.

#### Path 3 — Mnemonic-attested entries in the Reputation Registry

`giveFeedback` accepts `feedbackURI` + `feedbackHash` for off-chain richness. The spec example payload already carries `a2a` and `mcp` namespaces. Add a third:

```jsonc
{
  "agentRegistry": "eip155:1:0x...",
  "agentId": "<tokenId>",
  "clientAddress": "0x...",
  "createdAt": "2026-...",
  "value": 9800, "valueDecimals": 2,
  "mnemonic": {
    "schema": "MEMORY_V1",
    "attestation_id": "...",
    "cose_envelope_uri": "ar://...",
    "blake3": "..."
  }
}
```

`feedbackHash` = blake3 of canonical JSON. Trust upgrade: today any wallet can spam `giveFeedback`; Mnemonic-signed feedback proves the rater is the same long-lived signing identity that produced N other attestations. Ship: SDK + CLI helper that produces the URI bytes, hashes, and emits the on-chain call.

#### Path 4 — Three-way identity reconciliation via `did:mnemonic:`

Today three identifiers coexist:

| Layer | Identity |
|---|---|
| A2A | AgentCard JWS pubkey |
| Mnemonic | Ed25519 pubkey, base58 |
| ERC-8004 | ERC-721 `tokenId` (+ `agentWallet`) |

The reconciliation chain already has its first link in `work/a2a-bridge/tasks/4.md` (the AgentCard `x-mnemonic` extension). ERC-8004 closes it: `tokenId → registration file → AgentCard URL → x-mnemonic.ed25519_pubkey_base58`, with Mnemonic envelopes carrying that pubkey directly.

Ship: a `did:mnemonic:` resolver as the unified handle. Format: `did:mnemonic:<namespace>:<chainId>:<contract>:<tokenId>` (eip155 namespace) or `did:mnemonic:solana:<base58_pubkey>` (Solana namespace, legacy). One agent, multiple ecosystems, one cryptographic root.

### Solana decoupling — what counts as "broken"

For ERC-8004 V1 we do NOT need full Phase-3 chain-pluggability across every code path. We need a strict subset:

- **Anchor layer must be pluggable.** `core/src/storage/sqlite.rs` schema today encodes Solana-specific anchor fields. New abstraction: `Anchor::{Solana(memo_sig), Ethereum(tx_hash, contract_addr), Arweave(tx_id), None}` — discriminated, additive, idempotent migration.
- **Anchor writer is a trait, not a function.** `trait AnchorWriter { fn anchor(&self, blake3: &[u8; 32]) -> Result<AnchorRecord> }`. Solana SPL Memo path becomes one impl; Ethereum-via-ERC-8004-Validation-Registry call becomes another impl. Selection via config (`STORAGE_MODE` extends to carry anchor preference).
- **What we explicitly do NOT need for ERC-8004 V1:** off-chain envelope alg-pluggability (Option B of `mnemonic-cli` Phase 2). Ed25519 stays as the off-chain signer; only the *anchor* gains a second backend. This narrows the scope versus full Phase 3 — call it "Phase 3α".

For the validator-as-a-service in Path 1, the ERC-8004 `validationResponse` Ethereum tx **is** the anchor. No separate Solana memo needed. This is the architectural insight that makes the decoupling cheap: ERC-8004 doesn't *add* a new anchor — it reveals that the existing on-chain `responseHash` already commits the data, so for ERC-8004-routed attestations we get anchoring for free with the protocol call.

### Tasks (estimated)

- **erc8004-0 — Anchor pluggability (Phase 3α)** [PREREQUISITE]
  - Refactor `core/src/storage/sqlite.rs` to carry `Anchor` enum instead of solana-only columns.
  - Define `AnchorWriter` trait in `core/src/anchor/`.
  - Wrap existing Solana SPL Memo path as `SolanaAnchor`.
  - Add `EthereumAnchor` stub (no Ethereum client yet; just the type).
  - Idempotent SQLite migration; legacy rows = `Anchor::Solana(...)`.
  - **~5 dev-days.**

- **erc8004-1 — Registration file generator + identity binding**
  - CLI command `mnemonic erc8004 register-file --pubkey ... --a2a-card-url ...` emits JSON.
  - SDK helper `buildErc8004RegistrationFile(...)`.
  - Doc + smoke against an ERC-8004 testnet registry.
  - **~2 dev-days.**

- **erc8004-2 — Validator service**
  - Built on `bridge-a2a/`. Watches Validation Registry events, consumes A2A task, produces attestation, posts `validationResponse`.
  - Hosts attestation on Arweave (re-uses `core/src/arweave/`).
  - Ethereum client integration (`alloy-rs` is the leading candidate; vet during this task).
  - **~5 dev-days.**

- **erc8004-3 — Reputation feedback helper**
  - SDK + CLI: `submitMnemonicFeedback(agentId, value, attestationId)`.
  - Off-chain JSON document spec for the `mnemonic` namespace inside feedback payload (open PR to ERC-8004 working group proposing canonical schema).
  - **~2 dev-days.**

- **erc8004-4 — `did:mnemonic:` resolver**
  - DID method spec (lightweight; mirror established DID patterns).
  - Resolver in `mnemonic-core` (pure function, given a `did:mnemonic:` string returns a verified Ed25519 pubkey + linked identifiers).
  - Tests covering eip155 + solana namespaces.
  - **~3 dev-days.**

- **erc8004-5 — Ethereum anchor end-to-end**
  - Implement `EthereumAnchor` against ERC-8004 Validation Registry and/or generic Ethereum-tx anchoring contract.
  - Integration with Path 1's validator service (anchor falls out of `validationResponse` automatically).
  - **~3 dev-days.**

**Total: ~20 dev-days**, ordered as: erc8004-0 → (erc8004-1 || erc8004-3) → erc8004-2 → erc8004-5 → erc8004-4. Roughly half the size of A2A bridge V1, riding entirely on its substrate.

### What it brings Mnemonic

- **Position on the highest-leverage substrate.** ERC-8004 is *built on* A2A and went mainnet five months ago. Validators that show up first own the namespace. The two declared validator categories already filling are TEE (Phala, Marlin) and crypto-economic (staking) — Mnemonic occupies a third (signed-memory) that nobody else holds.
- **Stacks atop the A2A bridge for free.** Path 1 is essentially "the A2A bridge with an Ethereum tx at the end". Most engineering is already in `work/a2a-bridge/`.
- **On-chain reputation surface** that no off-chain memory competitor (letta / zep / mem0 / cognee) can replicate without first building signed attestations — a moat compounding on a moat.
- **New revenue surface** — validator-as-a-service is a per-call fee market; infrastructure already exists in `mcp/src/payment.rs`.
- **Forces the missing identity story.** `did:mnemonic:` driven by ERC-8004 binding turns identity from "Ed25519 base58" into something with a concrete on-chain anchor.
- **Closes the SVM-lock concern** as a side effect — the ERC-8004 trigger is the cleanest argument for landing Phase 3α now rather than later.

### Costs / risks

- **Validation Registry API is unstable** ("under active update and discussion with the TEE community"). Mitigation: ship behind `erc8004-experimental` feature flag, freeze schemas only at ERC-8004's next stable rev.
- **Hosting / availability of off-chain attestation files.** Arweave covers it; if a consumer can't fetch the URI they can't verify. Same liveness assumption as everywhere off-chain. Document explicitly in `references/threat-model.md`.
- **Ethereum gas costs for `validationResponse`** — every validation costs gas. Pricing engine in `mcp/src/pricing.rs` extends to include gas estimate; pass-through to the requesting agent via x402.
- **EIP-712 / ERC-1271 `agentWallet` mapping** is Ethereum-native; mapping `agentWallet` ↔ Ed25519 pubkey is new ground. Registration file is the natural binding document — declares both, signed by both keys; document the security model.
- **Adoption tied to ERC-8004 adoption.** Hedge same as A2A — the anchor pluggability we land is reusable for any future on-chain-anchor protocol (Polygon, Base, Arbitrum-native registries, Solana on-chain identity once it exists).

---

## Cross-cutting: shared `mnemonic-protocols/` crate

If we build A2A + MCP-delegation + ACP bindings, the `attest_*` / `recall_by_context` skeleton repeats. Refactor target: extract the common adapter pattern into `mnemonic-protocols/` with per-protocol modules. Premature for V1 (one binding); revisit at V2.

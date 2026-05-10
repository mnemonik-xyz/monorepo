---
created: 2026-05-01
status: locked-in (2026-05-01)
type: research / positioning
audience: contributors, reviewers, future-self
---

# Positioning: Verifiable Memory for Trustless Agents

This document is the strategic rationale behind the `work/a2a-bridge/` body of work and its ERC-8004 follow-on (see `backlog.md`). It locks in the positioning the project is committing to:

> **Mnemonic is verifiable memory for trustless agents.**

Not "memory for AI agents" (broader, head-to-head with letta / zep / mem0 / cognee on retrieval quality). Not "agent identity" (head-to-head with ERC-8004 / DIDs). Not "execution attestation" (head-to-head with TEE attestations). The specific, defensible niche is **cryptographic provenance over content the agent itself claims to remember**, composed underneath the trustless-agent stack.

The rest of the document substantiates this: what gaps in the trustless-agent stack we close, what we explicitly do not close, why the composition story works, and the three regimes under which we should expect the bet to pay off.

---

## 1. The trustless-agent stack as it exists in May 2026

After A2A v1.0.0-rc and ERC-8004 mainnet (2026-01-29), the stack of standards that defines a "trustless agent" looks like this:

| Layer | Standard / primitive | What it proves |
|---|---|---|
| Wire protocol | A2A (JSON-RPC + SSE), MCP | Agents can talk in standard shapes |
| Identity (off-chain) | A2A AgentCard JWS | This is who *claims* to be the agent |
| Identity (on-chain) | ERC-8004 Identity Registry (ERC-721) | There is a wallet-owned on-chain handle |
| Reputation | ERC-8004 Reputation Registry | Other parties have rated this agent |
| Validation hook | ERC-8004 Validation Registry | An external validator attested *something* |
| Execution attestation | TEE validators (Phala, Marlin) | The code ran in attested hardware |
| Economic guarantee | Crypto-economic validators | Stake is at risk to back a claim |

Every layer above is a real, deployed standard with running code in May 2026. None of them does what Mnemonic does. The next section enumerates the precise gaps.

---

## 2. The five gaps Mnemonic closes

### Gap 1 — Persistent, portable memory

A2A explicitly disclaims memory: agents collaborate "without needing access to each other's internal state, memory, or tools." ERC-8004 is identity + reputation + validation hooks — no memory layer. Without Mnemonic the only memory in the stack is whatever vendor-specific store the agent runs on (OpenAI Assistants memory, Anthropic projects, vendor-locked vector DB). Switch vendor, lose memory. This is the original Mnemonic problem statement and **the trustless-agent stack does not provide a substitute**.

### Gap 2 — Verifiable provenance over *content*, not just execution

TEE attestations prove "the hardware was real when this ran." Crypto-economic validators prove "stake is at risk." Neither validates *what the agent claims to remember*. A TEE can faithfully execute a model that fabricates a memory — the attestation says nothing about whether the memory is the same one the agent recorded last week.

Mnemonic's envelope says: *"this exact memory bytes existed at time T, signed by this identity, links to these prior memories. If anyone — including the agent — modifies them, the chain breaks visibly."* That is a structurally different proof from "execution was attested."

### Gap 3 — Cross-vendor, cross-session temporal coherence (lineage)

A2A `contextId` is local to a single A2A server. ERC-8004 entries are independent on-chain calls. Neither has any notion of "memory M2 succeeds M1 in this agent's timeline." Mnemonic's `prev_id` lineage DAG provides exactly that — and `recall_by_context` makes it queryable. This is what makes claims like *"agent X has consistently held this position across 2,400 tasks over six months"* provable rather than narrative.

### Gap 4 — Semantic recall over signed history

ERC-8004's read paths (`readAllFeedback`, `getAgentValidations`, etc.) are exact-handle lookups. A2A's are session-scoped. Mnemonic adds **cosine search over the agent's signed memory** — *"what has this agent recorded about topic X across vendors and sessions?"* That is a different query class. No layer in the trustless-agent stack offers it.

### Gap 5 — Long-lived signing identity decoupled from wallet identity

ERC-8004 agent identity = ERC-721 NFT owned by a wallet. Wallets get rotated, NFTs get transferred, custody changes. Mnemonic's Ed25519 signing identity is stable across all of that, and the registration-file binding (Path 2 of ERC-8004 V1, see `backlog.md`) reconciles them: an agent can change wallets, get its NFT transferred, and still have a continuous, verifiable signing history. **The reputation flywheel doesn't reset every time the wallet does.**

---

## 3. The composition advantage — Mnemonic is the off-chain layer the others assume but don't define

ERC-8004 is intentionally a thin on-chain layer pointing at off-chain content-addressed documents. It does not specify what those documents look like. The off-chain doc standard is currently a wide-open ecosystem question — TEE parties use their attestation formats, JWS parties use JWS, custom validators ship custom JSON. Mnemonic's COSE_Sign1 over deterministic CBOR with a published conformance suite (see Task 8 in `tasks/`) is one of the cleanest, most independently-verifiable shapes available, and the only one that's natively designed for **memory** rather than retrofitted from another problem domain.

Same story with A2A: it has `contextId` but no spec for what a "session memory" *is*. Mnemonic provides the canonical envelope.

If the integration work in `tasks/` and `backlog.md` ships well, Mnemonic is positioned to be **the canonical signed-memory off-chain format the trustless-agent ecosystem converges on**, similar to how DID Documents converged once W3C ratified the DID Core spec.

---

## 4. What we are NOT closing — being honest

This section exists to keep us from overclaiming, the same way the second message in this branch's research history (`docs-actualization` review) flagged earlier. Self-correction is cheap; reputation damage from over-pitching is not.

- **Trustless execution.** TEE attestations close that. Mnemonic envelopes say "I claim I produced this output"; they do not say "this code ran in a verified enclave." The two compose, but Mnemonic alone does not attest execution.
- **Economic skin in the game.** Crypto-economic validators bond stake; we do not.
- **Consensus / adjudication.** If two agents claim contradictory things, our DAG records both. We surface forks; we do not resolve them.
- **Better recall than mature memory systems.** Letta / zep / mem0 / cognee have years of retrieval-quality work. We are not strictly better at *recall quality*. We are better at *verifiable* recall — a different axis. Anyone whose primary need is "best possible memory" without provenance gets less from us than from them.
- **Magic for unsigned events.** If an A2A message goes through without bridge attestation, it's gone — same as today. Mnemonic does not retroactively prove anything; it has to be in the loop at write time.

---

## 5. The single-sentence pitch

Once the work in `work/a2a-bridge/tasks/` (A2A bridge V1) **and** the ERC-8004 follow-on in `backlog.md` (V1 + Phase 3α anchor pluggability) **and** the `did:mnemonic:` resolver (erc8004-4) all ship:

> Mnemonic is the only primitive in the trustless-agent stack that gives **cryptographic provenance over content the agent itself claims to remember, cross-vendor temporal coherence via lineage, and semantic recall over that signed history** — composable underneath A2A, anchored through ERC-8004's existing on-chain commitments without binding the agent's signing identity to a wallet.

That sentence is true *only if* all four pieces ship. Any one missing and the story collapses to "we have a nice memory format" — competitive with letta / zep / mem0 but not differentiated.

---

## 5b. Ecosystem partners — concrete deployments of the positioning

The standards-track work in §5 (A2A bridge, ERC-8004, `did:mnemonic:`) builds the durable surface. In parallel we land the same positioning against named runtimes and memory architectures with deployed users — these are the **near-term reference deployments** that prove the pitch in production while the standards work matures.

- **Hermes Agent runtime (Nous Research)** — multi-platform agent runtime with an explicit Memory Provider extension point and seven existing providers (Honcho, OpenViking, Mem0, Hindsight, Holographic, RetainDB, ByteRover). Mnemonik lands as the 8th — and the only cryptographically verifiable one. Six integration surfaces, four-step rollout (MCP registration → RL trajectory attestation demo with Nous → upstream Memory Provider PR → plugin + middleware bundle). The RL trajectory attestation in particular gives Nous a "verifiable RL datasets" pitch aligned with their open-training ethos. Full proposal: [`../../../.claude/skills/project-knowledge/recovered/research/mnemonik-hermes-integration.md`](../../../.claude/skills/project-knowledge/recovered/research/mnemonik-hermes-integration.md).
- **Hindsight × Mnemonik adapter (Latimer et al., arXiv:2512.12818)** — Hindsight is a four-network (W/B/O/S) cognitive memory architecture already shipped as a Hermes provider; Mnemonik composes underneath as the trust layer. Six contradictions reconciled (mutability, async observation regen, missing identity, latency, unverified extraction, closed evaluation). Cost model with five mitigations brings naive ~$5–7.5k/mo per 1k heavy users down to ~$500–1.5k via Merkle batching + selective per-network policy + sync-sign-async-anchor. Full analysis + cost model: [`../../../.claude/skills/project-knowledge/recovered/research/hindsight-mnemonik-analysis.md`](../../../.claude/skills/project-knowledge/recovered/research/hindsight-mnemonik-analysis.md).

Why these two are tracked here rather than alongside protocol bindings: Hermes is a **runtime**, Hindsight is a **memory architecture**. Per the framework in `protocol-integrations.md`, runtimes and memory architectures compose with Mnemonik via SDK adapters and per-architecture mappings, not via new core schemas. Both reuse `MEMORY_V1` (Hindsight one envelope per W/B/O/S network; Hermes one envelope per attested turn) — no schema-lock pressure, which makes them safe to ship before the standards-track work GAs.

Strategic role: the standards-track integrations build the long-horizon canonical surface; the ecosystem-partner deployments validate the positioning *now*, in front of named users, while we wait for A2A and ERC-8004 to compound.

---

## 6. Three-regime decision analysis

We commit to this work knowing the outcome depends on adoption beyond our control. Three plausible regimes:

**Regime 1 — Trustless-agent stack takes off** (A2A + ERC-8004 reach meaningful adoption; multiple ecosystems converge).
This work is the *cheapest* way to occupy the canonical-signed-memory position. Roughly 30 dev-days total (A2A V1 ~12d + ERC-8004 V1 ~20d, of which ~5d is anchor pluggability). Disproportionate leverage. Network effects compound — every signed attestation makes the next more valuable.

**Regime 2 — The stack stalls but signed memory still matters** (some teams want verifiable memory regardless of multi-agent protocols).
The A2A bridge surface is wasted; the ERC-8004 anchor pluggability is half-wasted (it's still useful as plain Ethereum anchoring); the core schema and conformance work was useful. Roughly 40% salvage value.

**Regime 3 — Neither stack nor signed memory takes off**.
Most of this work is wasted. Hedge: keep schemas extensible, do not couple our roadmap to A2A's GA timeline, ship behind feature flags so we can deprecate without breaking core.

We commit because regime 1 is plausible (mainnet ERC-8004 + A2A v1.0.0-rc are not vapor, and the first-mover window for validator-class participation is closing on a months-not-years timescale) and regime 2's salvage value is meaningful enough to make the bet asymmetric.

---

## 7. What this positioning *forecloses*

Locking in "verifiable memory for trustless agents" means we explicitly decline several adjacent positions, listed here so the trade-off is explicit:

- **General-purpose AI agent memory** (head-to-head with letta / zep / mem0 / cognee on quality alone). We will lose that fight; we are not optimizing for it.
- **Agent identity standard** (head-to-head with W3C DIDs / ERC-8004 directly). We compose with these standards, we don't replace them. `did:mnemonic:` is a *binding*, not a competing identity scheme.
- **General storage / KV** (head-to-head with Arweave SDK, IPFS, etc.). Storage is an anchor option; it is not the product.
- **Pure on-chain protocol**. The off-chain envelope is the product; on-chain is one of several anchor backends.

If at any point we feel the pull toward one of these adjacent positions, this document is the artifact to revisit before re-pivoting.

---

## 8. References

- `work/a2a-bridge/user-spec.md`, `tech-spec.md` — feature plan.
- `work/a2a-bridge/backlog.md` — ERC-8004 detailed plan + other protocol integrations.
- `work/a2a-bridge/decisions.md` — decision log including the Solana-decoupling and positioning lock-in entries.
- `.claude/skills/project-knowledge/references/protocol-integrations.md` — protocol-integration index across A2A, ERC-8004, MCP-delegation, ACP, AGNTCY.
- `.claude/skills/project-knowledge/references/project.md` — high-level project doc, updated to reflect this positioning.
- `docs/WHITEPAPER.md` §14 Roadmap — public roadmap, updated with Phase 5 reflecting this work.
- A2A: https://a2a-protocol.org/latest/specification/
- ERC-8004: https://eips.ethereum.org/EIPS/eip-8004

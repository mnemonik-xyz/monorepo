---
created: 2026-07-01
type: product-definition
status: draft
role: product-manager
assumption: selected zkVM proves arbitrary programs; recursion cost ignored
reads:
  - ../protocol/business-model.md
  - ../protocol/design.md
  - ../computation-proof/positioning.md
  - ../computation-proof/v1-agentic-payments.md
  - ../computation-proof/architecture.md
  - ../zigz-research-report.md
---

# Mnemonic — Product Definition & Readiness Assessment

> Scope note: this document distinguishes **built** (code exists and is tested in
> the repo today), **partial** (a working seam/mock exists but not the real
> thing), **designed-only** (a spec/decision exists, no code), and **missing**.
> The assessment is candid — several load-bearing pieces are designed, not built.

---

## 1. What the product is

**One-liner.** Mnemonic is the open, permanent, independently-verifiable record of
what an AI agent was authorized to do, what it knew, what it did, and the proof
that the action matched the mandate — a record that survives the transaction.

**Elevator pitch (3 sentences).** When a regulated enterprise runs AI agents, it
must later *prove* — to an auditor or regulator who does not trust the vendor —
that each agent acted within its principal's signed policy. Mnemonic produces a
signed, content-addressed **intent → action** record bound to a correspondence
proof and authenticated evidence, anchored for time and held in independent
durable custody, that anyone can re-verify offline with an open library. Delta and
AP2 gate the payment moment and then forget; Mnemonic is the forensic record that
outlives it — open verifier, permanent retrievable custody, and the knowledge link
they cede.

**Category.** Verifiable AI-agent accountability / audit infrastructure
(open-core protocol + managed proving & durable-custody SLA + compliance SaaS).
Adjacent to, and composable with, agentic-payment rails (AP2, Delta, x402).

---

## 2. Target users & buyers

The **user** operates agents day-to-day; the **buyer** is accountable for the
audit outcome. They are rarely the same person — this gap defines the sales motion.

| | Who | Role | Job-to-be-done |
|---|---|---|---|
| **Primary buyer** | Regulated enterprise running AI agents (banks, insurers, healthcare, brokerages) — CISO / Head of Compliance / GRC | Buyer + accountable party | "Prove my agents acted within policy or I face fines/liability (EU AI Act Aug-2026, FINRA, SOX, HIPAA)." Buys audit-grade proof + durable custody as a managed service. |
| **Secondary buyer** | AI agent platforms & frameworks (agent-commerce, orchestration) | B2B2B distributor | "Sell 'compliant / auditable' to regulated customers." Embeds the SDK; we are infrastructure underneath them. |
| **Secondary buyer** | Auditors / GRC vendors / Big Four | Consumer / white-label | "Run the actual audit." Consumes or white-labels the verifier + dashboards; wants a format they already trust. |
| **User (not buyer)** | Agent developer / platform engineer | Integrator | "Emit intent/action records and proofs from my agent with a few SDK calls, without becoming a cryptographer." |
| **User (not buyer)** | Regulator / auditor at verify time | Relying party | "Independently re-check a record I was handed, offline, without trusting the producer or the vendor." |

The decisive JTBD is the last one: **verify without trusting the vendor.** It is
why the verifier must be open — a closed proprietary audit proof is a non-starter
for compliance, and it is the structural wedge against Delta.

---

## 3. Value proposition

**Before → after.**

| | Before (today's agent audit trail) | After (Mnemonic) |
|---|---|---|
| Evidence of compliance | Vendor-hosted logs; trust-the-operator | Self-verifying object; trust-the-math |
| Who can verify | The vendor, on request | Anyone, offline, with an open library |
| Tamper-evidence | DB-level, mutable, deletable by producer | Content-addressed + anchored + independent custody |
| What's captured | "The action happened" | Intent + knowledge + action + proof + evidence, bound as one record |
| Longevity | As long as the vendor keeps the log | Permanent, retrievable custody (audit-grade) |

**Why now.**
- **EU AI Act** high-risk obligations bite around **Aug 2026** — a hard deadline
  with budget and existential risk attached.
- **Agentic payments are live**: AP2 (Agent Payments Protocol), Shopify UCP, x402,
  and Delta's funded closed alpha prove the demand for "prove the agent bought the
  right thing" *right now*.

**Differentiators vs Delta / AP2** (the four edges Delta structurally cannot copy):

1. **Open verifier** — a pure-Rust + WASM library the relying party runs itself; a
   regulator never trusts Mnemonic-the-vendor. Delta's verifier is closed & hosted.
2. **Permanent retrievable custody** — the record is anchored *and* held by a
   custodian who is not the producer, so the subject of an audit cannot delete its
   own evidence. Delta is payment-moment-ephemeral (prove, settle, forget).
3. **Knowledge link** — binds the Mnemonic-signed memory the agent retrieved at
   decision time into the same record: an audit log becomes a *forensic* record.
   Delta explicitly cedes "what the agent knew."
4. **Policy provenance** — the policy is a content-addressed program in a registry;
   the principal's signed intent binds the exact `program_hash`, so no one can
   silently swap in a weaker policy. Independently re-derivable from source.

Positioning: *composition, not competition* — Mnemonic can even re-verify a
Delta/SP1 or zkTLS proof (`proof_kind`) and wrap it in a permanent open record.

---

## 4. The product surface (what a customer consumes)

| Surface | What the customer does with it | Backing components today | State |
|---|---|---|---|
| **SDK** (`@mnemonik-xyz/sdk`, TS+WASM) | Build/sign envelopes, call tools, **verify client-side** | `packages/sdk` (client, signer, keypair, OAuth, WASM verifier) | **Built** (memory path); correspondence methods **missing** |
| **MCP tools** | Agents sign/recall/verify over JSON-RPC | `mcp/src/tools.rs`, `mcp.rs` — 10 tools live (`sign_memory`, `recall`, `verify`, `verify_trajectory`, `attest_step/verdict`, `whoami`, `prove_identity`, `publish_post`, `check_pending`) | **Built** for memory/trajectory; **no** `verify_correspondence` tool |
| **Verifier library** | Auditor/counterparty/contract re-checks a record offline | `core/src/correspondence` (`verify_correspondence`, 5 checks, `action_commitment`) native+WASM | **Built but behind `correspondence-experimental` feature flag; MockVerifier only; not wired to MCP/SDK** |
| **Dashboard / API** | Compliance sees traction, ledger, exports | `mcp/src/api.rs` (public Ledger + `analytics/attestations`), `webapp/` (Ledger, Analytics, Sign, Chat, Consent pages) | **Built** for memory attestations; **no** compliance/audit dashboard for correspondence |
| **Durability SLA** | Guarantee the payload is retrievable "forever" | `core/src/arweave` + `core/src/solana` (participate mode: Arweave bytes + Solana SPL Memo), pricing engine (`mcp/src/pricing.rs`) | **Partial**: single-copy Arweave + Solana timestamp exist; **durability *classes* (D1/D3), ANS-104 batching, k-of-n relay receipts, SLA are designed-only** |
| **Policy registry** | Author/publish/pin an audited policy | schema constants only | **Designed-only** |

**The blunt read:** the *memory* product (sign / recall / verify / anchor a
semantically-embedded memory) is a real, shipped, tested system. The *agentic-audit*
product (intent → action → proof → verify → durable custody) that this document is
about is **~80% designed, ~20% built** — the verify-side skeleton exists (Wave 1),
the producing side is mocked, and it is not exposed through MCP/SDK/dashboard yet.

---

## 5. What exists vs what's needed

| Capability | State | Evidence in repo |
|---|---|---|
| Signed, content-addressed, anchored **memory** objects (COSE_Sign1 + blake3 + CBOR) | **Built** | `core/src/codec`, `core/src/identity`, MCP `sign_memory`/`recall`/`verify` |
| Semantic recall (embeddings, TurboQuant compression, cosine search) | **Built** | `core/src/embed`, `core/src/compress`, `storage/sqlite` |
| Trajectory / lineage attestation + verify | **Built** | `core/src/trajectory`, `lineage`, `attest_step`/`attest_verdict`/`verify_trajectory` |
| **INTENT_V1 / ACTION_V1 schemas** (AP2-aligned) | **Built** (Wave 1) | `core/src/codec/schema.rs` |
| **`verify_correspondence`** (5 independent checks) + `action_commitment` binding | **Built but experimental & mock** | `core/src/correspondence/mod.rs` (feature-gated, `MockVerifier`) |
| Wave-1 **producer** (proof + evidence seams) | **Partial (mock)** | `prover/` — `MockProver`, `StubEvidence`, feature-gated empty by default |
| **Real prover** (zigz policy proof, `payment_mandate_v1` guest) | **Designed-only** (spikes prove feasibility) | zigz research + spikes; no `ZigzProver` in `prover/src` |
| Pure-Rust zigz **verifier** in `core` (Wave 2) | **Missing** | trait `CorrespondenceVerifier` exists; only `MockVerifier` impl |
| **zkTLS evidence** (TLSNotary over merchant/PSP) | **Designed-only** | `EvidenceSource` trait + `StubEvidence` stub |
| **Policy registry** (publish, `program_hash`, reproducible build, pin) | **Designed-only** | `architecture.md §8`; no code |
| **Durability service** — D1 accountable relays, D3 ANS-104 batching, availability SLA | **Designed-only** | `design.md` storage section; only single-copy Arweave exists |
| Anchor abstraction (OTS→Bitcoin default, TSA, Solana as one backend) | **Partial** | Solana SPL Memo built; **`Anchor` trait / OTS / TSA missing** |
| Correspondence wired to **MCP tools & SDK** | **Missing** | no `correspondence` reference anywhere in `mcp/` or SDK exports |
| **Compliance dashboards** (policy authoring, audit export, retention) | **Designed-only** | webapp has memory Ledger/Analytics; no audit console |
| **Billing** (per-attestation dynamic price, USDC balance/x402) | **Built** for memory | `mcp/src/pricing.rs`, `payment.rs`, `escrow.rs` |
| Conformance / differential vectors (Rust prover ↔ Rust verifier) | **Missing** | planned Wave-2 deliverable |

---

## 6. MVP definition (one agentic-payments design partner)

**Goal:** deliver end-to-end value to a single agentic-payments design partner —
prove and later re-verify that an agent's purchase matched a principal's signed
mandate, backed by evidence, in permanent independent custody.

**Exact feature set (smallest that is real, not mocked):**

1. **INTENT_V1 / ACTION_V1** signed envelopes (built) referencing `policy_id` +
   `intent_hash`. AP2 Intent-Mandate-aligned.
2. **`payment_mandate_v1`** as a *real* zigz guest program proving: `amount ≤ cap`;
   currency & category ∈ allowed; `merchant_id ∈ allowlist` (Merkle membership);
   `ts ≤ expiry`; and the **binding clause** `action.amount == evidence.amount ∧
   action.merchant_id == evidence.merchant_id`.
3. **Real pure-Rust zigz verifier** replacing `MockVerifier`, with a frozen
   `zigz-proof-v1` format and **differential conformance vectors**.
4. **Evidence**: `StubEvidence` acceptable for the MVP demo *only if* labeled a
   dev trust-hole; a thin **zkTLS/TLSNotary** path over one merchant/PSP surface is
   the credibility unlock and should be the MVP stretch goal.
5. **Durable custody**: D3 single-copy Arweave (built) is the MVP floor; **ANS-104
   batching + a stated availability target** is the sellable version.
6. **Expose** `mnemonic_verify_correspondence` via MCP + one SDK method + an
   auditor CLI `verify` command. Verification must run **offline** and in WASM.
7. **Anchor**: reuse Solana SPL Memo for the MVP (built); OTS-default can wait.

**The demo.** A published **purchase-intent benchmark**: N scenarios split into
*compliant* and *non-compliant* (over-cap, off-allowlist, expired,
evidence-mismatch). Run each through produce → anchor → store, then hand the record
to a fresh verifier process (and a browser WASM build) that re-checks it with no
network access to the producer.

**Success metric (binary, publishable).** MVP passes iff **every compliant
purchase verifies `Some(true)` and every non-compliant one fails the proof
(`Some(false)`)**, each result independently re-verifiable by the open library and
re-fetchable from durable custody. This answers Delta's benchmark on our own terms
— we publish the vectors, no unverifiable percentage claims.

---

## 7. Packaging & pricing sketch

Standard open-core infra playbook: the protocol + verifier are free (that openness
is the wedge); revenue is operating the guarantee.

| Layer | What it is | License / model | Pricing axis |
|---|---|---|---|
| **Open core** | Protocol, envelopes, `verify_correspondence`, WASM verifier, reference prover | Apache-2.0 (already) | free — adoption wedge |
| **Managed proving** | Hosted zigz proving so customers don't run zkVM infra | Metered | **per-proof** (tiered by policy complexity / steps) |
| **Managed durability** | D1 relays + D3 Arweave batching + availability SLA | Metered + SLA | **per-GB-permanent** + SLA tier |
| **Compliance SaaS** | Policy authoring, audit dashboards, regulator-ready export, retention/revocation, registry hosting | Subscription | **per-seat / enterprise** |
| **Enterprise** | SSO, multi-tenant, SOC2/ISO/eIDAS certs, support | Commercial add-on | annual contract |
| **(Deferred)** | Incentivized relay network + token | — | out of v1 |

Rough axes to converge with sales: **per-proof** (proving cost + margin),
**per-GB-permanent** (Arweave cost + custody margin — the pricing engine already
computes live Irys+SOL/USDC cost per attestation), **subscription** (compliance
console seats + SLA). The moat is operational: durability network at scale,
compliance certifications, being the format auditors already accept.

---

## 8. Metrics / KPI tree

```
North star: verifiable audit records under management (produced + re-verifiable)
├─ Activation
│  ├─ time-to-first-verified-correspondence (SDK install → green verify)
│  ├─ design partners integrated (target: 1 → 3)
│  └─ policies registered & pinned by a relying party
├─ Volume / usage
│  ├─ proofs produced / day  (per policy)
│  ├─ objects under durable custody (count, GB)
│  └─ independent verifications / day (esp. by non-producers = the real signal)
├─ Quality / SLA
│  ├─ verify latency (target: ms, O(log n) — measured ~42–96 ms in spikes)
│  ├─ prove latency (v1-sized intent target: 1–2 s; measured 1.5 s / 4 pays)
│  ├─ durability SLA: retrieval success %, time-to-retrieve, "never lost" count
│  └─ benchmark pass rate (compliant verify / non-compliant fail) = must be 100%
├─ Commercial
│  ├─ design-partner → paid conversion
│  ├─ per-proof + per-GB revenue, gross margin over Arweave/proving cost
│  └─ compliance-SaaS seats / ARR
└─ Trust
   ├─ # of independent parties running the open verifier
   └─ certifications achieved (SOC2 → ISO → eIDAS)
```

---

## 9. "Is it good enough?" — readiness gap analysis

**What makes the architecture strong (real, not aspirational).**
- The **trust model is clean and correct**: trust only the principal's signature +
  proof soundness; agent, prover-honesty, storage, and anchor are all untrusted.
  The five-check verifier and the `action_commitment` binding are *built and
  tested* today — the hard conceptual core is done, not hand-waved.
- **Open, verify-everywhere** (native + WASM) is a genuine structural edge over
  Delta that a funded closed competitor cannot easily neutralize.
- **Prover-agnostic** (`proof_kind: zigz|snark|zktls|mock`) means Mnemonic wins by
  *composition* even with the incumbents.
- The **memory/trajectory product already ships** (signing, anchoring, recall,
  billing, dashboards), so the operational plumbing — identity, CBOR, COSE,
  Arweave, Solana, pricing, MCP, SDK — is proven, not greenfield.
- zigz feasibility is **measured**: stateful evidence-bound payment mandates prove
  correctly in 1–2 s with ms verification. (Assumption granted: arbitrary-program
  proving; recursion cost ignored.)

**What's missing to be sellable (the honest gaps).**
- The correspondence stack is **mock end-to-end** and **feature-gated off**. No
  real prover (`ZigzProver`), no real verifier (only `MockVerifier`), no zkTLS.
- It is **not exposed**: zero wiring into MCP tools, SDK, CLI, or a dashboard.
- **Durability classes are a design, not a service** — today it's single-copy
  Arweave with no batching, no k-of-n relay receipts, and **no SLA**. The whole
  business thesis ("we operate the guarantee") is the least-built part.
- **No policy registry** — the trust root of the entire scheme is unimplemented.
- **No anchor abstraction** — Solana only; OTS→Bitcoin default and TSA are specs.
- **No compliance console / audit export** — the thing the buyer actually opens.

**Top 3 risks.**
1. **The moat is the least-built layer.** Revenue = durability SLA + managed
   proving; both are designed-only. Until an SLA-backed custody service exists,
   there is no product to sell, only a protocol to give away. *Highest risk.*
2. **zkVM (zigz) is unaudited & experimental.** Soundness is the one thing the
   relying party *must* trust; an unaudited prover undermines the compliance pitch.
   The mitigation (differential conformance, open reproducible build, eventual
   audit) is itself unbuilt. Recursion is deferred behind an unproven precompile.
3. **Evidence trust ceiling + standard churn.** zkTLS proves *origin, not truth*;
   AP2/UCP/x402 are moving targets. Overclaiming "verifiable" here is a
   credibility (and regulatory) hazard; the honest framing must be enforced.

**Rough sequence to first revenue.**

| Wave | Deliverable | Unlocks |
|---|---|---|
| **W2 — real prover + verifier** | `ZigzProver` + `payment_mandate_v1` guest; pure-Rust zigz verifier in `core`; frozen proof format + differential vectors; flip the feature flag toward stable | Produce/verify are real, not mocked |
| **W2.5 — expose it** | `mnemonic_verify_correspondence` MCP tool + SDK method + auditor CLI verify | A customer can actually consume it |
| **W3 — registry + evidence** | Policy registry (publish/pin `program_hash`, reproducible build) + one zkTLS/TLSNotary merchant surface | Trust root + real-world evidence; MVP benchmark passes credibly |
| **W4 — durability service** | D3 ANS-104 batching + D1 accountable-relay receipts + a stated availability SLA + anchor trait (keep Solana, add OTS default) | The billable guarantee exists |
| **W5 — one design partner + compliance console** | Sign one agentic-payments design partner; ship the audit dashboard + regulator export; SOC2 track begins | **First revenue** (per-proof + per-GB + pilot subscription) |

**Bottom line.** The *idea* and its *hardest cryptographic core* are sound and
partly built; the *product* — a real prover, real evidence, a registry, and above
all an SLA-backed durability service exposed through tools a buyer consumes — is
mostly ahead of us. It is a credible ~2-quarter path to a first design-partner
pilot, contingent on treating the **durability SLA (the moat)** as a first-class
build target rather than the last one.

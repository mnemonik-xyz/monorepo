---
created: 2026-06-30
updated: 2026-06-30
status: draft
type: v1-scope
vertical: agentic-payments
relates: ./tech-spec.md, ../protocol/design.md, ../protocol/business-model.md
---

# v1 scope — agentic payments

Owner decision (2026-06-30): the v1 design-partner vertical is **agentic
payments**. This pins the first policies, the evidence source, and the first zigz
guest program. Everything here rides the waves in `tech-spec.md`.

## The scenario

A principal pre-authorizes an agent to spend, under constraints (a **mandate**).
The agent makes a purchase. We produce a correspondence proof that **the purchase
matched the mandate**, bound to **merchant-authenticated evidence** — not the
agent's self-assertion. A regulator/PSP/auditor re-verifies it later, offline,
with the open library, and can always re-fetch the record (durable custody).

This is the Delta scenario (the "28.8%→0%" purchase-intent benchmark) — but with
an **open verifier + permanent retrievable custody + the knowledge link**, which
Delta (closed, payment-moment-only) does not offer. Per the full-compete decision,
we **produce** the proof (zigz), not just bind a third-party one.

## Standards alignment (do not reinvent)

- **AP2 (Agent Payments Protocol)** — `INTENT_V1` aligns with the **Intent
  Mandate** (pre-authorized constrained purchasing authority); `ACTION_V1` maps to
  the **Cart / Payment Mandate** (the executed purchase).
- **Shopify UCP** (Universal Commerce Protocol) / **x402** — the agent-commerce
  surfaces the evidence is sourced from. Track, align, don't fork.

## First policy: `payment_mandate_v1` (the first zigz guest program)

A single deterministic guest program proving "the agent bought the right thing,
within budget, from an approved seller, in scope, backed by merchant evidence":

- public inputs: `intent_hash`, `action_commitment`, `evidence_commitment`
- witness: intent `{ cap, currency, allowed_categories, merchant_allowlist_root,
  expiry }`, action `{ amount, currency, merchant_id, category, ts }`, evidence
  `{ merchant-attested amount, merchant_id, line_items }`
- proves:
  1. `amount ≤ cap` (Tier-1 arithmetic)
  2. `currency ∈ allowed` ∧ `category ∈ allowed_categories`
  3. `merchant_id ∈ merchant_allowlist` (Merkle membership vs root) — Tier-2
  4. `ts ≤ expiry` (Tier-5 temporal)
  5. **`action.amount == evidence.amount` ∧ `action.merchant_id ==
     evidence.merchant_id`** — the binding clause. *This* is the Delta lesson:
     the action must agree with merchant-authenticated evidence, so the agent
     cannot claim a compliant purchase without the merchant's receipt backing it.

Clauses 1–4 are the policy; clause 5 is what makes it a *proof about reality*, not
a signed self-claim. All deterministic, all in the cheap circuit regime.

Follow-ons (later guest programs): **best execution** (`|fill − reference| ≤ tol`,
needs a signed/zkTLS price), **velocity/rate limits**, **human-approval above
threshold**.

## Evidence plan (the hard part, phased)

- **Wave 1–2 → `StubEvidence`**: a trusted merchant-receipt blob + identity proof.
  Lets `payment_mandate_v1` + the zigz proof + verify run end-to-end before notary
  ops exist. Clause 5 is real against the stub.
- **Wave 3 → `TlsNotaryEvidence`**: zkTLS over the merchant checkout/receipt
  endpoint (Shopify UCP / PSP). The TLS transcript becomes the attestation — no
  merchant integration. Operational risk concentrated here, isolated by the
  `EvidenceSource` trait.

## Wave mapping (rides tech-spec.md)

- **Wave 1** — `INTENT_V1`/`ACTION_V1` (AP2-aligned) + MockProver + StubEvidence +
  `verify_correspondence`. End-to-end on a trivial mandate. Vertical-agnostic.
- **Wave 2** — `payment_mandate_v1` zigz guest + ZigzProver + pure-Rust verifier +
  frozen `zigz-proof-v1` + differential vectors. **First payments-specific work.**
- **Wave 3** — zkTLS evidence over a real merchant/PSP surface.
- **Wave 4** — knowledge link (bind retrieved memory) + composite Proof + AP2
  hardening.
- **Wave 5** — durability (D1+D3), conformance, audit, QA gate.

## Success metric (honest, demonstrable)

A **purchase-intent benchmark**: N scenarios (compliant + non-compliant —
over-cap, off-allowlist, expired, evidence-mismatch). v1 passes iff **every
compliant purchase verifies and every non-compliant one fails the proof**, with
each result independently re-verifiable by the open library and re-fetchable from
durable custody. Directly answers the Delta benchmark on our own terms; we publish
the vectors (no unverifiable percentage claims).

## Open questions

1. First evidence surface — Shopify UCP, a specific PSP API, or x402 flow?
2. zkTLS backend — TLSNotary (Rust, aligns with core) vs Reclaim/zkPass?
3. Compete-only, or also accept Delta/third-party proofs via `proof_kind` for
   interop with AP2 deployments already using them?

---
created: 2026-06-29
updated: 2026-06-29
status: draft
type: positioning-memo
extends: ./feasibility.md
origin: competitive analysis of Delta Network (delta.network) + AP2 Agent Payments Protocol
---

# Positioning: from "policy compliance" to "intent–action correspondence"

## Why this memo exists

`feasibility.md` framed the goal as *policy compliance* — prove a recorded action
satisfies a predicate. A competitor, **Delta Network**, is shipping the sharper
framing the owner actually wants: **cryptographically verifiable correspondence
between an agent's action and the principal's signed intent**, enforced *before*
funds move. This memo analyses Delta, updates two earlier conclusions, and sets
Mnemonic's defensible evolution.

## What's verified vs not (Delta)

- **Real:** Delta = spender-side settlement enforcement; settles only when a ZK
  proof shows the signed intent was satisfied; **zkTLS natively in the SDK**;
  closed alpha, hosted API, not open source (`delta.network`).
- **Real:** it rides the **AP2 (Agent Payments Protocol)** mandate model —
  **Intent / Cart / Payment Mandate** — the standardised "user signs a typed
  delegation contract" primitive.
- **Unverified vendor claim:** the "28.8% → 0% error rate on 100 Shopify UCP
  purchase intents." Web search surfaced only a 28% *conversion* figure, no such
  error-rate benchmark. Do not repeat it as fact.

## Two earlier conclusions, updated

Delta's architecture resolves the two walls this project kept hitting:

1. **The oracle problem → zkTLS.** Our gating question was "who holds the signing
   key for the external input?" zkTLS (TLSNotary / Reclaim / zkPass lineage) makes
   the answer **nobody** — the merchant's TLS session transcript *is* the
   attestation, no merchant integration. Strictly better than waiting for signed
   feeds. **Update `feasibility.md`'s "trusted oracle" caveat accordingly.**
2. **The DSL question → a zkVM is the rival answer.** Instead of "invent a policy
   DSL → compile to a circuit," Delta writes the policy as a normal program and
   proves its *execution* in the **SP1 zkVM**. Full expressiveness, no per-policy
   circuit compiler, proofs wrap to cheap verification. **Revision:** the
   DSL→circuit path (Noir/gnark) wins only for fixed, high-volume, simple
   predicates; for arbitrary evolving business logic the **zkVM path is more
   pragmatic** — which is why the serious competitor chose it. Mnemonic need not
   pick: it *verifies* whichever produced the proof.

## Decompose "action ⊨ intent" — and who owns each layer

| Layer | Proves | Strong holder | Mnemonic today |
|---|---|---|---|
| 1. **Intent** | principal signed a typed mandate | AP2 standard; Mnemonic turf | signed envelopes already are this |
| 2. **Evidence authenticity** | the world-facts are real | Delta (zkTLS) | ✗ — rebuilding zkTLS is a trap |
| 3. **Correspondence proof** | evidence ⊨ intent | Delta (SP1) | the *verify-in-`core/`* capability we scoped |
| 4. **Binding / anchoring** | intent+evidence+proof+action = one permanent tamper-evident record | **Mnemonic, uniquely** | core competency |
| 5. **Knowledge link** | what the agent *knew* at decision time, authentic | **Mnemonic-only — Delta cedes it** | signed memory + trajectories |

## Strategic call: don't become Delta — be the layer it structurally can't be

Delta is **closed, hosted, payment-moment-ephemeral** (a turnstile: prove,
settle, done) and deliberately ignores *what the agent knew*. Mnemonic's edges are
**open source, permanent anchoring, and the knowledge layer**. Therefore:

**Own layers 1, 4, 5; *verify* (never produce) layer 3; stay prover-agnostic.**
Concretely, generalise the `computation-proof` feature from *policy compliance* to
**intent–action correspondence**:

- New **`INTENT_V1`** artifact — principal-signed typed mandate, anchored.
  **Align its shape with the AP2 Intent Mandate; do not reinvent it.**
- The action artifact references `intent_hash`. The correspondence proof's public
  inputs commit to **both** `intent_hash` and `action_hash` (+ an evidence
  commitment). "This action corresponds to that intent" becomes an independently
  re-checkable, **permanently anchored** fact — not a transient settlement gate.
- **Bind the correspondence + evidence proofs by hash and re-verify in `core/`**
  (SP1 and zkTLS proofs both have cheap verifiers). Extends the existing
  verify-only decision with `proof_kind: sp1 | zktls` alongside `snark`. Mnemonic
  becomes the **open verifier + permanent record** for Delta-style proofs,
  whoever produced them.
- **Keep the knowledge link** — bind the Mnemonic-signed memory the agent
  retrieved at decision time into the same record. That is the difference between
  an *audit log* and a *forensic record*, and it is the question Delta cedes.
- New verifier surface `mnemonic_verify_correspondence` returning per-check
  tri-states (`intent_sig`, `action_sig`, `intent_link`, `correspondence_proof`,
  `evidence_proof`) — same `Option<bool>` convention as `chain_valid`.

**Positioning one-liner.** *Delta is the turnstile; Mnemonic is the permanent,
open, independently-verifiable record of the intent, the knowledge, the action,
and the proof — that survives the transaction.* Composition, not competition —
which is how Delta's own thread frames it, and the honest play given they hold a
funded closed prover and we hold open-source + permanence + the knowledge layer.

## Decision (recommended, pending owner confirmation)

- **Compose-only.** Mnemonic produces Intent/Action/Knowledge envelopes, anchors
  them, and re-verifies correspondence/zkTLS proofs; it does **not** build an
  Evidence Layer / zkTLS / zkVM prover. Rationale: plays to the open-source +
  permanence moat; rebuilding a funded competitor's closed, specialised stack is
  the trap.
- **Optional later wave — open reference prover.** Ship an open-source
  self-hosted SP1 guest program so users aren't forced through a closed hosted
  API. An open wedge against Delta's closed alpha; sequence after compose lands.
- **Rejected — full compete.** Building our own zkTLS + zkVM correspondence stack.
  Duplicative, deep specialty, no clear open wedge today.

## Honest caveats

- zkTLS and SP1 are real and production-grade, but the recommendation uses them
  **verify-side only** — no claim Mnemonic produces such proofs.
- AP2 is an *emerging* standard; align but track churn.
- The correspondence proof still inherits the evidence's trust model: zkTLS proves
  *the bytes came from that TLS endpoint*, not that the endpoint told the truth.
  State this whenever the word "verifiable" is used.

# Decisions — computation-proof

Append-only log. Each entry: date, who, what, why, downstream.

---

## 2026-06-29 — Feasibility memo created

Author: claude. Origin: thread proposing Gabbay-style (arXiv 2606.23768) SNARK
policy certificates on the Mnemonic envelope. Conclusion: buildable now; policy
circuits scale with rule complexity, not model size; slots into the
verifiable-trajectories "bind by hash, no prover in core/" decision as a new
proof_kind, adding one net-new primitive — a *verifier* in core/. See
`feasibility.md`.

---

## 2026-06-29 — Reframe: policy-compliance → intent–action correspondence

Author: claude. Trigger: competitive analysis of **Delta Network** (spender-side
settlement enforcement; zkTLS-native SDK; SP1 prover; rides AP2 Intent/Cart/
Payment Mandate model). Delta targets exactly the owner's goal — provable
correspondence between an agent's action and the principal's signed intent.

**What changes.** The feature's framing generalises from "prove action satisfies a
policy" to "prove action ⊨ the principal's signed Intent." The "policy" becomes a
principal-signed **`INTENT_V1`** mandate (aligned with AP2, not reinvented). See
`positioning.md` for the 5-layer decomposition and ownership table.

**Two earlier conclusions updated:**
- Oracle problem is answered by **zkTLS** (TLS transcript = attestation, no
  merchant integration). Supersedes the "wait for signed feeds" framing.
- The DSL→circuit path is now *one* option; a **zkVM (SP1)** — policy as a
  program, prove execution — is the rival, more pragmatic path for evolving logic.
  Mnemonic stays prover-agnostic and verifies whichever.

---

## 2026-06-29 — Decision: compose, do not compete (recommended; pending owner)

Mnemonic **owns layers 1 (Intent), 4 (binding/anchoring), 5 (knowledge link)**,
**verifies** layer 3 (correspondence proof) in core/ via new `proof_kind:
sp1 | zktls` alongside `snark`, and **does not build** layer 2 (Evidence/zkTLS) or
a zkVM prover.

Why: Mnemonic's defensible edges are open-source + permanent anchoring + the
knowledge layer Delta explicitly cedes. Rebuilding a funded competitor's closed,
specialised zkTLS+zkVM stack is the trap. Positioning: *Delta is the turnstile;
Mnemonic is the permanent, open, re-verifiable record that survives the
transaction.*

Optional later wave: an open-source self-hosted SP1 reference prover, so users
aren't forced through a closed hosted API. Full-compete rejected for now.

Downstream: extend `feasibility.md`'s envelope-binding design with `INTENT_V1`,
`intent_hash` linkage, public-input commitment to (intent_hash, action_hash,
evidence), and a `mnemonic_verify_correspondence` surface. The benchmark Delta
cites (28.8%→0%) is unverified — never repeat as fact.

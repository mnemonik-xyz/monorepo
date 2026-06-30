# Decisions — agent-provenance

Append-only log. Each entry: date, who, what, why, what changes downstream.

---

## 2026-06-30 — Feature folder created

Author: claude (in response to owner request to evolve Mnemonic toward
"cryptographically verifiable provenance of agent actions against the initial
intent/task," framed against Delta Network's thesis).

Origin: Delta Network proves an agent's *action* matches a user-signed *intent*
via a centralized closed-source SP1 zkVM + zkTLS, gating before funds move. The
owner wants Mnemonic to occupy the same problem — action↔intent correspondence —
but **decentralized, efficient, and as provenance not only a gate**.

Substrate audit found Mnemonic already ships 2 of the 3 legs (chain integrity +
per-step verdict, via `work/verifiable-trajectories/`). The missing leg is
**intent binding**. This feature extends — not replaces — that work.

---

## 2026-06-30 — Decision: correspondence is re-runnable policy eval, NOT a zkVM

The central architectural choice. Delta's correspondence proof is a hosted SP1
zkVM evaluation you must trust the operator to run. Mnemonic's is a **pure,
total, deterministic policy evaluator** (`core/src/trajectory/policy.rs`) over
**hash-committed evidence**. Any verifier re-executes it and gets the same
verdict — no trusted prover, no hosted service.

Why: it is strictly more decentralized than a hosted prover (re-runnable from the
permaweb), strictly cheaper (microseconds, no proving overhead), and it stays
inside the locked-in philosophy — Mnemonic *produces* the commitment/lineage/
correspondence layer and *binds* external correctness proofs by hash, never
produces them (mirrors verifiable-trajectories Decision "scope to A+B, bind C").

Consequence: zk is used ONLY where evidence must stay private — bound via
`COMPLIANCE_V1.proof_ref` / `proof_kind` (a Delta SP1 proof, a TEE quote, a zkTLS
notarization). Mnemonic orders + anchors + makes it recallable. Composition, not
competition: a Delta proof can be a leaf in a Mnemonic record.

---

## 2026-06-30 — Decision: the mandate is the genesis root (no breaking change)

The signed `MANDATE_V1` is the genesis of correspondence. The genesis step
(`seq == 0`) sets `prev_hash = mandate.content_hash` (instead of `null`) and
adds `parents: [{ artifact_id, role: "mandate" }]`. The whole hash-linked chain
is therefore tamper-evidently rooted in the intent: edit/swap the mandate and the
genesis link breaks under the *existing* `verify_chain` linkage rule.

Why this shape: `STEP_V1` needs **no new field** — `prev_hash` and `parents`
already exist, so we avoid a schema version bump on the step. Only `TRAJECTORY_V1`
gains one optional field (`mandate_hash`) so the anchored summary names the intent
it fulfills — permitted because trajectory schemas are experimental / pre-GA (not
frozen in verifiable-trajectories `decisions.md`). Mandate-less trajectories keep
today's `prev_hash == null` genesis behavior unchanged (backward compatible).

Downstream: `verify_chain` learns a mandate-aware genesis rule; a new
`verify_correspondence` does the intent-level checks.

---

## 2026-06-30 — Decision: principal ≠ subject ≠ checker (three roles, three keys)

- **principal** — signs the `MANDATE_V1` (the authority delegating the task).
- **subject** — the pubkey the principal authorizes to execute; every step's
  `producer` must equal it.
- **checker** — signs `COMPLIANCE_V1`; MUST differ from `subject` (SHOULD differ
  from `principal`).

Why: a mandate an agent writes for itself, or a compliance verdict the executing
agent signs over its own trajectory, is worthless as a correspondence signal —
the same "signed hallucination" failure the verdict-independence rule already
guards against (verifiable-trajectories Decision "judge ≠ producer"). This makes
delegation explicit and the authorization chain verifiable: principal → subject.

V1 keeps `subject` a single pubkey; multi-hop delegation chains are out of scope.

---

## 2026-06-30 — Decision: non-custodial for all three new artifacts

`MANDATE_V1` / `EVIDENCE_V1` / `COMPLIANCE_V1` follow the corrected trajectory
model: the client (principal / producer / checker) signs locally with its OWN
key and submits the COSE_Sign1 envelope; the server only **verifies and stores**,
signing nothing. Reuses `reconstruct.rs`'s `*_from_cose` pattern (verify sig,
take identity from COSE `kid`, parse the signed CBOR payload).

Why: the operator keypair must never sign content authored by a different
identity (hard rule, `mcp/src/tools.rs` routing + verifiable-trajectories
2026-06-27 correction). Principal/subject/checker identities are whatever keys
signed the envelopes.

---

## 2026-06-30 — Decision: `safe_to_settle` becomes intent-aware, stays backward compatible

When a mandate is present, `safe_to_settle` AND-extends the existing gate with
`rooted_in_mandate && authorized && capabilities_respected && all machine-
constraints satisfied && no unmet subjective constraints && compliant`. With no
mandate, the boolean is computed exactly as today.

Why: this is the decentralized analog of Delta's "Verifier approved the
proposal," but computed by any party offline. Keeping the mandate-less path
unchanged means the shipped verifiable-trajectories behavior and golden vectors
are untouched.

---

## 2026-06-30 — Open questions (for owner)

1. **Capability evidence source.** Where does a step declare its action (tool +
   spend) for capability-conformance — a reserved `metadata.action` shape on
   `STEP_V1`, or a dedicated `EVIDENCE_V1` kind? Leaning `metadata.action`
   (no schema bump) + optional `EVIDENCE_V1` for the external-data proof.
2. **Constraint DSL scope.** V1 predicate set is deliberately tiny (number/
   string/bool/set/exists + bounded regex) to stay total + re-runnable. Confirm
   that covers the near-term use cases (commerce-style: price/size/qty/allowlist)
   before anyone reaches for a richer DSL.
3. **GA gate.** Do mandate-bound trajectories ship under the *same*
   `trajectory-experimental` feature, or a new `provenance-experimental` gate so
   the two can GA independently? Leaning same gate (they're one story).

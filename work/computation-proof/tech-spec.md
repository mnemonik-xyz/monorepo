---
created: 2026-06-30
updated: 2026-06-30
status: draft
type: feature
size: XL
feature_flag: correspondence-experimental
depends_on: [verifiable-trajectories]
---

# Tech Spec: Intent–Action Correspondence (full compete)

## Goal

Let a verifier prove, offline and without re-execution, that an agent's **action
matched the principal's signed intent**, given authenticated real-world evidence.
Mnemonic **produces** the full stack — Intent envelope, Evidence (zkTLS),
correspondence proof (zkVM), binding/anchoring, and the knowledge link — as the
**open-source, AP2-aligned, permanently-anchored** alternative to Delta's closed
hosted enforcement. Decision basis: `decisions.md` (2026-06-30, full compete);
landscape: `positioning.md`; primitives: `feasibility.md`.

## Hard architectural constraint

The **prover lives in a new workspace member `prover/` (`mnemonic-prover`)**.
`core/` stays native-only, pure, and **verify-only** — the "no prover in `core/`"
rule and one-way `core → mcp` dependency both hold. `mcp/` orchestrates:
`prover/` produces → `core/` verifies + binds + anchors. The lightweight open
verifier in `core/` remains the moat even as we add production.

Everything ships behind cargo feature `correspondence-experimental`; default
`cargo build --workspace` stays green and dependency-free (mirrors
`trajectory-experimental`). Non-custodial throughout: **the server signs nothing**
— the principal signs the Intent, the agent signs the Action, the prover emits π.

## Layer ownership (from positioning.md)

| Layer | Produced by | Verified in |
|---|---|---|
| 1 Intent (signed mandate) | principal (client) | `core/` sig+anchor |
| 2 Evidence authenticity (zkTLS) | `prover/` `EvidenceSource` | `core/` re-verify |
| 3 Correspondence proof (zkVM) | `prover/` `Prover` | `core/` re-verify |
| 4 Binding / anchoring | `core/` + `mcp/` | n/a (is the record) |
| 5 Knowledge link | agent (refs memory hashes) | `core/` sig+hash |

## Schemas (`core/src/codec/schema.rs`, gated)

Add following the existing `cbor_field_order` discipline; **do not mutate**
shipped schemas.

- **`INTENT_V1`** (`type: "intent"`): required `artifact_id, type,
  schema_version, constraints, producer, created_at`; optional `expiry, nonce,
  metadata, tags`. `producer` = principal `did:sol`. `constraints` carries the
  **AP2-aligned mandate** (typed limits, allowlist roots, a `policy_id` naming the
  guest program + its params hash). `intent_hash = blake3(canonical_cbor)`.
- **`ACTION_V1`** (`type: "action"`): required `artifact_id, type,
  schema_version, content, producer, created_at, intent_ref`; optional
  `knowledge_refs` (hashes of retrieved memories), `metadata` (carries the
  correspondence cert), `tags`. `intent_ref == INTENT_V1.content_hash`.
- **Correspondence certificate** (nested in `ACTION_V1.metadata.correspondence`):
  `{ intent_hash, action_commitment, evidence_commitment, policy_id, params_hash,
  public_inputs, proof_kind: "sp1" | "snark" | "zktls", proof_ref, backend }`.
  `proof_ref` = blake3 of π; π itself stored on Arweave (bind-by-hash). Reuses the
  feasibility-memo binding + the `proof_kind` precedent from `VERDICT_V1`.

### Circularity fix (from feasibility.md)

`action_commitment = blake3(canonical_cbor_of(content, producer, created_at,
intent_ref, knowledge_refs))` — the pre-cert fields. π's public inputs commit to
`(intent_hash, action_commitment, evidence_commitment)`. Then canonicalize → blake3
→ COSE-sign the whole `ACTION_V1` including the cert. No fixed-point.

## `core/` — verify only (`core/src/correspondence/`)

```
core/src/correspondence/mod.rs   # CorrespondenceVerifier trait, result types
core/src/correspondence/verify.rs# verify_correspondence(...) orchestration
core/src/correspondence/sp1.rs   # SP1 proof re-verify (feature corr-sp1)
core/src/correspondence/mock.rs  # #[cfg(test)] MockVerifier
```

`verify_correspondence(intent, action, evidence_verifier) ->
CorrespondenceVerification` with per-check tri-states (same `Option<bool>`
convention as `chain_valid`):

1. `intent_sig` — Ed25519/COSE over `INTENT_V1` valid; anchor resolvable.
2. `action_sig` — Ed25519/COSE over `ACTION_V1` valid; anchor resolvable.
3. `intent_link` — `action.intent_ref == intent.content_hash`; intent not expired.
4. `correspondence_proof` — recompute `action_commitment` from action fields,
   assert it equals the cert's public input, fetch verifying key for `policy_id`,
   run `backend.verify(vk, public_inputs, π)`.
5. `evidence_proof` — re-verify the zkTLS/evidence attestation against
   `evidence_commitment`.

`safe = all Some(true)`. **Honesty rule:** zkTLS proves *bytes came from that TLS
endpoint*, not that the endpoint told the truth — surface this in the result docs.

## `prover/` — new workspace member (`mnemonic-prover`, native)

```
prover/src/lib.rs
prover/src/evidence/mod.rs   # EvidenceSource trait
prover/src/evidence/stub.rs  # StubEvidence (trusted; Wave 1)
prover/src/evidence/tlsn.rs  # TlsNotaryEvidence (Wave 3)
prover/src/prove/mod.rs      # Prover trait
prover/src/prove/mock.rs     # MockProver (Wave 1)
prover/src/prove/sp1.rs      # Sp1Prover (Wave 2)
prover/guests/<policy>/      # Rust zkVM guest programs (evidence ⊨ intent)
```

- `EvidenceSource::collect(action, intent) -> (Evidence, EvidenceCommitment,
  EvidenceProof)`. Stub returns a trusted blob + identity proof; TLSNotary returns
  a TLS-transcript attestation. The trait is the seam so the stack runs
  end-to-end before notary ops exist.
- `Prover::prove(intent, action, evidence) -> Certificate` runs the guest program
  proving the intent constraints hold over (action, evidence); public inputs =
  `(intent_hash, action_commitment, evidence_commitment)`.
- `prover/` depends on `core/` for canonicalization/hashing only (one-way intact);
  `core/` never depends on `prover/`.

## `mcp/` orchestration (`mcp/src/`, gated tools)

- `mnemonic_prove_correspondence { intent, action }` — collect evidence → prove →
  embed cert in `ACTION_V1.metadata` → client signs → anchor (Arweave bytes +
  Solana memo `v:5` adds `p: policy_id`). Returns the composite Proof object.
- `mnemonic_verify_correspondence { action }` — load intent + action (+ π from
  Arweave) → `core::correspondence::verify_correspondence` → tri-state report.

## Data flow

1. Principal client-signs `INTENT_V1`; anchor. `intent_hash` published.
2. Agent prepares action; `EvidenceSource` collects + attests external facts.
3. `Prover` emits π (public inputs bind intent_hash + action_commitment +
   evidence_commitment).
4. `core/` binds cert into `ACTION_V1`; agent client-signs; anchor.
5. Any verifier runs the 5 checks above — no re-execution, no private witness.

## Waves

- **Wave 1 — thin slice, no real crypto.** `INTENT_V1`/`ACTION_V1` schemas;
  `action_commitment` + circularity; `MockProver` + `StubEvidence`;
  `verify_correspondence` with `MockVerifier`; `mnemonic_prove/verify_correspondence`
  tools. End-to-end intent→action→proof→verify→anchor on a trivial policy. Default
  build stays green.
- **Wave 2 — real zkVM.** `prover/` `Sp1Prover` + one guest program (spending-cap
  or allowlist mandate); `core/` `corr-sp1` re-verify of the wrapped Groth16.
- **Wave 3 — real zkTLS.** `TlsNotaryEvidence` behind `EvidenceSource`; evidence
  re-verify in `core/`. **Operational risk concentrated here** (notary/TEE ops).
  Partial-compete fallback available (bind external zkTLS) per decisions.md.
- **Wave 4 — knowledge link + composite.** Bind retrieved memory hashes into the
  witness/commitment; finalize the composite Proof object; AP2 Intent hardening.
- **Wave 5 — audit + conformance.** Golden vectors `{intent, action, cert,
  public_inputs, π, vk}`; threat model (notary trust, evidence honesty); QA gate.

## Testing

- Golden vectors frozen for byte-parity (`references/conformance.md`).
- Property tests: tamper intent ⇒ `intent_link=false`; swap π ⇒
  `correspondence_proof=false`; wrong evidence ⇒ `evidence_proof=false`; expired
  intent ⇒ rejected; server-signed anything ⇒ test fails (non-custodial guard).
- `cargo build --workspace` (no feature) green — proves gating.

## Out of scope (V1)

zkML inference proofs (layer C — `proof_kind: zkml` bind-only); semantic policies;
production notary-ops hardening (Wave 3 integrates TLSNotary; HA/ops deferred);
on-chain settlement enforcement (Mnemonic produces the record, not the turnstile).

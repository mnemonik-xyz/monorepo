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
correspondence proof (our own **zigz** zkVM), binding/anchoring, and the
knowledge link — as the
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
  public_inputs, proof_kind: "zigz" | "snark" | "zktls", proof_ref, backend }`.
  `proof_ref` = blake3 of π; π itself stored on Arweave (bind-by-hash). Reuses the
  feasibility-memo binding + the `proof_kind` precedent from `VERDICT_V1`.

### Circularity fix (from feasibility.md)

`action_commitment = blake3(canonical_cbor_of(content, producer, created_at,
intent_ref, knowledge_refs))` — the pre-cert fields. π's public inputs commit to
`(intent_hash, action_commitment, evidence_commitment)`. Then canonicalize → blake3
→ COSE-sign the whole `ACTION_V1` including the cert. No fixed-point.

## Prover backend: zigz (own zkVM)

The correspondence proof is produced by **zigz** (`mnemonik-dev/zigz`) — our own
Jolt-inspired Zig zkVM: sumcheck + Lasso lookups, **Binary Merkle commitments →
transparent (no trusted setup), post-quantum**, proving **RISC-V RV64IM**
execution. The policy is a RISC-V guest program; `zigz prove` emits π; public
inputs bind `(intent_hash, action_commitment, evidence_commitment)` via zigz's
existing public-input-to-transcript binding.

**Why zigz over SP1/Groth16.** It is ours and open (the whole stack is then
open — the point of "compete"); transparent setup **eliminates the Groth16
trusted-setup concern** from `feasibility.md`; zigz already ships the
Fiat-Shamir "unfaithful-claims" hardening (Jolt PR #981 / osec.io 2026-03).

**Verifier path = pure-Rust re-implementation (option C).** `core/` does NOT FFI
into Zig or shell out to the `zigz` binary; it carries a **pure-Rust verifier**
for zigz proofs. Rationale:
- **Embeddable moat.** A pure-Rust verifier compiles to **WASM / browser**;
  FFI-to-Zig and CLI-shell-out cannot. Client-side verification is a stated
  direction (`verifiable-trajectories` wanted the verifier pure for wasm).
- **Differential security.** An independent Rust verifier that must agree with
  the Zig prover gives cross-implementation testing — the exact discipline that
  catches Fiat-Shamir/transcript bugs.
- **No non-Rust build/runtime dependency** in the pure `core/`.

**Format-freeze discipline (the one real cost).** zigz is experimental, so we
**freeze a versioned `zigz-proof-v1` serialization** (field = BabyBear, transcript
binding order, Merkle/commitment encoding) the Rust verifier targets. CI carries
**differential conformance vectors** `{program, public_inputs, π}`: the Zig
verifier and the Rust verifier MUST agree (accept/reject) on every vector; any
divergence fails the build. The two implementations move in lockstep, gated here.

**Tradeoffs (on record).** Hash/Merkle proofs are KB–MB (no ~200 B Groth16
wrapper) and on-chain (Solana) verification is impractical → we **anchor π on
Arweave and verify off-chain** (already the design). zigz is **unaudited** →
stays behind `correspondence-experimental`; never claim production.

## `core/` — verify only (`core/src/correspondence/`)

```
core/src/correspondence/mod.rs   # CorrespondenceVerifier trait, result types
core/src/correspondence/verify.rs# verify_correspondence(...) orchestration
core/src/correspondence/zigz.rs  # pure-Rust zigz-proof re-verifier (feature corr-zigz)
core/src/correspondence/mock.rs  # #[cfg(test)] MockVerifier
```

`verify_correspondence(intent, action, evidence_verifier) ->
CorrespondenceVerification` with per-check tri-states (same `Option<bool>`
convention as `chain_valid`):

1. `intent_sig` — Ed25519/COSE over `INTENT_V1` valid; anchor resolvable.
2. `action_sig` — Ed25519/COSE over `ACTION_V1` valid; anchor resolvable.
3. `intent_link` — `action.intent_ref == intent.content_hash`; intent not expired.
4. `correspondence_proof` — recompute `action_commitment` from action fields,
   assert it equals the cert's public input, then run the **pure-Rust zigz
   verifier** over `(program_hash[policy_id], public_inputs, π)`. Transparent —
   no verifying key / trusted setup. Must accept iff the Zig verifier accepts
   (differential conformance).
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
prover/src/prove/zigz.rs     # ZigzProver — drives zigz prove (Wave 2)
prover/guests/<policy>/      # RISC-V guest programs (Rust or Zig via `zigz build`)
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
- **Wave 2 — real zkVM (zigz).** `prover/` `ZigzProver` + one RISC-V guest
  (spending-cap or allowlist mandate); **freeze `zigz-proof-v1`**; `core/`
  `corr-zigz` **pure-Rust** re-verifier; differential conformance vectors
  asserting Zig-prover ↔ Rust-verifier agreement in CI.
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

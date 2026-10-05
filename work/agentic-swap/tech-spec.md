---
created: 2026-10-05
updated: 2026-10-05
status: draft
type: feature
size: M
---

# Tech Spec: Generic signed sealed messages and key custody (Mnemonik side of agentic swap)

Status: **planned**. Gaps and reasons are in [gap-analysis.md](gap-analysis.md).
The swap protocol itself is in the
[Warrant swap specification](https://github.com/mnemonik-xyz/warrant/blob/main/swap/spec.md).

## Goal

Add two generic capabilities and one profile to Mnemonik. The agentic swap needs
them. Any other integration that negotiates or settles through Mnemonik can use
them too.

1. **Sealed A2A profile.** Sign, seal, sign exists since PR #276
   (`A2aInnerBinding`, `mnemonic.a2a.signed.v2`). This item documents the payload
   fields `protocol`, `nonce` and `expires_at` and the open checks. It adds no new
   signed format.
2. **Generic COSE verification in WASM.** TypeScript can sign generic COSE today
   (`sign_cose_payload`), but it cannot verify a standalone COSE signature.
3. **Key-custody `Signer`.** A signing interface for an Ed25519 key that stays in
   an HSM, a KMS or a TEE. Today `sign_cose` and `sign_artifact` take a concrete
   `Keypair`.

## Integration boundary

This rule decides what enters `mnemonic-core`:

- Mnemonik provides transport, identity, sealing, signing and anchoring. These
  parts know nothing about any one application.
- An integration defines its own message types, state machines and policies in
  its own package. It uses Mnemonik through the public Rust API or the SDK.
- A new integration therefore needs no change to `mnemonic-core`. A change enters
  core only when it is generic and at least one integration needs it.

For the agentic swap, the message kinds (RFQ, QUOTE, COUNTER, ACCEPT, REJECT,
EXPIRE), the swap intent and the transcript state machine are integration code.
They live in the Warrant swap specification (`swap-core`), not here.

## Design

### 1. Sealed A2A V2 profile

Sign, seal, sign is available now (PR #276). Send with `prepare_signed_a2a` and
recipient cards. Open with `open_signed_a2a_full` (SDK: `openA2AAttestation`,
which returns `innerSigned`; `verifyA2AInner` checks it). The inner binding
(`A2aInnerBinding`, `mnemonic.a2a.inner.v1`) signs the author, the sorted
recipients, the kind, `context_id`, `prev_id`, `created_at` and the payload.

The inner binding has no nonce, no expiry and no application protocol tag. Do
not add fields to `A2aInnerBinding`: it uses `deny_unknown_fields` and a strict
JCS check, so a new field needs a new inner protocol version, and old readers
reject it. Do not add a second inner format. The integration puts these values
in the signed A2A payload, for example in `metadata`:

```text
protocol:   ASCII, 1 to 128 bytes, e.g. "warrant.swap.negotiation.v1"
nonce:      hex, 16 random bytes
expires_at: RFC 3339
```

**Open checks.** After `open_signed_a2a_full`, the integration, or one SDK helper,
checks these items in order:

1. `inner_signed` is present. Plain and legacy V1 sealed envelopes open with no
   inner signature, so they are rejected.
2. The inner `recipients` equal the expected set exactly.
3. `prev_id` equals the session head.
4. Payload `protocol` equals the expected tag.
5. Now is before payload `expires_at`, and `created_at` is within the skew limit.
6. The payload `nonce` is new for (author, `context_id`). The nonce store records
   it only after every other check passes.

**Identity encoding.** The inner binding stores `author` and `recipients` as bare
base58 Ed25519 keys, and the code compares these strings. Each grant reader must
decode to exactly 32 bytes. `did:key` is a display and policy form only.

**Anchoring.** Each sealed message is anchored with the existing
`mnemonic_attest_a2a`. The server keeps receipts only.

### 2. Generic COSE verification in WASM

Add `verify_cose_payload(cose_bytes) -> { payload, kid }` to `core/src/wasm/mod.rs`.
It verifies an Ed25519 COSE_Sign1 signature against the key in `kid` and returns
the payload bytes and the base58 key. It reuses `verify_artifact` in
`core/src/codec/sign.rs`. It applies no schema check: the caller checks the payload.
The SDK wraps it as `verifyCosePayload` next to `coseSignPayload`
(`packages/sdk/src/cose.ts`).

### 3. Key-custody `Signer`

Add a `Signer` trait to `core/src/identity/`:

```text
trait Signer {
  fn public_key(&self) -> [u8; 32];               // Ed25519
  fn sign(&self, message: &[u8]) -> Result<[u8; 64]>;
}
```

- `LocalSigner` wraps today's `Keypair`. Existing behaviour does not change.
- `sign_cose_with(payload, &dyn Signer)` and `sign_artifact_with(…, &dyn Signer)`
  are added next to the current functions. The current functions call them with
  a `LocalSigner`.
- HSM, KMS and TEE adapters live outside core. They implement `Signer`.

**Relation to `work/multi-suite-signing/`.** That folder also plans a `Signer`
trait, as the last step of `work/DECOUPLING-SEQUENCE.md`, because a change to
`alg` or `kid` is unrecoverable. This task does not start that work:

- the algorithm stays Ed25519, and the protected header stays `alg = EdDSA`;
- the `kid` stays the base58 Ed25519 key;
- every fixture in `core/tests/golden_fixtures.rs` and
  `core/tests/a2a_golden_fixtures.rs` stays byte-identical.

Multi-suite signing can later extend this trait. It does not have to replace it.

### Code placement

- `core/src/identity/signer.rs`: `Signer`, `LocalSigner`.
- `core/src/codec/sign.rs`: `sign_cose_with`, `sign_artifact_with`.
- `core/src/wasm/mod.rs`: `verify_cose_payload`.
- `packages/sdk/src/a2a.ts`: one optional helper for the profile checks, with a
  nonce store. `packages/sdk/src/cose.ts`: `verifyCosePayload`.
- Docs: an "Integration profile" section in `docs/sealed-a2a.md`; the SDK README.

Nothing in this list names a swap.

## Tests

- One negative test per profile check: missing `inner_signed`, a different
  recipient set, a `prev_id` that is not the head, a wrong `protocol`, an expired
  message, a replayed nonce.
- `Signer`: `LocalSigner` output is byte-identical to `sign_cose`; all existing
  golden fixtures still verify byte for byte.
- An end-to-end test with a dummy protocol (`test.echo.v1`) proves that the
  profile needs no knowledge of the protocol.

The sealed A2A V2 vectors already exist (`core/tests/sealed_a2a_vectors.rs`,
`packages/sdk/test/fixtures/sealed-a2a.json`).

## Tasks

Task files: [`tasks/`](tasks/).

| # | Task | Wave | Depends on |
|---|---|---|---|
| 1 | ~~Freeze `SIGNED_INNER_V1`~~ — cancelled by PR #276 | — | — |
| 2 | ~~`inner.rs` build, sign, verify~~ — cancelled by PR #276 | — | — |
| 3 | `Signer` trait and `LocalSigner`; `sign_cose_with` | 1 | — |
| 4 | Profile checks helper and nonce store (SDK) | 1 | — |
| 5 | `verify_cose_payload` in WASM and the SDK | 2 | 3 |
| 6 | End-to-end test with a dummy protocol; profile docs | 2 | 4 |

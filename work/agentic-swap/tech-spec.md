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

Add three generic capabilities to Mnemonik. The agentic swap needs them. Any
other integration that negotiates or settles through Mnemonik can use them too.

1. **Signed sealed message.** A sender signs a plaintext payload, seals it for a
   recipient and signs the sealed bytes. The plaintext signature binds a protocol
   tag, the session, the previous message, a nonce and the recipient. Either party
   can later prove the plaintext to a third party.
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

### 1. Signed sealed message

**Construction: sign, seal, sign.** No cryptographic change is needed. The design
composes existing parts of `core/src/codec/a2a/signed.rs` and `core/src/sealed/`:

1. **Inner sign.** The sender builds `SIGNED_INNER_V1` (below) as JCS and signs it
   with `sign_cose`.
2. **Seal.** The inner COSE bytes go into an A2A message as one DataPart with media
   type `application/vnd.mnemonic.signed-inner+cose`. `prepare_signed_a2a` seals
   the message and makes a grant for each recipient.
3. **Outer sign.** The existing A2A binding signature covers the sealed bytes,
   `context_id` and `prev_id`.

**`SIGNED_INNER_V1`:**

```text
{
  protocol:   string    caller-chosen domain tag, e.g. "warrant.swap.negotiation.v1"
  session_id: string    equals the A2A context_id
  prev_hash:  hex blake3 of the previous inner JCS in this session, or null
  nonce:      hex, 16 random bytes
  created_at: RFC 3339
  expires_at: RFC 3339
  sender:     base58 Ed25519 public key (same form as the COSE kid)
  recipient:  base58 Ed25519 public key (same form as a grant reader)
  body:       JSON object, defined by the protocol; opaque to Mnemonik
}
```

The protected COSE header has no domain field (`codec/sign.rs`). The `protocol`
field therefore carries the domain. The caller passes the expected `protocol`
value to the open function, and the open function rejects any other value.

**Identity encoding.** `sender` and `recipient` use the bare base58 Ed25519 public
key. This is the form that `sign_cose` writes into the COSE `kid` and that sealed
A2A grants store as `reader`. Every identity comparison decodes both sides to the
raw 32-byte key and compares the bytes. A value that does not decode to exactly 32
bytes is rejected. `did:key` (`identity::did_key_from_pubkey`) is a display and
policy form only.

**Open and verify.** `open_signed_inner` runs these checks in order. Any failure
rejects the message:

1. Verify the A2A binding and the grant (existing code).
2. Open the sealed A2A message and extract exactly one inner DataPart.
3. Verify the inner COSE signature.
4. `protocol` equals the value that the caller expects.
5. Inner `sender` equals the outer binding signer and the inner COSE `kid`
   (32-byte key comparison).
6. Inner `recipient` equals the local identity key and one grant `reader`
   (32-byte key comparison).
7. Inner `session_id` equals the binding `context_id`.
8. `prev_hash` equals the session head that the caller supplies.
9. `nonce` is new for (sender, session). A `NonceStore` that the caller supplies
   records it only after every other check passes.
10. `created_at` is not in the future beyond a skew limit, and now is before
    `expires_at`.

The function returns the verified inner payload, its JCS hash and the exact inner
COSE bytes. The caller then applies its own protocol rules to `body`. For the
swap, that is the transcript state machine of the Warrant specification.

**Anchoring.** Each sealed message is anchored with the existing
`mnemonic_attest_a2a` (`kind: "message"`, `prev_id` set). The server keeps
receipts only. No new MCP tool and no new ingestion adapter are needed.

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

- `core/src/codec/a2a/inner.rs`: `SIGNED_INNER_V1`, `sign_inner`, `verify_inner`,
  `seal_signed_inner`, `open_signed_inner`, `NonceStore`. Behind the existing
  `a2a-experimental` feature.
- `core/src/identity/signer.rs`: `Signer`, `LocalSigner`.
- `core/src/codec/sign.rs`: `sign_cose_with`, `sign_artifact_with`.
- `core/src/wasm/mod.rs`: `a2a_seal_signed_inner`, `a2a_open_signed_inner`,
  `verify_cose_payload`.
- `packages/sdk/src/a2a.ts` and `cose.ts`: TypeScript wrappers and a nonce store.
- Docs to update with the code: `docs/sealed-a2a.md`, the SDK README.

Nothing in this list names a swap.

## Tests

- Golden vectors for `SIGNED_INNER_V1`, Rust and TypeScript byte parity (the
  `a2a_golden_fixtures` pattern).
- Negative tests, one per open check: forwarded message (wrong recipient),
  replayed nonce, cross-session replay, inner and outer signer mismatch, wrong
  `protocol`, `prev_hash` mismatch, two inner DataParts, expired message.
- `Signer`: `LocalSigner` output is byte-identical to `sign_cose`; all existing
  golden fixtures still verify byte for byte.
- An integration test with a dummy protocol (`test.echo.v1`) proves that the
  helper needs no knowledge of the protocol.

## Tasks

Task files: [`tasks/`](tasks/).

| # | Task | Wave | Depends on |
|---|---|---|---|
| 1 | Freeze `SIGNED_INNER_V1` and add the module skeleton | 1 | — |
| 2 | `inner.rs`: build, sign, verify | 2 | 1 |
| 3 | `Signer` trait and `LocalSigner`; `sign_cose_with` | 2 | — |
| 4 | Seal and open over sealed A2A; checks 1 to 10; `NonceStore` | 3 | 2 |
| 5 | WASM exports and SDK wrappers | 4 | 3, 4 |
| 6 | Golden vectors, negative tests, docs | 4 | 4 |

Tasks 2 and 3 can run in parallel: task 2 touches `codec/a2a/`, task 3 touches
`identity/` and `codec/sign.rs`. Task 5 touches `core/src/wasm/mod.rs`, a common
conflict point.

# A2A Conformance Vectors

Reference for `@mnemonik-xyz/conformance` — the portable fixture package that
third-party A2A attestation implementations use to prove byte-for-byte parity
with Mnemonic's serialization pipeline.

---

## How to use the fixtures

Install the package:

```sh
npm install @mnemonik-xyz/conformance
```

Load the vectors array:

```ts
import { loadVectors, vectorsForKind } from "@mnemonik-xyz/conformance";

const all = loadVectors();                      // all 22+ entries
const messages = vectorsForKind("message");     // message_* subset
const tasks    = vectorsForKind("task");        // task_* subset
const artifacts = vectorsForKind("artifact");   // artifact_* subset
```

The vectors file is also importable directly as JSON:

```ts
import vectors from "@mnemonik-xyz/conformance/vectors";
```

---

## Schema — one vector entry

| Field | Type | Meaning |
|---|---|---|
| `name` | `string` | Stable, human-readable label for the test case. |
| `a2a_json` | `string` | The A2A object as regular JSON. **Documentation only** — consume `jcs_canonical_hex` for byte-exact checks. |
| `jcs_canonical_hex` | `string` | Hex of the RFC 8785 JCS canonical bytes (`to_jcs_bytes()`). This is the authoritative byte sequence: a conforming implementation must produce the same hex for the same object. |
| `cbor_envelope_hex` | `string` | Hex of the CBOR envelope. **In V1 this equals `jcs_canonical_hex`** — the inner payload is the JCS bytes verbatim (Decision 1, see below). Kept separate for transparency and future use. |
| `cose_signed_hex` | `string` | Hex of the COSE_Sign1 envelope (`to_canonical_envelope(keypair, None)`). Third-party signers using the same keypair must produce the exact same bytes (Ed25519 is deterministic per RFC 8032). |
| `keypair_secret_hex` | `string` | Hex of the 64-byte Ed25519 keypair secret (`seed ++ pubkey`, as returned by `Keypair::to_bytes()`). All entries share the same fixed test seed. **TEST-ONLY — never use this key in production.** |

---

## Fixed test seed

All vectors are signed with:

```
seed (32 bytes): 0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20
```

This is a standard constant for test fixtures. The resulting keypair is deterministic — running the emitter twice with the same seed produces byte-identical output.

---

## Schema-compatibility matrix

| A2A protocol version | Mnemonic schema version | Status |
|---|---|---|
| v1.0.0-rc | `a2a-experimental` feature | Experimental — schema can change before GA |
| v1.0.0 GA | `A2A_*_V1` (locked) | Stable — vectors locked; regeneration requires a version bump |

Until A2A v1.0.0 GA the codec lives behind the `a2a-experimental` cargo feature (Decision: schema lock waits for v1.0.0 GA). The vectors file is named `vectors-a2a-v1.0.0.json` to track the A2A protocol version, not the Mnemonic SDK version.

---

## How to regenerate vectors

Run the golden fixture emitter in `core/tests/a2a_golden_fixtures.rs`:

```sh
cargo test --features "golden-fixtures a2a-experimental" -p mnemonic-core \
  --test a2a_golden_fixtures emit_fixtures -- --ignored --nocapture \
  > packages/conformance/vectors-a2a-v1.0.0.json
```

Verify determinism:

```sh
# Run twice and diff — must produce no output.
cargo test --features "golden-fixtures a2a-experimental" -p mnemonic-core \
  --test a2a_golden_fixtures emit_fixtures -- --ignored --nocapture > /tmp/v1.json
cargo test --features "golden-fixtures a2a-experimental" -p mnemonic-core \
  --test a2a_golden_fixtures emit_fixtures -- --ignored --nocapture > /tmp/v2.json
diff /tmp/v1.json /tmp/v2.json
```

The emitter's internal `test_emitter_deterministic` test asserts this in CI:

```sh
cargo test --features "golden-fixtures a2a-experimental" -p mnemonic-core \
  --test a2a_golden_fixtures test_emitter_deterministic
```

---

## What byte-for-byte parity proves

**Proves:**
- The implementation applies RFC 8785 JCS canonicalization correctly (key order, number serialization, Unicode escaping).
- The CBOR envelope wraps the JCS bytes verbatim — no re-canonicalization (Decision 1).
- The COSE_Sign1 structure matches: same algorithm identifier (`EdDSA`, COSE alg -8), same protected header encoding, same signature produced from the same keypair over the same payload.

**Does NOT prove:**
- Semantic correctness of the A2A object content (the vectors use fixed fixture strings, not real agent output).
- Schema evolution safety (a future A2A protocol revision may require new fields; vectors must be regenerated at each breaking A2A version).
- Cross-keypair portability (passing these vectors only proves your implementation is compatible with this specific signing key; a production conformance run should also use a freshly generated keypair and verify the full pipeline end-to-end).

---

## CI publish recipe

Vectors are published on every tagged release:

```yaml
# .github/workflows/publish.yml (excerpt)
- name: Publish conformance vectors
  run: npm publish --workspace @mnemonik-xyz/conformance
  env:
    NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
```

Tags follow the A2A protocol version: `vectors-a2a-v1.0.0`, `vectors-a2a-v1.1.0`, etc.

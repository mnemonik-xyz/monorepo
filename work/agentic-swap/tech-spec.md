---
created: 2026-10-05
updated: 2026-10-05
status: draft
type: feature
size: M
---

# Tech Spec: Agentic swap — sealed negotiation (Mnemonik side)

Status: **planned**. This spec covers phase 1 of the agentic swap: the sealed
negotiation channel. Authorization, chain checks and the policy signer are in
the [Warrant swap specification](https://github.com/mnemonik-xyz/warrant/blob/main/swap/spec.md).
Gaps and reasons are in [gap-analysis.md](gap-analysis.md).

## Goal

Two agents negotiate swap terms over sealed A2A. Each message carries a
signature over its plaintext. The signature binds the session, the previous
message, a nonce and the recipient. Either party can later prove the agreed
terms to a third party without help from the other party.

## Design

### Construction: sign, seal, sign

No cryptographic change is needed. The design composes existing parts:

1. **Inner sign.** The sender builds `AGSWAP_INNER_V1` (below) as JCS and signs
   it with `sign_cose` (Ed25519 identity key).
2. **Seal.** The inner COSE bytes go into an A2A message as a DataPart with media
   type `application/vnd.mnemonic.agswap.inner+cose`. The existing sealed A2A
   path (`core/src/codec/a2a/signed.rs`) seals the message and makes a grant for
   the recipient.
3. **Outer sign.** The existing A2A binding signature covers the sealed bytes,
   `context_id` and `prev_id`.

### `AGSWAP_INNER_V1`

```text
{
  protocol:   "mnemonic.agswap.inner.v1"   domain separator inside the payload
  kind:       "rfq" | "quote" | "counter" | "accept" | "reject" | "expire"
  session_id: string                        equals the A2A context_id
  prev_hash:  hex blake3 of the previous inner JCS, or null for the first message
  nonce:      hex, 16 random bytes
  created_at: RFC 3339
  expires_at: RFC 3339
  sender:     did:key of the sender
  recipient:  did:key of the recipient
  payload:    object, by kind (below)
}
```

The protected COSE header has no domain field (`codec/sign.rs`). The `protocol`
field therefore carries the domain. A verifier rejects any other value.

Payloads:

| Kind | Payload |
|---|---|
| `rfq` | `intent`: give leg and take leg without price; hash function `sha256`; settlement venue |
| `quote`, `counter` | `intent_id`, full terms: both legs (CAIP-2, CAIP-19, CAIP-10, base-unit amounts), proposed timelocks, `valid_until` |
| `accept` | `intent_id`, `terms_hash = blake3(JCS(terms))`, hash of the accepted quote or counter |
| `reject` | `intent_id`, fixed reason code |
| `expire` | `intent_id` |

`intent_id = blake3(JCS(terms))` for the terms in the message. Amounts are
integers in base units. Free text is not part of the terms.

### Open and verify

`open_negotiation_message` runs these checks in order. Any failure rejects the
message:

1. Verify the A2A binding and the grant (existing code).
2. Open the sealed A2A message and extract exactly one inner DataPart.
3. Verify the inner COSE signature.
4. Inner `sender` key equals the outer binding signer.
5. Inner `recipient` equals the local identity and is a grant reader.
6. Inner `session_id` equals the binding `context_id`.
7. `prev_hash` equals the head of the local transcript for this session.
8. `nonce` is new for (sender, session). The client nonce store records it.
9. `created_at` is not in the future beyond a skew limit, and now is before
   `expires_at`.
10. The transcript state machine accepts `kind` (below).

### Transcript state machine

```text
rfq → quote → (counter)* → accept | reject | expire
```

Parties alternate after `rfq`. `accept` must reference the hash of the last
`quote` or `counter`. `accept`, `reject` and `expire` are terminal. The
transcript hash is the blake3 hash of the terminal inner JCS. The `prev_hash`
chain covers every earlier message.

### Anchoring

Each sealed message is anchored with the existing `mnemonic_attest_a2a`
(`kind: "message"`, `prev_id` set). The server keeps receipts only. No new MCP
tool and no new ingestion adapter are needed in phase 1.

### Code placement

- `core/src/codec/agswap/{mod.rs, inner.rs, intent.rs, transcript.rs}` behind a
  new feature `agswap-experimental`, which enables `a2a-experimental`.
- WASM wrappers in `core/src/wasm/mod.rs`: `agswap_seal`, `agswap_open`,
  `agswap_verify_transcript`.
- TypeScript: `packages/sdk/src/agswap.ts` and a nonce store next to the
  existing local A2A state.
- Docs to update with the code: `docs/sealed-a2a.md`, the SDK README.

## Tests

- Golden vectors for `AGSWAP_INNER_V1`, Rust and TypeScript byte parity (the
  `golden_fixtures` pattern).
- Negative tests, one per check above: forwarded message (wrong recipient),
  replayed nonce, cross-session replay, inner and outer signer mismatch,
  reordered transcript, `accept` of a stale quote, expired message, wrong
  `protocol` value, two inner DataParts.
- End-to-end: two local stdio agents complete `rfq → accept` and anchor every
  message. No funds move.

## Tasks

| # | Task | Wave | Depends on |
|---|---|---|---|
| 1 | Freeze `AGSWAP_INNER_V1`, payload schemas and the CAIP identifier rules | 1 | — |
| 2 | `inner.rs`: build, sign, verify | 2 | 1 |
| 3 | `intent.rs`, `transcript.rs`: intent id, terms hash, state machine | 2 | 1 |
| 4 | Seal and open composition over sealed A2A, checks 1 to 10 | 3 | 2, 3 |
| 5 | WASM wrappers and `agswap.ts`, nonce store | 4 | 4 |
| 6 | Golden vectors, negative tests, end-to-end test, docs | 4 | 4 |

Tasks 2 and 3 can run in parallel. Task 5 touches `core/src/wasm/mod.rs`, a
common conflict point.

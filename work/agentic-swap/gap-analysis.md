---
created: 2026-10-05
status: draft
type: analysis
---

# Agentic swap: gap analysis for Mnemonik and Warrant

This analysis answers three questions about the draft "Agentic Swap Spec v0.1"
(2026-10-05):

1. What do Mnemonik and Warrant already supply?
2. What is missing?
3. Is Warrant too large for this use case?

The Warrant side has its own specification:
[Warrant for swaps](https://github.com/mnemonik-xyz/warrant/blob/main/swap/spec.md).

Abbreviations: A2A — agent to agent; COSE — CBOR Object Signing and Encryption;
HSM — hardware security module; HTLC — hashed timelock contract; JCS — JSON
Canonicalization Scheme (RFC 8785); KMS — key management service; LLM — large
language model; TEE — trusted execution environment.

Limit of this analysis: the Flo.tc code (`@portaldefi/sdk`, `UserSDK`, `Deal`)
was not available. Statements about Flo.tc come from the draft specification.

## 1. What exists

### 1.1 Mnemonik (available now)

| Need from the draft | Code |
|---|---|
| Content encryption, key commitment `kc` | `core/src/sealed/content.rs` |
| Seal arbitrary inner bytes | `core/src/sealed/api.rs` `seal_memory`, `open_memory` |
| Per-recipient grants (HPKE, X25519) | `core/src/sealed/api.rs` `make_grant`, `open_grant`; `core/src/sealed/wrap.rs` |
| Ed25519 to X25519 | `core/src/sealed/keys.rs` |
| Signed A2A binding with `context_id` and `prev_id` | `core/src/codec/a2a/signed.rs` (feature `a2a-experimental`) |
| Parent link check (same context, parent hash, signer is author or reader) | `verify_parent_link` in the same file |
| Recipient key from a verified AgentCard | `recipient_from_verified_card` |
| Anchoring of exact bytes, receipts only on the server | `mnemonic_attest_a2a`, `mnemonic_recall_a2a` (`mcp/src/tools.rs` `ingest_a2a`) |
| TypeScript and WASM entry points | `packages/sdk/src/a2a.ts`; `core/src/wasm/mod.rs` `prepare_a2a`, `open_a2a`, `verify_a2a` |
| A signed judgement over a hash | `VERDICT_V1` (feature `trajectory-experimental`) |

### 1.2 Warrant (`policy-execution`, available now)

- `verified/`: a Verus-proved evaluator with three values (`Allow`, `Deny`,
  `Ask`). Its atoms are for invoices and contractor payments only.
- `core/`: policy validation, evidence checks with secp256k1, an invoice parser
  and fixed-width journals for EVM escrows.
- `methods/`, `host/`, `contracts/`: RISC Zero guests, a prover host and four
  escrow contracts.
- No COSE, no Ed25519 and no Mnemonik integration.

## 2. What is missing

### 2.1 Integration boundary

One rule decides where each gap goes. Mnemonik provides transport, identity,
sealing, signing and anchoring, and knows nothing about any one application. An
integration defines its own message types, state machines and policies in its own
package, on top of the public Rust API or the SDK. A new integration therefore
needs no change to `mnemonic-core`. A change enters core only when it is generic.

### 2.2 Gaps

Generic gaps. They go into Mnemonik once and serve every integration
([tech-spec.md](tech-spec.md)):

| # | Gap | Home | Size |
|---|---|---|---|
| M1 | Signed sealed message: a plaintext signature before sealing that binds a caller-chosen protocol tag, the session, the previous message, a nonce and the recipient; the matching open checks and a nonce store. Today a sealed A2A message has no plaintext signature, and replay protection is only content-hash deduplication. | `core/src/codec/a2a/inner.rs`, SDK | S–M |
| M2 | A WASM and SDK function that verifies a standalone COSE signature. TypeScript can sign generic COSE today, but not verify it. | `core/src/wasm/mod.rs`, SDK | S |
| M3 | A key-custody `Signer` interface, so an Ed25519 key can stay in an HSM, a KMS or a TEE. Same algorithm and `kid`. | `core/src/identity/` | M |
| M4 | Key rotation and recipient key discovery | [`work/agent-identity-records/`](../agent-identity-records/) | L |

Swap gaps. They are integration code and live in the Warrant swap
specification or at the venue. None of them changes `mnemonic-core`:

| # | Gap | Home | Size |
|---|---|---|---|
| W1 | Negotiation message kinds RFQ, QUOTE, COUNTER, ACCEPT, REJECT, EXPIRE, carried as the `body` of an M1 message | Warrant `swap-core` | S |
| W2 | Swap intent with `intent_id = blake3(JCS(terms))`; CAIP chain, account and asset ids | Warrant `swap-core` | S |
| W3 | Transcript state machine and transcript hash | Warrant `swap-core` | M |
| W4 | Swap rule atoms and facts | Warrant `swap-verified` | M |
| W5 | Signed oracle price facts | Warrant `swap-core` | M |
| W6 | Notional ledger for limits per period | Warrant `swap-signer` | S–M |
| W7 | Swap warrant record, signed with the Mnemonik identity and anchored through `mnemonic_attest_a2a` | Warrant `swap-core` and `swap-signer` | M |
| W8 | Policy signer: phase gate, secret hygiene, timeout checks, watchers | Warrant `swap-signer` + venue adapter | L |
| V1 | MCP and REST tools for swaps | Venue | M |
| V2 | Additive `Deal` fields: actors, intent hash, warrant references, transcript hash | Venue | S |
| V3 | Agent discovery: signed RFQ, well-known context per pair | Venue + AgentCard | M |
| V4 | Human approval and audit views | Venue web application | M |

## 3. Is Warrant too large for swaps?

The proving and settlement stack is too large for swaps. The evaluator idea is
the right size.

No HTLC reads a warrant. The venue keeps its settlement contracts unchanged. A
warrant therefore stops a bad trade only where the key that funds the lock
refuses to sign without it. For that, the swap path needs:

- a verified evaluator with swap atoms (about 500 lines, like `verified/`);
- a small fact builder and a set of chain safety checks;
- a signed and anchored record.

The swap path does not need these parts: RISC Zero guests, the Groth16 wrap, the
Circom prototype, the escrow contracts, the fixed-width EVM journals, secp256k1
evidence signing or the invoice parser.

The evaluator in `verified/` is already separate from the zkVM code. Do not add
swap atoms to it. All three guests compile it, and four contracts pin a guest
image id as an immutable value. New swap crates keep the invoice product
stable. The Warrant swap specification, section 13, gives the layout.

## 4. Where the decision runs

The agent (the LLM process) holds no key and no funds. The funds and keys belong
to the owner: the human or company that the agent acts for. The owner's policy
signer runs in the owner's own environment: a separate process next to the agent,
an HSM, a KMS, a TEE, or the owner's own wallet. It holds the owner's funding key,
identity key and swap secret. No funds move to a third party. The signer
builds facts, runs the evaluator and signs only on `Allow`. It sends `Ask` to
the owner. A WASM build of the evaluator lets the counterparty and auditors
re-run a decision. An evaluator inside the agent process is acceptable for a
demonstration. It is not a control, because a manipulated agent can skip it.

## 5. Constraints found in Mnemonik code

- `sign_cose` (`core/src/codec/sign.rs`) puts only `alg` and `content_type` in
  the protected header. External additional authenticated data is empty. The
  domain separator must therefore go inside the signed payload, as the A2A
  binding does with `protocol: "mnemonic.a2a.signed.v1"`.
- Use JCS for the new objects. `to_canonical_cbor` drops fields that are not in
  `cbor_field_order`, converts RFC 3339 strings to tags and serves memory
  artifacts only.
- Anchor transcripts and warrants through `mnemonic_attest_a2a` with
  `kind: "artifact"` in the negotiation `context_id`. `/api/ingest-artifact`
  accepts only memory and sealed artifacts. No new ingestion adapter is needed.
- `sign_artifact` takes a concrete keypair. Core has no `Signer` trait. The SDK
  has `SignerInterface`, but the A2A path passes the raw secret to WASM
  (`packages/sdk/src/client.ts`). This is gap M3.
- Identity strings differ: ingestion uses `did:sol:<key>`, A2A `SEALED_V1` uses
  the bare base58 key. The generic signed sealed message (M1) uses the bare
  base58 key and compares raw 32-byte keys (tech-spec, "Identity encoding").
- Key rotation is not implemented and has no design yet (gap M4). The
  Ed25519-to-EVM key binding (`KEY_BINDING_V1`, `work/dual-key-identity/`) that
  the Warrant swap specification, section 4.3, needs is also still to do.

## 6. Corrections to the draft specification

1. The draft says that encrypt-then-sign gives no content non-repudiation. This
   is only partly true. The signed `kc` commits to the per-message key. A party
   can reveal that key and so prove the plaintext of one message. The inner
   signature is still useful: it is self-contained and binds the recipient and
   the nonce.
2. Sign, encrypt, sign needs no change to `SEALED_V1`. The signed inner message
   goes into the sealed A2A message as one DataPart, which the existing path
   seals and signs.
3. Warrant needs no zkVM and no on-chain verifier for HTLC swaps.
4. A daily limit needs a stateful ledger. Warrant has none today.
5. The draft does not list the chain-level safety checks: SHA-256 on both legs,
   a 32-byte preimage check, a reveal deadline, finality before dependence, fixed
   receivers, contract and asset identity, fee reserves and prepared refunds. The
   Warrant swap specification, section 7, makes them obligatory.
6. The draft places the negotiation messages in Mnemonik ("adopt as negotiation
   channel"). The channel is generic Mnemonik. The swap message types are
   integration code and belong to the swap layer (section 2.1).

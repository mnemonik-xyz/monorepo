# Decisions log: erc8004-reputation

Append-only. Owner decisions, task reports and audit findings go here.

---

## Fixed inputs (owner)

- **2026-09-27.** Scope is off-chain document plus hash plus ready-to-send calldata only. No
  EVM signer, no gas handling, no new `alloy` dependency, no transaction submission from our
  side. The agent's own wallet submits `giveFeedback`.
- **2026-09-27.** The MCP server stores no memory, so no hosted hosting route for the feedback
  document. See `work/arweave-as-source-of-truth/`.
- **2026-09-27.** ERC-8004 work may run in parallel with the Arweave source-of-truth waves. It
  has no logical dependency on them, and it does not need anchor pluggability (#70).

## Spec findings (2026-09-27)

- **F1.** `work/a2a-bridge/backlog.md` Path 3 specifies `feedbackHash = blake3 of canonical
  JSON`. This is wrong and load-bearing: `feedbackHash` is a `bytes32` read by EVM consumers,
  which have keccak256 built in and no blake3. See D-1.
- **F2.** The Reputation Registry is stable; the Validation Registry is explicitly still under
  discussion with the TEE community. So reputation is shippable now and the validator service
  (#72) is not.
- **F3.** `alloy-primitives` with the `k256` feature is already a non-optional dependency of
  `mcp/`, so keccak256 exists in Rust today. There are no JS EVM libraries anywhere
  (`viem`, `ethers`, `wagmi` all absent), and this is an SDK and CLI deliverable, so the
  TypeScript side needs `@noble/hashes` promoted from devDependencies.
- **F4.** `core/src/codec/canonical.rs::to_canonical_cbor` must not be used for this document:
  non-integer numbers become text and RFC 3339 strings become CBOR tag 1 epoch seconds, so both
  the score and the timestamp would be corrupted.
- **F5.** `mcp/src/wallet_link.rs` already binds an EVM address to a Mnemonic subject over
  EIP-191. It is the right message-discipline precedent but not reusable: server-side state,
  scoped to a paid operation id, five-minute expiry, and it publishes only a subject hash. A
  durable public reputation artifact must stay verifiable forever.
- **F6.** The registry forbids self-promotion, so an in-family rater address is a guaranteed
  revert. Without a client-side guard the agent burns gas on a doomed transaction.

---

## Decisions

### D-1. keccak256 over RFC 8785 JCS, not blake3 (2026-09-27)

Corrects F1. An artifact that crosses into another ecosystem takes that ecosystem's hash.
`keccak256(fetchedBytes) == feedbackHash` is one line in viem or `cast keccak`; a blake3 digest
in a `bytes32` is well formed and practically unverifiable by every tool that reads it.

blake3 is not dropped: `mnemonic.blake3` remains the content hash of the cited attestation, the
value already anchored. Two hashes, two distinct roles, no drift.

### D-2. Two-level hash so the proofs are committed (2026-09-27)

`payload_hash` covers the rating; `feedbackHash` covers the whole document including
`payload_hash` and `proofs`. A single level is broken either way: signatures inside the hashed
bytes are circular, signatures outside can be stripped while the on-chain hash still matches.

### D-3. `clientAddress` lives inside the hashed payload (2026-09-27)

This is what makes the rater-to-sender binding free, but only if verifiers check
`msg.sender == feedback.clientAddress`. That check is a MUST in the spec document, and the
verifier reports `not-checked` rather than `ok` when the sender is not supplied, so a lazy
verifier cannot read more into the Ed25519 proof than it carries.

### D-4. Caller-provided `feedbackURI` in round 1 (2026-09-27)

The hosted route is ruled out by the no-server-memory direction, and Arweave hosting needs a
raw-bytes publish surface that does not exist — `write_item` is reachable only through the
anchored-memory path, which uploads a COSE envelope, so the keccak would not match. `feedbackUri`
is an input rather than a derived value, so Arweave hosting is a later additive flag.

When it lands, put the gateway HTTPS URL on chain, not `ar://`: no EVM consumer resolves `ar://`,
and the bytes are content-addressed, so a gateway is a substitutable mirror, not a trust anchor.

### D-5. No aggregation in round 1 (2026-09-27)

Ship the fields any future signal needs — `mnemonic.ed25519_pubkey`, `mnemonic.blake3`,
`mnemonic.anchor`, `createdAt` — inside the hashed payload from day one, so aggregation needs no
`V2`. With no server-side index, a signal must be computable client-side or by anyone from
Arweave, which is the honest version. Proposed as `erc8004-6`.

### D-6. `mnemonic.anchor` is `{chain, kind, ref}` (2026-09-27)

Anchor-agnostic from the start, so an Ethereum anchor (#75) slots in with no version bump. The
cheapest item in the specification.

## Open questions

- **Q-1.** Does the owner want the optional EIP-191 proof in round 1, or only the Ed25519 one?
  The design accepts it as caller-supplied input either way, so this only changes documentation
  and one test.
- **Q-2.** Which testnet to pin alongside mainnet: Sepolia, Base Sepolia, or both.

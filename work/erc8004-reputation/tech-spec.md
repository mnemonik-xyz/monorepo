---
created: 2026-09-27
status: draft
size: M
branch: feat/erc8004-reputation
---

# Tech Spec: ERC-8004 reputation feedback helper

Closes issue #73 (`erc8004-3`). Scope: **produce, never submit**. No EVM signer, no gas
handling, no new `alloy` dependency, no transaction from our side, and no server involvement.
The whole feature is a client-side function pair plus a command, and it works offline.

## Solution

Four deliverables, all in the TypeScript packages:

1. **A canonical off-chain document** (`MNEMONIC_FEEDBACK_V1`) that carries the score and the
   Mnemonic evidence, canonicalized with RFC 8785 (JCS) and committed with keccak256.
2. **`prepareFeedback`** — builds the document, signs the payload with the caller's Ed25519
   identity, and returns the bytes to host plus ready-to-send `giveFeedback` calldata.
3. **`verifyFeedbackDocument`** — the offline verifier any third party or aggregator runs.
4. **A CLI** (`mnemonic erc8004 feedback` and `feedback-verify`).

## Blocking verification step

These five facts must be pinned from the deployed contracts before any code. They fill data
tables; they do not change the design. Do not guess them.

1. The exact `giveFeedback` ABI types. Pull the verified ABI from the mainnet Reputation
   Registry, commit it as data with the source address, explorer URL and fetch date in a
   header comment, and assert the 4-byte selector in a test.
2. Registry addresses per chain (mainnet plus one testnet), always overridable by a flag so a
   stale table never blocks a user.
3. The `tag1` / `tag2` encoding: right-padded UTF-8 `bytes32`, or `keccak256(tag)`. Mirror the
   reference implementation.
4. Whether `feedbackURI` is kept in contract storage or only emitted in an event. This decides
   the gas advice and whether a `data:` URI is ever sensible.
5. The contract's exact self-promotion `require` list. The client guard must mirror it.

## Decisions

### Decision 1 — keccak256 over JCS, not blake3

`feedbackHash` is a `bytes32` that EVM consumers read. Solidity, viem, ethers and foundry all
have keccak256 built in and have no blake3. A blake3 digest in that slot is well formed and
practically unverifiable by every tool that reads it: `keccak256(fetchedBytes) == feedbackHash`
is one line in viem or `cast keccak`, while blake3 needs a non-standard dependency and the
knowledge to add it. **An artifact that crosses into another ecosystem takes that ecosystem's
hash.**

This corrects `work/a2a-bridge/backlog.md` Path 3, which specifies "`feedbackHash` = blake3 of
canonical JSON". That is the most load-bearing error in the backlog.

blake3 keeps its own job: `mnemonic.blake3` is the content hash of the **cited attestation**,
the value already anchored on Solana and Arweave. So the document carries two hashes with two
distinct roles and no drift risk.

Canonicalization is JCS over JSON, never `core/src/codec/canonical.rs::to_canonical_cbor`. That
path turns non-integer numbers into text and RFC 3339 strings into CBOR tag 1 epoch seconds, so
it would destroy both the score and the timestamp. A CBOR document is also unreadable to an
EVM-native consumer.

### Decision 2 — a two-level hash, so the proofs are committed

A single hash fails either way. Put the signatures inside the hashed bytes and the dependency
is circular; leave them outside and an adversary strips or swaps them while the on-chain hash
still matches. Nest the commitment:

- `payload_hash = keccak256(JCS(document.feedback))` — what the Ed25519 key signs.
- `feedbackHash = keccak256(JCS(document))` — the whole document, including `payload_hash` and
  `proofs`. This is the `bytes32` that goes on chain.

No circularity, and the on-chain hash covers the proofs.

### Decision 3 — the rater-to-sender binding comes free, on two conditions

`clientAddress` sits **inside** the hashed `feedback` object, and the documented verification
procedure requires `msg.sender == feedback.clientAddress`. Then:

- the Ed25519 key named that address, so a copier cannot replay the document — their address
  differs and they cannot re-sign;
- the address committed that exact statement on chain, so it endorsed the statement.

If a verifier skips the sender check, the Ed25519 proof degrades to "somebody with this key
wrote this text". The check is therefore a MUST in the spec document and is implemented in
`verifyFeedbackDocument`, which takes the on-chain sender as an optional input.

An optional EIP-191 proof lets a verifier confirm the binding without reading the chain. The
caller produces it themselves; we never hold the key.

`mcp/src/wallet_link.rs` is the right precedent for message discipline but is not reusable
here: it is server-side state, scoped to a paid operation id, expires in five minutes, and
publishes only a subject hash. A durable public reputation artifact must stay verifiable
forever.

### Decision 4 — the caller hosts the document; the hosted route is not an option

Round 1 requires a caller-provided `feedbackURI`. The hosted read route is ruled out by the
architectural direction that the server stores no memory, and Arweave hosting needs a
raw-bytes publish surface that does not exist: `core/src/arweave/mod.rs::write_item` is
reachable only through the anchored-memory path, which uploads a COSE envelope of a memory
artifact, not our JCS bytes — so the keccak would not match.

Liveness, stated plainly: if the URI dies the rating stays on chain and immutable, but is no
longer interpretable. The hash keeps it tamper-evident, not available.

`feedbackUri` is an input rather than a derived value, so Arweave hosting is a later additive
flag with no schema change. When it lands, put the **gateway HTTPS URL** on chain, not
`ar://`, because no EVM consumer resolves `ar://`; the bytes are content-addressed and the hash
is on chain, so any gateway is a substitutable mirror, not a trust anchor.

The document is plain JSON, not a signed Mnemonic artifact: a viem or ethers consumer must be
able to `JSON.parse` it. The Mnemonic signature lives inside it as a `proofs[]` entry.

### Decision 5 — no aggregation in round 1, but no schema change needed later

What Mnemonic's data can give that the registry structurally cannot: rater tenure priced in
time rather than stake (first-anchored timestamp and anchored count, enumerable from Arweave by
anyone); basis-in-evidence with time-priority; collusion signals from overlapping citations.

Round 1 ships none of it, and instead makes it possible without a `V2`:
`mnemonic.ed25519_pubkey`, `mnemonic.blake3`, `mnemonic.anchor` and `createdAt` are inside the
hashed payload from day one. With no server-side index, any such signal must be computable
client-side from a local store or by anyone from Arweave — which is the honest version, and the
only one that matches the trust story. Propose `erc8004-6` for it.

Round 1 does ship the cheap half: `feedback-verify`, the offline single-document verifier that
any aggregator will call in a loop.

### Decision 6 — `mnemonic.anchor` is anchor-agnostic from the start

Shape it as `{chain, kind, ref}`. An Ethereum anchor (#75) then slots in as
`{chain: "ethereum", kind: "validation-response", ref: "<txhash>"}` with no version bump.
Reserving that shape now is the cheapest item in this specification.

## The document

```jsonc
{
  "schema": "MNEMONIC_FEEDBACK_V1",
  "feedback": {
    "agentRegistry": "eip155:1:0x<reputationRegistry>",
    "agentId": "42",                        // decimal STRING; uint256 exceeds the JSON safe range
    "clientAddress": "0x<EIP-55>",          // MUST equal msg.sender of the giveFeedback tx
    "createdAt": "2026-09-27T12:00:00Z",    // RFC 3339, UTC, second precision
    "value": 9800,                          // integer within the JSON safe range
    "valueDecimals": 2,
    "tags": { "tag1": "quality", "tag2": "research" },   // optional keys OMITTED, never null
    "mnemonic": {
      "schema": "MEMORY_V1",                // schema of the CITED attestation
      "attestation_id": "...",
      "blake3": "<64 lowercase hex>",
      "ed25519_pubkey": "<base58>",
      "cose_envelope_uri": "ar://<txid>",   // optional
      "anchor": { "chain": "solana", "kind": "spl-memo", "ref": "<sig>" }  // optional
    },
    "note": "..."                           // optional, <= 280 chars
  },
  "payload_hash": "0x<keccak256 of JCS(feedback)>",
  "proofs": [
    { "type": "ed25519", "kid": "<base58>", "sig": "<base64, 64 bytes>" },
    { "type": "eip191", "address": "0x<EIP-55>", "sig": "0x<65 bytes>" }   // optional
  ]
}
```

Signed message for both proof types, one line and shell-safe so `cast wallet sign` can be
copy-pasted:

```
MNEMONIC_FEEDBACK_V1:0x<payload_hash, lowercase hex>
```

`ed25519.sig` is a raw Ed25519 signature over the UTF-8 bytes of that string, deliberately not
COSE: an outsider must be able to verify it with `@noble/ed25519` or `ed25519-dalek` in three
lines and no CBOR stack. COSE remains the envelope for memory artifacts.

The document never contains its own URI. That would make hosting circular.

### Why restricted JSON makes JCS small

RFC 8785 is defined in terms of ECMAScript `JSON.stringify` number and string serialization. If
the value space is restricted to strings, safe-range integers, booleans, objects and arrays —
which the schema enforces — JCS reduces to: sort object keys recursively (the default string
sort is JCS's UTF-16 code-unit order) and stringify with no spacing. So the serializer must
reject, loudly: non-integer numbers, numbers outside the safe-integer range, `null`,
`undefined`, `NaN`, `Infinity`, lone surrogates, and non-plain objects. Tests still validate
against published JCS vectors, so the shortcut is proven rather than assumed.

### Libraries

- **TypeScript:** keccak256 from `@noble/hashes/sha3`. Promote `@noble/hashes` from
  devDependencies to dependencies — already in the lockfile, pure ESM, no `node:` imports, so
  the runtime-agnostic property of the SDK survives. JCS is about 60 lines we own; the npm
  `canonicalize` package is a cross-check in tests, not a runtime dependency.
- **Rust:** `alloy_primitives::keccak256`, already a non-optional dependency of `mcp/`, so the
  cross-ecosystem parity test costs nothing. When #72 must *produce* these documents
  server-side, port JCS into `core/src/erc8004/` and add `sha3` there rather than pulling
  `alloy` into `core/`.

## API surface

`submitMnemonicFeedback` from issue #73 is the wrong name: it promises a transaction we
deliberately never send, and its `(agentId, value, attestationId)` arity omits everything that
matters. Replace with free functions — no client, no base URL, no JWT, no network:

- `prepareFeedback(input, opts) -> PreparedFeedback`
- `verifyFeedbackDocument(input) -> VerifyFeedbackResult`
- `encodeGiveFeedback(args) -> calldata`
- `checkSelfPromotion(opts) -> SelfPromotionCheck`

`PreparedFeedback` carries the document, the canonical bytes and both hashes, the
`feedbackUri`, an `onchain` block (`chainId`, `to`, `data`, `value`, the pinned function
signature, the selector and the decoded args), a `preflight` block naming the required sender
and chain, and `warnings`.

Supporting internal modules, flat and not re-exported: `jcs.ts` (canonicalizer plus the
restricted-value validator), `abi.ts` (a minimal encoder for `uint256`/`uint8`/`bytes32`/
`string` and a static decoder), `evm.ts` (keccak256, EIP-55, hex, raw `eth_call` over `fetch`).

One small Rust addition: export `verify_ed25519(pubkey_base58, msg, sig)` from
`core/src/wasm/mod.rs`, wrapping the existing `identity::verify_signature`. About ten lines, no
new crates, and it keeps verification at parity with the native path.

## How the caller sends it

We hand over a transaction request and nothing more:
`{ chainId, to, data, value: "0x0" }`. Documented recipes need only `to` and `data`:
`cast send`, viem `sendTransaction`, wagmi `useSendTransaction`, or pasting into a Safe
Transaction Builder or an ERC-4337 user-op. This is why the output is `to` plus `data` rather
than a contract-call abstraction.

The caller's responsibilities, echoed in `preflight` and on stderr: gas and nonce are the
wallet's job; the sender **must** be `feedback.clientAddress`, because the document commits to
it and verifiers check it, so sending from another address permanently mismatches the record
and the fix is to regenerate the document, not to resend; the wallet must be on `chainId`; and
the document must be reachable before the transaction lands, or early readers see a dangling
pointer.

## Self-promotion guard

The registry forbids an agent's controllers and operators from rating it, so an in-family
address is a guaranteed revert and wasted gas. When an RPC URL is available, check by raw
`eth_call` over `fetch` (the same shape as the existing JSON-RPC calls in `mcp/src/payment.rs`;
no EVM library needed, since we have keccak for selectors and static ABI decoding is trivial):
`ownerOf(agentId)`, `getApproved(agentId)`, `isApprovedForAll(owner, clientAddress)`, the
reserved `agentWallet` metadata, and whatever else the verification step finds.

Offline and free regardless of RPC: reject the zero address and a bad EIP-55 checksum.

Limits, stated in the documents: it needs an endpoint, and without one we emit a warning rather
than silently passing; it is a snapshot, and ownership can change before inclusion; and it is
**not** an anti-collusion mechanism — a fresh unrelated address held by the same operator is
undetectable at this layer, by construction.

## Testing

- `jcs.test.ts` — published RFC 8785 vectors (unicode, escapes, key ordering including
  non-ASCII keys), plus rejection of floats, unsafe integers, `null`, `undefined` and lone
  surrogates.
- `erc8004.abi.test.ts` — the selector equals the pinned value, and full calldata equals a
  checked-in fixture generated by `cast calldata`, with the generating command in a comment.
- `erc8004.feedback.test.ts` — deterministic `prepareFeedback` with a fixed keypair and
  `createdAt`; verify round-trip; byte-tamper fails the hash; **stripping a proof fails the
  hash**, which is what proves the two-level design works; sender mismatch is reported;
  validation errors for oversized tags, a bad checksum and an out-of-range value.
- `erc8004.selfpromotion.test.ts` — mocked RPC: owner match, approved-operator match, clean
  pass, RPC error.
- **`fixtures/erc8004-feedback-v1.json` — reproducible by a third party with no Mnemonic code**:
  `jq -j .documentJson fixture.json | cast keccak` must equal `feedbackHash`. The header
  documents that two-command reproduction.
- `mcp/tests/erc8004_feedback_parity.rs` — reads the same fixture, recomputes keccak256 with
  `alloy_primitives::keccak256`, and asserts both hashes and the selector. Zero new
  dependencies; this is the cross-ecosystem proof.
- `packages/cli/test/commands/erc8004.test.ts` — `--json` stdout shape, human rendering, stderr
  warnings, `--out` bytes identical to `canonical.documentJson`, exit codes, and the
  `--uri`/`--data-uri` mutual exclusion.

## Sequencing (waves)

| # | Task | wave | depends_on | Days |
|---|---|---|---|---|
| 1 | Pin the ABI, addresses, tag convention, URI semantics and require-list | 1 | [] | 0.5 |
| 2 | `jcs.ts` + `evm.ts` + tests | 1 | [] | 0.5 |
| 3 | `abi.ts` + selector and calldata fixture test | 2 | [1] | 0.5 |
| 4 | `prepareFeedback`, `verifyFeedbackDocument`, `verify_ed25519`, the reproducible fixture, `docs/spec/erc8004-feedback-v1.md` | 3 | [2, 3] | 1.0 |
| 5 | `checkSelfPromotion` + mocked-RPC tests | 4 | [4] | 0.5 |
| 6 | CLI command + tests | 4 | [4] | 0.5 |
| 7 | Rust parity test, READMEs, docs, backlog and issue rewrites | 5 | [4, 5, 6] | 0.5 |

**Total about 4 dev-days**, not the 2 in issue #73: the pinning work and the reproducible
fixture are real and were unpriced. Tasks 1 and 2 are independent, so the critical path is
about 3 days.

Task 1 blocks everything. Do not start task 3 or 4 by guessing the ABI.

## Risks

- **A stale registry address table.** Mitigation: every address is overridable by flag, and an
  opt-in test asserts the pinned mainnet address has code.
- **The tag encoding convention differs from the reference implementation.** Mitigation: the
  document stores the tag string and the calldata stores the bytes32, so a mismatch is
  detectable rather than silent.
- **A verifier skips the `msg.sender` check** and reads more into the Ed25519 proof than it
  carries. Mitigation: it is a MUST in the specification document, and the verifier reports
  `senderBinding: "not-checked"` rather than `"ok"` when the sender is not supplied.
- **#72 introduces a second, incompatible canonicalization.** Mitigation:
  `docs/spec/erc8004-feedback-v1.md` plus the language-neutral fixture are what prevent it.
- **Adoption is tied to ERC-8004 adoption.** Mitigation: the document and its verifier are
  useful off chain too, and nothing here is on the critical path of another feature.

## What to rewrite elsewhere

- **Issue #73** — replace the body: wrong verb (`submit`), wrong arity, estimate 2 to 4 days,
  and add the explicit non-goals.
- **`work/a2a-bridge/backlog.md` Path 3** — `feedbackHash = blake3` is wrong (Decision 1); the
  sample conflates `MEMORY_V1` with the document schema; the sample omits `payload_hash`,
  `proofs[]` and `mnemonic.ed25519_pubkey`, without which the namespace proves nothing about
  the rater; "emits the on-chain call" becomes "emits calldata the agent's wallet submits".
- **`core/src/codec/canonical.rs`** — add one sentence to the module doc saying it is for memory
  artifacts only and must not be used for cross-ecosystem documents. This is the trap #72 would
  otherwise walk into.

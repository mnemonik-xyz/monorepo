# dual-key-identity — parking lot

Status: **TODO — mostly already built; needs assembling, not inventing.**

Lets one agent hold an Ed25519 Mnemonic identity **and** a secp256k1 Ethereum key, with a
verifiable link between them.

Siblings:
- [`work/erc8004-reputation/`](../erc8004-reputation/) — needs this. Its `clientAddress` binding
  is the first consumer.
- [`work/multi-suite-signing/`](../multi-suite-signing/) — the alternative approach, and the
  risky one. Read the comparison below before choosing.

## The constraint, first

**Solana keys are Ed25519. Ethereum keys are secp256k1. Different curves.** You cannot derive
one from the other, and one key cannot produce both signature types. So "use the Solana secret
key as an Ethereum key" is not possible, and no amount of plumbing changes that.

What is possible: hold both, and bind them with signatures so a verifier can prove they belong
to the same principal.

## Why this is the cheap path

The Ed25519 key keeps signing memory artifacts, unchanged. That matters more than it sounds:

- `core/src/codec/sign.rs:56` pins `alg = EdDSA`.
- `verify_artifact` requires the COSE `kid` to parse as a base58 Solana pubkey (`:121`) and
  verifies Ed25519 only.
- Every artifact ever written carries that shape, **including bytes already permanent on
  Arweave**, which cannot be rewritten.

Touch none of that, and there is no way for this feature to invalidate an existing attestation.
That is the whole argument for doing it this way first.

## What already exists

- **`mcp/src/wallet_link.rs`** — binds an EVM address to a Mnemonic subject over EIP-191, with a
  challenge, a nonce and a `paid_wallet_links` table. The right precedent for message discipline
  (domain-tagged, one field per line). Not directly reusable: it is server-side state, scoped to
  a paid operation id, expires in five minutes, and publishes only a subject hash. A durable
  identity binding must stay verifiable for ever.
- **`alloy-primitives` with `k256`** — already a non-optional dependency of `mcp/`, so secp256k1
  and keccak256 are available with no new crates.
- **`work/erc8004-reputation/` D-3** — already specifies a per-statement binding:
  `clientAddress` inside the hashed payload plus an optional EIP-191 proof.

## What is missing

A **standing** binding, rather than a per-statement one. `KEY_BINDING_V1`:

- Payload: `{ed25519_pubkey_base58, evm_address, created_at, nonce}`.
- Signed **twice**: once by the Ed25519 identity (COSE_Sign1, the existing path) and once by the
  EVM key (EIP-191 over the same canonical bytes).
- Either signature alone proves nothing. Both together prove one principal holds both keys.
- Published where a verifier already looks: the AgentCard `x-mnemonic` extension, and later
  `did:mnemonic` (#74).

Note the overlap with `work/sealed-memories/` Decision 7, which adds an X25519 key to the same
extension. These should be designed together so the extension is not extended twice.

## Compared with multi-suite signing

| | dual-key binding | multi-suite signing (#29) |
|---|---|---|
| Existing attestations | unaffected | must all keep verifying |
| `kid` format | unchanged | must change or become multi-format |
| New crypto | none | secp256k1 verification in `core` |
| Worst failure | a binding is rejected | **every stored artifact stops verifying** |
| Reversible | yes | no, for anchored bytes |

Do this one first. It also removes the only urgent reason to do the other one.

## Out of scope

- Signing memory artifacts with a secp256k1 key. That is `work/multi-suite-signing/`.
- Renaming `did:sol:` — 66 references and live production data, no technical benefit.
- Key rotation. Needs its own thought; a binding is not a rotation mechanism.

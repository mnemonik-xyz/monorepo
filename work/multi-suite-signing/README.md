# multi-suite-signing — parking lot

Status: **TODO — last in the sequence. Needs its own spec and a golden-vector proof.**

Tracks issue **#29** ("Crypto-flexibility (Phase 2): off-chain alg-pluggable Signer"). Lets an
artifact be signed by something other than an Ed25519 Solana key.

**Do not start this before [`work/dual-key-identity/`](../dual-key-identity/).** That feature
satisfies the ERC-8004 requirement without touching the signature suite, which removes the only
pressing reason to attempt this one.

## What it changes

Today the suite is pinned in two places, and both are load-bearing:

- `core/src/codec/sign.rs:56` — protected header `alg = EdDSA`.
- `core/src/codec/sign.rs:121` — `verify_artifact` parses the COSE `kid` as a base58 Solana
  `Pubkey` and verifies Ed25519 only.

Multi-suite means a `Signer` trait in `core`, an `alg` that varies, and a `kid` that can express
more than one key type.

## Why it is last

**The failure mode is unrecoverable.** Every artifact ever written carries the current `kid`
shape, and the anchored ones are permanent on Arweave — they cannot be rewritten. A `kid` change
that is not perfectly backward compatible does not degrade service; it makes existing memories
permanently unverifiable, and no migration can fix bytes that are already immutable.

So the bar is higher than "tests pass":

- Every fixture in `core/tests/golden_fixtures.rs` must still verify, byte for byte, with the
  pinned sha256 unchanged. `work/arweave-as-source-of-truth/` used exactly this check to prove a
  schema change moved no bytes; the same evidence is required here.
- The existing `kid` spelling must remain valid input for ever, not just for a release.
- A cross-language conformance vector per suite, so a third-party verifier can be checked rather
  than trusted.

## Secondary reason to wait

There is no `Signer` trait in `core` today: `sign_artifact` takes a concrete
`&solana_sdk::signature::Keypair`. Introducing the seam is most of the work, and
`work/sealed-memories/` will likely want the same seam for its X25519 operations. Doing them
together avoids two incompatible abstractions over key handling.

## Out of scope

- Post-quantum suites. Not until a suite is standardised and needed.
- Changing the canonical CBOR or the hash. Different axis; see the scope note now in
  `core/src/codec/canonical.rs`.

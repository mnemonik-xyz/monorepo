# User Specification: Dual-Key Identity

## What is this?

A Mnemonic agent currently has one key: an Ed25519 keypair tied to a Solana address. That
key signs every memory artifact. It is what makes `did:sol:<pubkey>` mean something.

Ethereum works on a different curve — secp256k1 — so the Solana key cannot be reused as an
Ethereum key. The solution is not to replace the Ed25519 key; it is to let the agent hold
**both keys at once** and prove — cryptographically — that both keys belong to the same agent.

## The binding

The agent creates a short record called `KEY_BINDING_V1`. It contains:
- the Ed25519 public key (the agent's Mnemonic identity),
- the Ethereum address the agent also controls,
- a timestamp and a nonce (to prevent replay attacks).

This record is then signed **twice**:
1. By the Ed25519 key, using the same COSE_Sign1 format the agent already uses for memory
   artifacts.
2. By the Ethereum key, using the standard EIP-191 personal-sign format that any Ethereum
   wallet understands.

**Either signature alone proves nothing.** An attacker could produce a signature from either
key in isolation. Holding both signatures simultaneously proves that one principal controlled
both keys at the moment the binding was created.

## Why this is needed

Ethereum's ERC-8004 standard (reputation NFTs) requires a `clientAddress` field: an Ethereum
address that identifies who is making a claim. Without dual-key identity, a Mnemonic agent
cannot supply a verifiable `clientAddress` — it has no Ethereum key to sign with.

With this binding in place, a verifier reading an ERC-8004 statement can confirm that the
`clientAddress` listed there genuinely belongs to the same agent whose Ed25519 identity signed
the Mnemonic memory artifacts.

## What does not change

- **All existing attestations continue to verify.** The Ed25519 key keeps doing exactly what
  it does today. The COSE_Sign1 format is unchanged. The `kid` field in every artifact stays
  a base58-encoded Solana public key. Nothing written to Arweave needs to be touched.
- **`did:sol:` identifiers are unchanged.** The Solana DID is still the primary identity. The
  Ethereum address is a secondary key, bound to it.
- **The agent's Solana key is the root of trust.** The binding is anchored by the Ed25519
  signature, which is the same key the verifier already trusts.

## Where the binding lives

The binding is published in the agent's **AgentCard** — the public JSON document that describes
the agent's capabilities and identity. Two new fields appear in the `x-mnemonic` extension
block of the AgentCard:
- `evm_address`: the Ethereum address, checksummed per EIP-55.
- `evm_binding_sig`: the EIP-191 signature over the binding payload.

The COSE_Sign1 envelope wrapping the same payload is derived from the existing signing path
and is self-verifying from the `kid` already in the AgentCard.

The AgentCard extension is being designed at the same time as the `sealed-memories` X25519
key addition (Decision 7 of that feature), so the extension is extended exactly once rather
than twice.

## What a verifier does

To trust an ERC-8004 statement from this agent, a verifier:
1. Fetches the AgentCard and reads `x-mnemonic.evm_address` and `x-mnemonic.evm_binding_sig`.
2. Reconstructs the canonical binding payload bytes from the public fields.
3. Checks that the EIP-191 signature over those bytes recovers to `evm_address`.
4. Checks that the COSE_Sign1 binding (also in the AgentCard) verifies with the Ed25519 pubkey
   already trusted as the agent's `did:sol:` identity.
5. Both checks must pass. If either fails, the binding is rejected.

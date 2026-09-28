# Decisions log: dual-key-identity

Append-only.

## Fixed inputs (owner)
- **2026-09-28.** Bind Ed25519 Mnemonic identity to secp256k1 EVM address. Required for ERC-8004 clientAddress.

## Design decisions
- **2026-09-28. D-1 — Ed25519 path unchanged.** `core/src/codec/sign.rs` is not touched. `verify_artifact` continues to require EdDSA + base58 kid. Every artifact on Arweave stays valid with no re-signing.
- **2026-09-28. D-2 — Both signatures required for a valid binding.** An EIP-191 signature alone proves EVM key control but not Mnemonic identity. A COSE_Sign1 alone proves Mnemonic identity but not EVM key control. Neither alone is sufficient; both together prove the same principal holds both keys.
- **2026-09-28. D-3 — AgentCard `x-mnemonic` extension designed once, coordinated with sealed-memories D-7.** The same extension block will also receive an X25519 encryption key (sealed-memories Decision 7). Both additions land in a single schema revision to avoid extending the extension twice.
- **2026-09-28. D-4 — No new crates.** `alloy_primitives` (with `k256` for secp256k1 and keccak256) is already a non-optional dependency of `mcp/`. CBOR canonicalization and COSE_Sign1 are already available in `core/`. This feature adds no entries to any `Cargo.toml` unless a direct dependency on `alloy_primitives` must appear in `core/Cargo.toml`; that is to be verified against the dependency graph before adding.

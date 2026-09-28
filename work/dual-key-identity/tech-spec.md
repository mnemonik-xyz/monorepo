# Technical Specification: Dual-Key Identity

## Overview

Introduce `KEY_BINDING_V1`: a dual-signed payload that cryptographically links an agent's
Ed25519 Mnemonic identity to a secp256k1 Ethereum address. Both signatures are required for
the binding to be considered valid. The Ed25519 signing path (`core/src/codec/sign.rs`) is
unchanged. No new crates are required.

---

## 1. KEY_BINDING_V1 Payload Struct

Located in `core/src/identity/dual_key.rs`.

```rust
/// Canonical payload for a KEY_BINDING_V1 record.
/// Serialized to deterministic CBOR before signing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindingPayload {
    /// Base58-encoded Ed25519 public key (Solana pubkey format).
    pub ed25519_pubkey_base58: String,
    /// EIP-55 checksummed Ethereum address, e.g. "0xAbCd...".
    pub evm_address: String,
    /// ISO-8601 UTC timestamp of binding creation.
    pub created_at: String,
    /// 32-byte random nonce, hex-encoded with 0x prefix.
    pub nonce: String,
}
```

Map key order for deterministic CBOR: `created_at`, `ed25519_pubkey_base58`, `evm_address`,
`nonce` (lexicographic, consistent with the existing canonical CBOR convention in
`core/src/codec/canonical.rs`).

---

## 2. Signing

### 2a. Ed25519 path — COSE_Sign1

Reuse `core::codec::sign::sign_cose` directly. The payload passed to `sign_cose` is the
deterministic CBOR encoding of `KeyBindingPayload` (see §3). The resulting `COSE_Sign1` bytes
are stored as `cose_binding` in the `KeyBinding` output struct. The `kid` in the unprotected
header is the base58 Solana pubkey, identical to every other artifact produced by this agent.

### 2b. secp256k1 path — EIP-191 personal-sign

```
message = canonical_bytes(payload)   // deterministic CBOR from §3
prefixed = "\x19Ethereum Signed Message:\n" + len(message).to_string() + message
digest   = keccak256(prefixed)
evm_sig  = secp256k1_sign(digest, evm_secret_key)   // 65 bytes, r|s|v
```

Use `alloy_primitives::Signature` (already in `mcp/Cargo.toml`) and its `sign_hash` helper.
The resulting signature is hex-encoded with `0x` prefix and stored as `evm_binding_sig`.

Address recovery for verification:

```rust
let recovered = Signature::from_str(&evm_binding_sig)?
    .recover_address_from_msg(&canonical_bytes)?;
assert_eq!(recovered.to_checksum(None), payload.evm_address);
```

---

## 3. Canonical Bytes

The binding payload is serialized to deterministic CBOR using the existing
`core::codec::canonical::to_canonical_cbor_raw(value)` (or an equivalent function that
accepts a pre-formed map). Rules:
- Map keys sorted lexicographically (same as every other canonical artifact in this codebase).
- Strings encoded as definite-length UTF-8 text strings.
- No floats, no indefinite-length encoding.

The same `canonical_bytes` slice is the payload for both signatures (§2a and §2b), guaranteeing
the two signatures are over identical bytes.

---

## 4. Output Struct and AgentCard Extension

### 4a. Output struct

```rust
pub struct KeyBinding {
    /// Canonical CBOR bytes of the payload (source of truth for both signatures).
    pub canonical_bytes: Vec<u8>,
    /// Serialized COSE_Sign1 (Ed25519, kid = ed25519_pubkey_base58).
    pub cose_binding: Vec<u8>,
    /// EIP-191 signature over canonical_bytes, hex with 0x prefix.
    pub evm_binding_sig: String,
    /// The payload, for convenience.
    pub payload: KeyBindingPayload,
}
```

### 4b. AgentCard `x-mnemonic` extension fields

These are the two new fields added to the existing `x-mnemonic` JSON block. The AgentCard
schema is extended once, coordinated with `work/sealed-memories` Decision 7 (X25519 key
addition) so both land in the same schema revision.

```jsonc
{
  "x-mnemonic": {
    // existing fields unchanged ...
    "evm_address": "0xAbCd1234...",          // EIP-55 checksummed
    "evm_binding_sig": "0xdeadbeef...",      // EIP-191 sig over canonical CBOR
    "evm_binding_cose": "<base64url>",       // COSE_Sign1 bytes, base64url-encoded
    // sealed-memories addition (D-7, same revision):
    "x25519_pubkey": "<base64url>"
  }
}
```

`evm_binding_cose` carries the COSE_Sign1 envelope; the Ed25519 pubkey needed to verify it
is recoverable from the `kid` inside that envelope, which equals the agent's known
`did:sol:` public key.

---

## 5. Verification Logic

`core::identity::dual_key::verify_binding(card_extension: &XMnemonicExtension) -> Result<()>`

Steps:
1. Decode `evm_binding_cose` from base64url to bytes.
2. Call `core::codec::sign::verify_artifact` on those bytes. Confirm `result.valid == true`
   and `result.signer == card_extension.ed25519_pubkey_base58`.
3. Extract `canonical_bytes` from `result.payload`.
4. Reconstruct `KeyBindingPayload` by CBOR-deserializing `canonical_bytes`.
5. Confirm `payload.ed25519_pubkey_base58 == result.signer`.
6. Confirm `payload.evm_address == card_extension.evm_address` (case-insensitive checksum
   comparison).
7. Recover EVM address: `Signature::from_str(&evm_binding_sig)?.recover_address_from_msg(&canonical_bytes)`.
8. Confirm recovered address matches `payload.evm_address`.

**Rejection rule:** any single check failure returns an `Err`. Either signature alone never
produces `Ok`. A valid COSE but invalid EIP-191 is rejected. A valid EIP-191 but invalid
COSE is rejected.

---

## 6. Dependencies

No new crates. All required primitives are already available:

| Primitive | Already present in |
|---|---|
| `alloy_primitives::Signature` | `mcp/Cargo.toml` |
| `k256` (secp256k1) | pulled by `alloy_primitives` |
| `coset` (COSE_Sign1) | `core/Cargo.toml` |
| `serde` / `ciborium` (CBOR) | `core/Cargo.toml` |
| `rand` (nonce) | `mcp/Cargo.toml` |

If `core/` does not already depend on `alloy_primitives`, add it as an optional feature
(`evm`) re-exporting nothing; `mcp/` already pulls it in transitively. Verify before adding.

---

## 7. New Module: `core/src/identity/dual_key.rs`

Public API surface:

```rust
pub fn create_binding(
    ed25519_keypair: &solana_sdk::signature::Keypair,
    evm_secret_key: &k256::ecdsa::SigningKey,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<KeyBinding, DualKeyError>;

pub fn verify_binding(
    cose_bytes: &[u8],
    evm_address: &str,
    evm_binding_sig: &str,
) -> Result<KeyBindingPayload, DualKeyError>;
```

Module lives under a new `core/src/identity/` directory. `core/src/identity/mod.rs` re-exports
`dual_key`. The existing `core/src/codec/` tree is not modified.

---

## 8. New MCP Tool: `identity_bind_evm`

Located in `mcp/src/tools/identity_bind_evm.rs` (or added to existing tools file).

**Tool name:** `identity_bind_evm`

**Input schema:**
```jsonc
{
  "evm_private_key": "string"   // hex-encoded 32-byte secp256k1 secret key, 0x prefix optional
}
```

**Behaviour:**
1. Load the agent's Ed25519 keypair from the configured keystore.
2. Parse `evm_private_key` into a `k256::ecdsa::SigningKey`.
3. Call `core::identity::dual_key::create_binding(ed25519_keypair, evm_key, Utc::now())`.
4. Persist the resulting `KeyBinding` to a new `key_bindings` SQLite table (columns:
   `ed25519_pubkey`, `evm_address`, `created_at`, `nonce`, `cose_binding_b64`, `evm_binding_sig`,
   `canonical_bytes_hex`). One row per (ed25519, evm_address) pair; upsert on conflict.
5. Write/update the `x-mnemonic` block in the agent's AgentCard JSON on disk.
6. Return:
   ```jsonc
   { "evm_address": "0x...", "created_at": "...", "status": "bound" }
   ```

**Error cases:** invalid private key format, keypair not loaded, AgentCard not found on disk.

---

## 9. Tasks

| ID | Title | Scope |
|---|---|---|
| T-1 | `KeyBindingPayload` + canonical CBOR + unit tests | `core/src/identity/dual_key.rs` |
| T-2 | `create_binding` + `verify_binding` + property tests | `core/src/identity/dual_key.rs` |
| T-3 | `identity_bind_evm` MCP tool + SQLite migration | `mcp/src/tools/` |
| T-4 | AgentCard `x-mnemonic` schema extension (coord. sealed-memories D-7) | AgentCard types + serialization |
| T-5 | Integration test: create binding → write AgentCard → verify from AgentCard | `mcp/tests/` |

---

## 10. Decisions

### D-1 — Ed25519 path unchanged
`core/src/codec/sign.rs` is not modified. `verify_artifact` continues to require EdDSA +
base58 kid. All existing artifacts on Arweave remain valid without any re-signing.

### D-2 — Both signatures required
A binding with only one valid signature is rejected. This is the security property that makes
the binding meaningful: proving simultaneous control of both keys.

### D-3 — AgentCard extension designed once with sealed-memories X25519 key
`work/sealed-memories` Decision 7 adds an X25519 encryption key to the same `x-mnemonic`
extension block. Both additions (EVM binding fields and X25519 key) are put into a single
schema revision. Tasks T-4 in this feature and the corresponding task in `sealed-memories`
are coordinated or merged.

### D-4 — No new crates
`alloy_primitives` (with `k256`) is already a non-optional dependency of `mcp/`. CBOR and
COSE are already in `core/`. Adding a dependency that is already present transitively is
acceptable only if it must appear in `core/Cargo.toml` directly; verify the dependency graph
before adding anything.

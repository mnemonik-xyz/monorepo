//! WASM bindings for the browser-mediated signing flow.
//!
//! This module exposes a small, stateless surface that the webapp calls from the
//! browser to manage Ed25519 keypairs and produce COSE_Sign1 attestations
//! entirely client-side. There is no network access, no I/O, and no state held
//! across calls. Every function is a thin wrapper over an existing primitive in
//! `crate::identity`, `crate::codec::canonical`, `crate::codec::hash`, or
//! `crate::codec::sign`.
//!
//! Architectural rules:
//! - Compiled only when `target_arch = "wasm32"` AND the `wasm` feature is on
//!   (see `core/src/lib.rs`). Native builds never see this file.
//! - No `unwrap()` / `panic!()` outside `#[cfg(test)]`. Errors are surfaced as
//!   `JsValue::from_str(msg)` so callers in JS receive a proper rejection.
//! - No business logic — composition only. Keypair semantics, canonical CBOR
//!   layout, hashing, and COSE_Sign1 framing all live in `identity` / `codec`.
//!
//! See `work/mnemonic-integrations/tech-spec.md` Decision 3 + Decision 4 for the
//! browser-mediated signing rationale.

#![cfg(all(target_arch = "wasm32", feature = "wasm"))]

use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;
use wasm_bindgen::prelude::*;

use crate::codec::canonical::to_canonical_cbor;
use crate::codec::hash::hash_bytes;
use crate::codec::schema::MEMORY_V1;
use crate::codec::sign::sign_cose;
use crate::identity::{pubkey_base58, sign_bytes};

/// On-the-wire keypair representation that travels across the WASM boundary.
///
/// `secret` is the full 64-byte Solana keypair format (32 bytes seed + 32 bytes
/// public key). `pubkey_base58` is the canonical base58-encoded Ed25519 public
/// key — kept alongside the secret so the JS side can display it without
/// re-running the keypair.
#[derive(Serialize, Deserialize)]
struct KeypairJson {
    secret: Vec<u8>,
    pubkey_base58: String,
}

/// Reconstruct a `Keypair` from a `KeypairJson` payload received from JS.
///
/// Validates: (a) secret is exactly 64 bytes, (b) the embedded `pubkey_base58`
/// matches the public key derived from the secret. Mismatch returns Err — never
/// panics. This is the single choke point for "is this keypair JSON valid?"
/// checks; all five exports route through it.
fn keypair_from_json(value: KeypairJson) -> Result<Keypair, JsValue> {
    if value.secret.len() != 64 {
        return Err(JsValue::from_str(&format!(
            "invalid keypair: secret must be 64 bytes, got {}",
            value.secret.len()
        )));
    }
    let kp = Keypair::try_from(value.secret.as_slice())
        .map_err(|e| JsValue::from_str(&format!("invalid keypair bytes: {e}")))?;
    let derived = pubkey_base58(&kp);
    if derived != value.pubkey_base58 {
        return Err(JsValue::from_str(
            "invalid keypair: embedded pubkey does not match secret",
        ));
    }
    Ok(kp)
}

/// Helper: deserialize a `KeypairJson` from a `JsValue`.
fn keypair_json_from_value(value: JsValue) -> Result<KeypairJson, JsValue> {
    serde_wasm_bindgen::from_value::<KeypairJson>(value)
        .map_err(|e| JsValue::from_str(&format!("invalid keypair JSON: {e}")))
}

/// Generate a fresh Ed25519 keypair.
///
/// Returns `{ secret: Uint8Array(64), pubkey_base58: string }` to JS. Entropy
/// comes from `getrandom` which, on wasm32 with the `js` feature, resolves to
/// `crypto.getRandomValues` (Web Crypto). Without the `js` feature this would
/// panic at runtime — see `Cargo.toml` for the mandatory feature.
#[wasm_bindgen]
pub fn generate_keypair() -> Result<JsValue, JsValue> {
    let kp = Keypair::new();
    let payload = KeypairJson {
        secret: kp.to_bytes().to_vec(),
        pubkey_base58: pubkey_base58(&kp),
    };
    serde_wasm_bindgen::to_value(&payload)
        .map_err(|e| JsValue::from_str(&format!("keypair serialization failed: {e}")))
}

/// Sign an arbitrary challenge byte string with the given keypair.
///
/// Used by the OAuth flow (`/oauth/authorize` signed challenge per Decision 10).
/// Returns the raw 64-byte Ed25519 signature. Verification on the native side
/// is `identity::verify_signature(&Pubkey, challenge, &sig)`.
#[wasm_bindgen]
pub fn sign_challenge(keypair_json: JsValue, challenge: &[u8]) -> Result<Vec<u8>, JsValue> {
    let kpj = keypair_json_from_value(keypair_json)?;
    let kp = keypair_from_json(kpj)?;
    Ok(sign_bytes(&kp, challenge))
}

/// Build the canonical-CBOR memory bundle, hash it, and produce COSE_Sign1
/// signed bytes.
///
/// The artifact JSON layout mirrors `mcp/src/tools.rs::sign_memory` exactly so
/// the resulting COSE_Sign1 verifies byte-for-byte against
/// `codec::sign::verify_artifact` on the native side. Drift here breaks the
/// browser-mediated signing round-trip in Task 7.
///
/// Inputs:
/// - `content` — user memory text.
/// - `embedding_bytes` — TurboQuant-compressed embedding bytes (server-computed
///   and supplied to the browser inside the unsigned bundle from
///   `GET /api/pending/<id>`). Encoded as base64 inside `metadata.embedding_compressed`.
/// - `content_hash` — hex blake3 of the canonical CBOR computed by the server;
///   echoed back so the server-side validator can cross-check before persisting.
///   Currently the WASM side recomputes from its own canonicalization, but
///   accepting it as input keeps the API symmetric with the spec'd endpoint.
/// - `owner_pubkey` — base58 OAuth-resolved pubkey, also used as the artifact
///   `producer` (`did:sol:<owner_pubkey>`). For Phase 1 the signer == owner
///   (Decision 4): the browser's keypair and the JWT.sub are the same identity.
/// - `keypair_json` — the user's localStorage keypair as the standard
///   `KeypairJson` shape.
///
/// Returns a single `Uint8Array` containing the COSE_Sign1 bytes. The caller
/// POSTs this to `/api/sign-callback` along with `correlation_id` and
/// `signer_pubkey`.
#[wasm_bindgen]
pub fn sign_attestation_bundle(
    content: &str,
    embedding_bytes: &[u8],
    content_hash: &str,
    owner_pubkey: &str,
    keypair_json: JsValue,
) -> Result<Vec<u8>, JsValue> {
    let kpj = keypair_json_from_value(keypair_json)?;
    let kp = keypair_from_json(kpj)?;

    // The artifact_id is derived from the content_hash so the browser can
    // produce a stable identifier without bringing in `uuid` (which would pull
    // additional `getrandom` paths and bloat the wasm binary). Server-side
    // sign_memory uses a UUIDv4 — that's fine: the artifact_id is opaque from
    // the canonical-CBOR perspective, only its presence and stability matter.
    let artifact_id = format!("art:{}", &content_hash.get(..16).unwrap_or(content_hash));
    let now = chrono_now_rfc3339();

    let embedding_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, embedding_bytes);

    let artifact = serde_json::json!({
        "artifact_id": artifact_id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": format!("did:sol:{owner_pubkey}"),
        "created_at": now,
        "metadata": {
            "embedding_compressed": embedding_b64,
        },
    });

    let canonical_cbor = to_canonical_cbor(&artifact, &MEMORY_V1)
        .map_err(|e| JsValue::from_str(&format!("canonical CBOR encoding failed: {e}")))?;

    // Hash recomputed locally — useful as a future cross-check against the
    // server-supplied content_hash, but we do not enforce equality here because
    // the server-built JSON (with full metadata: embed_provider, embed_dim,
    // turbo_bits) may differ from the browser-supplied subset. The validator
    // on the server is the ultimate arbiter (`POST /api/sign-callback`).
    let _local_hash = hash_bytes(&canonical_cbor);

    sign_cose(&canonical_cbor, &kp)
        .map_err(|e| JsValue::from_str(&format!("COSE signing failed: {e}")))
}

/// Wrap server-provided canonical-CBOR payload bytes in a COSE_Sign1 envelope
/// signed by the user's keypair.
///
/// Use this when the canonical-CBOR was BUILT BY THE SERVER (e.g. /api/pending
/// returns the bytes the server stored — full metadata: embed_provider,
/// embed_dim, turbo_bits, etc). Re-deriving the bytes in the browser would
/// produce a DIFFERENT byte sequence (subset of metadata), and the server's
/// `verify_artifact` would fail `content_integrity` because the embedded
/// content_hash references the server's full bytes, not the browser's subset.
///
/// This function wraps `payload` verbatim — it does NOT rebuild CBOR.
/// The COSE_Sign1 protected header carries `alg=EdDSA` and `kid=<pubkey>`,
/// and the signature is computed over `Sig_structure1(protected, payload)`
/// per RFC 8152.
#[wasm_bindgen]
pub fn sign_cose_payload(payload: &[u8], keypair_json: JsValue) -> Result<Vec<u8>, JsValue> {
    let kpj = keypair_json_from_value(keypair_json)?;
    let kp = keypair_from_json(kpj)?;
    sign_cose(payload, &kp).map_err(|e| JsValue::from_str(&format!("COSE signing failed: {e}")))
}

/// Serialize a keypair JSON object to a string for download/backup.
///
/// The returned string is plain JSON — the webapp wraps it in a `Blob` for
/// download. There is no encryption applied at this layer; UX-level encryption
/// (per the Risks table in tech-spec) lives in the webapp.
#[wasm_bindgen]
pub fn export_keypair_json(keypair_json: JsValue) -> Result<String, JsValue> {
    let kpj = keypair_json_from_value(keypair_json)?;
    // Validate roundtrip — refuse to export a corrupted backup.
    let _ = keypair_from_json(KeypairJson {
        secret: kpj.secret.clone(),
        pubkey_base58: kpj.pubkey_base58.clone(),
    })?;
    serde_json::to_string(&kpj)
        .map_err(|e| JsValue::from_str(&format!("keypair JSON serialization failed: {e}")))
}

/// Parse a keypair JSON backup string and return a `KeypairJson` `JsValue`.
///
/// Used by the "Import keypair" flow on `/install`. All failure modes
/// (malformed JSON, wrong secret length, public/private mismatch) become
/// `Err(JsValue)` with a human-readable message — never panic, never `unwrap`.
#[wasm_bindgen]
pub fn import_keypair_json(json_str: &str) -> Result<JsValue, JsValue> {
    let parsed: KeypairJson = serde_json::from_str(json_str)
        .map_err(|e| JsValue::from_str(&format!("malformed keypair JSON: {e}")))?;
    // Validate the secret + pubkey pair before returning to JS.
    let _ = keypair_from_json(KeypairJson {
        secret: parsed.secret.clone(),
        pubkey_base58: parsed.pubkey_base58.clone(),
    })?;
    serde_wasm_bindgen::to_value(&parsed)
        .map_err(|e| JsValue::from_str(&format!("keypair value conversion failed: {e}")))
}

/// RFC-3339 timestamp string for `created_at` fields. Pulled out so the test
/// module can inspect or override it later if needed.
fn chrono_now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::sign::verify_artifact;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Round-trip helper: convert a `JsValue` keypair back to `KeypairJson`.
    fn read_keypair(value: &JsValue) -> KeypairJson {
        serde_wasm_bindgen::from_value::<KeypairJson>(value.clone()).expect("deserialise keypair")
    }

    #[wasm_bindgen_test]
    fn keypair_gen_produces_valid_ed25519() {
        let kp_value = generate_keypair().expect("generate_keypair");
        let kpj = read_keypair(&kp_value);
        assert_eq!(kpj.secret.len(), 64, "ed25519 secret must be 64 bytes");
        let pubkey: Pubkey = kpj.pubkey_base58.parse().expect("base58 pubkey parses");
        assert_eq!(pubkey.to_bytes().len(), 32);
    }

    #[wasm_bindgen_test]
    fn sign_challenge_roundtrip_with_native_verifier() {
        let kp_value = generate_keypair().expect("generate_keypair");
        let kpj = read_keypair(&kp_value);
        let challenge: Vec<u8> = (0u8..32).collect();
        let sig = sign_challenge(kp_value.clone(), &challenge).expect("sign_challenge");
        assert_eq!(sig.len(), 64, "ed25519 sig is 64 bytes");
        let pubkey = Pubkey::from_str(&kpj.pubkey_base58).expect("pubkey parses");
        assert!(
            crate::identity::verify_signature(&pubkey, &challenge, &sig),
            "native verify_signature must accept WASM-produced signature"
        );
    }

    #[wasm_bindgen_test]
    fn json_export_import_preserves_keypair() {
        let kp_value = generate_keypair().expect("generate_keypair");
        let kpj_before = read_keypair(&kp_value);
        let exported = export_keypair_json(kp_value).expect("export");
        let imported = import_keypair_json(&exported).expect("import");
        let kpj_after = read_keypair(&imported);
        assert_eq!(kpj_before.secret, kpj_after.secret);
        assert_eq!(kpj_before.pubkey_base58, kpj_after.pubkey_base58);
    }

    #[wasm_bindgen_test]
    fn repeated_gen_distinct_keys() {
        let a = read_keypair(&generate_keypair().expect("a"));
        let b = read_keypair(&generate_keypair().expect("b"));
        assert_ne!(a.secret, b.secret, "two generated secrets must differ");
        assert_ne!(
            a.pubkey_base58, b.pubkey_base58,
            "two generated pubkeys must differ"
        );
    }

    #[wasm_bindgen_test]
    fn malformed_import_returns_err_not_panic() {
        let err1 = import_keypair_json("not-json").expect_err("non-JSON must error");
        let s1: String = err1.as_string().unwrap_or_default();
        assert!(!s1.is_empty(), "error must carry a message");

        let err2 = import_keypair_json("{}").expect_err("empty object must error");
        let s2: String = err2.as_string().unwrap_or_default();
        assert!(!s2.is_empty());

        // Wrong-shape JSON: secret present but wrong length.
        let err3 = import_keypair_json(r#"{"secret":[1,2,3],"pubkey_base58":"abc"}"#)
            .expect_err("short secret must error");
        let s3: String = err3.as_string().unwrap_or_default();
        assert!(s3.contains("64") || s3.contains("invalid"));
    }

    #[wasm_bindgen_test]
    fn getrandom_non_zero_entropy() {
        // Two back-to-back generates: prove getrandom is wired, entropy is
        // nonzero, and successive calls do not collide. If `getrandom`'s `js`
        // feature was missing we would panic before reaching the asserts.
        let a = read_keypair(&generate_keypair().expect("a"));
        let b = read_keypair(&generate_keypair().expect("b"));
        assert!(a.secret.iter().any(|&b| b != 0), "entropy not all-zero (a)");
        assert!(b.secret.iter().any(|&b| b != 0), "entropy not all-zero (b)");
        assert_ne!(a.secret, b.secret);
    }

    #[wasm_bindgen_test]
    fn sign_attestation_bundle_roundtrip_with_native_verifier() {
        let kp_value = generate_keypair().expect("generate_keypair");
        let kpj = read_keypair(&kp_value);

        let content = "hello mnemonic browser-side signing";
        let embedding_bytes: Vec<u8> = (0u8..16).collect();
        // Pre-compute the content_hash the same way the server would, so the
        // input mirrors the production /api/pending/<id> contract. The WASM
        // function rebuilds canonical CBOR internally — this argument is not
        // currently enforced (see comment in sign_attestation_bundle) but the
        // test still threads the API.
        let content_hash = hash_bytes(content.as_bytes());

        let cose_bytes = sign_attestation_bundle(
            content,
            &embedding_bytes,
            &content_hash,
            &kpj.pubkey_base58,
            kp_value,
        )
        .expect("sign_attestation_bundle");
        assert!(!cose_bytes.is_empty(), "COSE bytes returned");

        // Native verification path — same code an MCP `/api/sign-callback`
        // handler will run server-side.
        let result = verify_artifact(&cose_bytes, None).expect("verify_artifact");
        assert!(result.valid, "valid: {result:?}");
        assert!(result.cose_signature, "cose_signature");
        assert!(result.content_integrity, "content_integrity");
        assert!(result.algorithm_valid, "algorithm_valid");
        assert_eq!(
            result.signer, kpj.pubkey_base58,
            "signer kid matches generated pubkey"
        );
    }
}

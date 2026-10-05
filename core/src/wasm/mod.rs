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
use crate::compress::{CompressedEmbedding, EmbeddingCompressor};
use crate::identity::{pubkey_base58, sign_bytes, verify_signature};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Deterministic seed used by every `EmbeddingCompressor` instance —
/// must match `mcp/src/main.rs:309` and every test compressor in the
/// codebase. Drift here breaks T05 byte parity and recall.
const COMPRESS_SEED: u64 = 42;

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

/// Verify a raw Ed25519 signature produced outside COSE/CBOR.
///
/// Used by the cross-ecosystem parity test (`mcp/tests/erc8004_feedback_parity.rs`)
/// to confirm that a `MNEMONIC_FEEDBACK_V1` proof verifies in both TypeScript
/// (`@noble/ed25519`) and Rust (`identity::verify_signature`).
///
/// Returns `true` when valid, `false` on any mismatch or parse failure (never panics).
#[wasm_bindgen]
pub fn verify_ed25519(pubkey_b58: &str, message: &[u8], signature: &[u8]) -> bool {
    let Ok(pubkey) = Pubkey::from_str(pubkey_b58) else {
        return false;
    };
    verify_signature(&pubkey, message, signature)
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

// ── T05: extension crypto-pipeline bindings ─────────────────────────────────
//
// Exports added so a Chrome MV3 service worker can run the *exact same*
// embedding-compress / canonical-CBOR / blake3 / COSE pipeline the server
// runs natively. Byte parity is enforced by the golden fixtures emitted
// from `core/tests/golden_fixtures.rs::emit_extension_fixtures` and
// consumed from `packages/extension/tests/unit/sign/cose.test.ts`.

/// Compress an f32 embedding via TurboQuant and return the serialized bytes.
///
/// Uses the codebase-wide deterministic seed (42) and the bit-width supplied
/// by the caller. Bit-width MUST match `mcp/src/config.rs::turbo_bits` (4 by
/// default) — drift makes server-stored embeddings incomparable to
/// browser-rebuilt ones at recall time.
#[wasm_bindgen]
pub fn compress_embedding(embedding: &[f32], bit_width: u8) -> Result<Vec<u8>, JsValue> {
    if embedding.is_empty() {
        return Err(JsValue::from_str(
            "compress_embedding: embedding must not be empty",
        ));
    }
    let bits = bit_width as usize;
    if !(2..=8).contains(&bits) {
        return Err(JsValue::from_str(&format!(
            "compress_embedding: bit_width must be 2..=8, got {bits}"
        )));
    }
    let compressor = EmbeddingCompressor::new(embedding.len(), bits, COMPRESS_SEED);
    let compressed = compressor.compress(embedding);
    Ok(compressed.to_bytes())
}

/// Decompress TurboQuant bytes back to an approximate f32 embedding.
///
/// `dim` is required because `EmbeddingCompressor::new(...)` is dimension-
/// specific — the same bytes decompress to different vectors under
/// different `dim` configurations. The browser must know the dimension
/// from the artifact's `metadata.embed_dim` field (server-set; 384 for
/// the default `Xenova/all-MiniLM-L6-v2` per Decision 3).
#[wasm_bindgen]
pub fn decompress_embedding(bytes: &[u8], dim: usize) -> Result<Vec<f32>, JsValue> {
    let compressed = CompressedEmbedding::from_bytes(bytes)
        .ok_or_else(|| JsValue::from_str("decompress_embedding: malformed compressed bytes"))?;
    if compressed.dim != dim {
        return Err(JsValue::from_str(&format!(
            "decompress_embedding: dim mismatch — bytes report {}, caller passed {}",
            compressed.dim, dim
        )));
    }
    let compressor = EmbeddingCompressor::new(dim, compressed.bit_width, COMPRESS_SEED);
    Ok(compressor.decompress(&compressed))
}

/// Canonicalize a JS object to CBOR bytes per `MEMORY_V1` schema.
///
/// Same byte sequence the server emits via `codec::canonical::to_canonical_cbor`.
/// The caller passes a plain JS object whose fields match `MEMORY_V1`'s
/// `cbor_field_order`; missing optional fields are dropped (consistent with
/// the native path's null-omission rule).
#[wasm_bindgen]
pub fn to_canonical_cbor_bytes(value: JsValue) -> Result<Vec<u8>, JsValue> {
    let json: serde_json::Value = serde_wasm_bindgen::from_value(value).map_err(|e| {
        JsValue::from_str(&format!("to_canonical_cbor_bytes: invalid JS value: {e}"))
    })?;
    to_canonical_cbor(&json, &MEMORY_V1)
        .map_err(|e| JsValue::from_str(&format!("to_canonical_cbor_bytes: {e}")))
}

/// Compute the raw 32-byte blake3 digest of `bytes`.
///
/// Returns the binary digest, not hex — the extension renders hex only at
/// display boundaries (web-app parity). For a hex string call
/// `core::codec::hash::hash_bytes` natively.
#[wasm_bindgen]
pub fn blake3_hash(bytes: &[u8]) -> Vec<u8> {
    blake3::hash(bytes).as_bytes().to_vec()
}

/// Hex string of the blake3 digest. Convenience export so the browser can
/// echo the exact `content_hash` string the server stored without
/// re-implementing hex encoding in TS.
#[wasm_bindgen]
pub fn blake3_hash_hex(bytes: &[u8]) -> String {
    hash_bytes(bytes)
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

    /// Wave 2 of work/arweave-as-source-of-truth: a browser client must be able
    /// to rebuild a row from Arweave bytes on its own. This signs an artifact in
    /// the WASM build, rebuilds it through the exported entry point, and checks
    /// the recovered fields — including that the seed came from the artifact
    /// rather than a hardcoded default.
    #[wasm_bindgen_test]
    fn rebuild_row_reconstructs_a_row_in_the_browser() {
        use crate::compress::EmbeddingCompressor;
        use base64::Engine as _;

        let dim = 8usize;
        let bits = 4usize;
        let seed = 9_001u64; // deliberately not the protocol default
        let embedding: Vec<f32> = (0..dim).map(|i| i as f32 * 0.25 - 1.0).collect();

        let kp_value = generate_keypair().expect("generate_keypair");
        let kpj = read_keypair(&kp_value);
        let signer_b58 = kpj.pubkey_base58.clone();
        let kp = keypair_from_json(kpj).expect("keypair from json");

        let c = EmbeddingCompressor::new(dim, bits, seed);
        let b64 =
            base64::engine::general_purpose::STANDARD.encode(c.compress(&embedding).to_bytes());
        let artifact = serde_json::json!({
            "artifact_id": "att-wasm",
            "type": "memory",
            "schema_version": 1,
            "content": "rebuilt in the browser",
            "producer": crate::identity::did_sol(&kp),
            "created_at": "2026-09-27T00:00:00Z",
            "tags": ["wasm"],
            "visibility": "public",
            "anchor": "arweave",
            "metadata": {
                "embed_provider": "stub",
                "embed_dim": dim,
                "turbo_bits": bits,
                "turbo_seed": seed,
                "embedding_compressed": b64,
            },
        });
        let signed =
            crate::codec::sign::sign_artifact(&artifact, &crate::codec::schema::MEMORY_V1, &kp)
                .expect("sign artifact");

        let value = rebuild_row(&signed.cose_bytes).expect("rebuild_row must succeed in wasm");
        let row: crate::wasm::RebuiltRowJs =
            serde_wasm_bindgen::from_value(value).expect("row deserialises");

        assert_eq!(row.attestation_id, "att-wasm");
        assert_eq!(row.content, "rebuilt in the browser");
        assert_eq!(row.content_hash, signed.content_hash);
        assert_eq!(row.tags, vec!["wasm".to_string()]);
        assert_eq!(row.signer_pubkey, signer_b58);
        assert_eq!(row.precision, "compressed");
        assert_eq!(
            row.embedding,
            c.decompress(&c.compress(&embedding)),
            "the seed must be read from the artifact, not assumed"
        );
    }

    /// A tampered artifact must be refused in the browser too.
    #[wasm_bindgen_test]
    fn rebuild_row_rejects_a_tampered_artifact() {
        use crate::compress::EmbeddingCompressor;
        use base64::Engine as _;

        let kp_value = generate_keypair().expect("generate_keypair");
        let kpj = read_keypair(&kp_value);
        let kp = keypair_from_json(kpj).expect("keypair from json");
        let c = EmbeddingCompressor::new(8, 4, 42);
        let b64 = base64::engine::general_purpose::STANDARD
            .encode(c.compress(&vec![0.1f32; 8]).to_bytes());
        let artifact = serde_json::json!({
            "artifact_id": "att-bad",
            "type": "memory",
            "schema_version": 1,
            "content": "tamper me",
            "producer": crate::identity::did_sol(&kp),
            "created_at": "2026-09-27T00:00:00Z",
            "metadata": { "embed_dim": 8, "turbo_bits": 4, "embedding_compressed": b64 },
        });
        let mut cose =
            crate::codec::sign::sign_artifact(&artifact, &crate::codec::schema::MEMORY_V1, &kp)
                .expect("sign")
                .cose_bytes;
        let last = cose.len() - 1;
        cose[last] ^= 0xff;
        assert!(
            rebuild_row(&cose).is_err(),
            "a tampered artifact must not rebuild"
        );
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

/// One row rebuilt from a signed artifact, shaped for JavaScript.
///
/// Mirrors [`crate::rebuild::RebuiltRow`] but with camelCase keys and the
/// precision tier as a string, so the SDK consumes it without a mapping layer.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RebuiltRowJs {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub tags: Vec<String>,
    pub owner_pubkey: String,
    pub signer_pubkey: String,
    pub created_at: String,
    pub embedding: Vec<f32>,
    /// `"f32"` when the artifact carried the exact vector, `"compressed"` when
    /// only the lossy TurboQuant copy was available. A client showing search
    /// quality should surface the difference rather than hide it.
    pub precision: String,
}

// ── T5: Sealed-memory WASM bindings ─────────────────────────────────────────
//
// These exports expose the T3 sealed-memory API (`core::sealed`) to JavaScript.
// All secret inputs (x25519_secret, k) arrive as `&[u8]` and are copied into
// `Zeroizing<Vec<u8>>` so the Rust-side copy is wiped when it goes out of scope.
// Errors are converted to `JsValue::from_str` at this boundary only — no
// business logic lives here.

/// Seal an inner memory JSON blob for the given author.
///
/// Generates a fresh content key `K` and nonce internally (WebCrypto via
/// `getrandom` `js` feature).  Returns a JS object:
/// `{ outer_cbor: Uint8Array, content_hash: Uint8Array }`.
/// `K` is intentionally NOT returned — callers that need it (for grants /
/// bearer links) must call `make_grant` or `link_fragment` in a single
/// server-side call before discarding the key.
///
/// # Parameters
/// - `inner_json`         — raw bytes of the inner memory artifact JSON.
/// - `author_ed25519_pub` — author's Ed25519 public key (32 bytes).
/// - `artifact_id`        — the outer artifact's unique ID string.
/// - `producer`           — the author's DID / identity string.
/// - `created_at`         — ISO 8601 creation timestamp.
#[wasm_bindgen]
pub fn seal_memory(
    inner_json: &[u8],
    author_ed25519_pub: &[u8],
    artifact_id: &str,
    producer: &str,
    created_at: &str,
) -> Result<JsValue, JsValue> {
    let pub_bytes: [u8; 32] = author_ed25519_pub.try_into().map_err(|_| {
        JsValue::from_str(&format!(
            "seal_memory: author_ed25519_pub must be 32 bytes, got {}",
            author_ed25519_pub.len()
        ))
    })?;

    let mut rng = rand_core::OsRng;
    let artifact = crate::sealed::seal_memory(
        inner_json,
        &pub_bytes,
        artifact_id,
        producer,
        created_at,
        &mut rng,
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))?;

    #[derive(serde::Serialize)]
    struct SealResult {
        outer_cbor: Vec<u8>,
        content_hash: Vec<u8>,
    }
    let result = SealResult {
        outer_cbor: artifact.outer_cbor,
        content_hash: artifact.content_hash,
    };
    serde_wasm_bindgen::to_value(&result)
        .map_err(|e| JsValue::from_str(&format!("seal_memory: serialise failed: {e}")))
}

/// Open a sealed memory using the recipient's X25519 secret key (32 bytes).
///
/// Returns the inner memory JSON as a `Uint8Array`.
/// The `x25519_secret` copy is zeroized after use.
#[wasm_bindgen]
pub fn open_memory(outer_cbor: &[u8], x25519_secret: &[u8]) -> Result<Vec<u8>, JsValue> {
    let secret = zeroize::Zeroizing::new(x25519_secret.to_vec());
    let key: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
        JsValue::from_str(&format!(
            "open_memory: x25519_secret must be 32 bytes, got {}",
            x25519_secret.len()
        ))
    })?;
    crate::sealed::open_memory(outer_cbor, &key).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Open a sealed memory when `K` is already known (bearer links / grants).
///
/// Returns the inner memory JSON as a `Uint8Array`.
/// The `k` copy is zeroized after use.
#[wasm_bindgen]
pub fn open_with_key(outer_cbor: &[u8], k: &[u8]) -> Result<Vec<u8>, JsValue> {
    let secret = zeroize::Zeroizing::new(k.to_vec());
    let key: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
        JsValue::from_str(&format!(
            "open_with_key: k must be 32 bytes, got {}",
            k.len()
        ))
    })?;
    crate::sealed::open_with_key(outer_cbor, &key).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Build an unsigned GRANT_V1 CBOR artifact.
///
/// - `memory_hash` — 32-byte hash of the sealed artifact this grant unlocks.
/// - `outer_cbor`  — the SEALED_V1 CBOR bytes (used to extract `producer`).
/// - `k`           — 32-byte content encryption key (zeroized after use).
/// - `reader_pk`   — optional 32-byte X25519 public key for a targeted grant;
///                   `undefined` / absent for an anonymous (bearer) grant.
/// - `author_did`  — the author's DID string.
/// - `perms`       — optional JSON permission token / expiry string.
/// - `created_at`  — ISO 8601 timestamp.
///
/// Returns the raw GRANT_V1 CBOR bytes as a `Uint8Array`.
#[wasm_bindgen]
pub fn make_grant(
    memory_hash: &[u8],
    outer_cbor: &[u8],
    k: &[u8],
    reader_pk: Option<Vec<u8>>,
    author_did: &str,
    perms: Option<String>,
    created_at: &str,
) -> Result<Vec<u8>, JsValue> {
    let k_secret = zeroize::Zeroizing::new(k.to_vec());
    let k_arr: [u8; 32] = k_secret.as_slice().try_into().map_err(|_| {
        JsValue::from_str(&format!("make_grant: k must be 32 bytes, got {}", k.len()))
    })?;
    let hash_arr: [u8; 32] = memory_hash.try_into().map_err(|_| {
        JsValue::from_str(&format!(
            "make_grant: memory_hash must be 32 bytes, got {}",
            memory_hash.len()
        ))
    })?;

    let reader_pk_arr: Option<[u8; 32]> = match reader_pk {
        Some(ref rpk) => {
            let arr: [u8; 32] = rpk.as_slice().try_into().map_err(|_| {
                JsValue::from_str(&format!(
                    "make_grant: reader_pk must be 32 bytes, got {}",
                    rpk.len()
                ))
            })?;
            Some(arr)
        }
        None => None,
    };

    crate::sealed::make_grant(
        &hash_arr,
        outer_cbor,
        &k_arr,
        reader_pk_arr.as_ref(),
        author_did,
        perms.as_deref(),
        created_at,
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Extract the content key `K` from a GRANT_V1 CBOR artifact for a targeted
/// recipient.
///
/// The `x25519_secret` copy is zeroized after use.
/// Returns the raw 32-byte `K` as a `Uint8Array`.
#[wasm_bindgen]
pub fn open_grant(grant_cbor: &[u8], x25519_secret: &[u8]) -> Result<Vec<u8>, JsValue> {
    let secret = zeroize::Zeroizing::new(x25519_secret.to_vec());
    let key: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
        JsValue::from_str(&format!(
            "open_grant: x25519_secret must be 32 bytes, got {}",
            x25519_secret.len()
        ))
    })?;
    let k = crate::sealed::open_grant(grant_cbor, &key)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(k.to_vec())
}

/// Derive the X25519 public key from a raw Ed25519 public-key byte array (32 bytes).
///
/// Returns the 32-byte X25519 public key as a `Uint8Array`.
#[wasm_bindgen]
pub fn x25519_public_from_ed25519(ed25519_pub: &[u8]) -> Result<Vec<u8>, JsValue> {
    let pub_bytes: [u8; 32] = ed25519_pub.try_into().map_err(|_| {
        JsValue::from_str(&format!(
            "x25519_public_from_ed25519: ed25519_pub must be 32 bytes, got {}",
            ed25519_pub.len()
        ))
    })?;
    crate::sealed::x25519_public_from_ed25519(&pub_bytes)
        .map(|k| k.to_vec())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Encode the 32-byte content key `K` as a URL fragment: `k=<base64url-nopad>`.
///
/// Returns the fragment string (without the leading `#`).
#[wasm_bindgen]
pub fn link_fragment(k: &[u8]) -> String {
    match k.try_into() as Result<[u8; 32], _> {
        Ok(arr) => crate::sealed::link_fragment(&arr),
        Err(_) => format!("link_fragment: k must be 32 bytes, got {}", k.len()),
    }
}

/// Parse a URL fragment of the form `k=<base64url-nopad>` back to a 32-byte `K`.
///
/// Returns the raw 32 key bytes as a `Uint8Array`.
#[wasm_bindgen]
pub fn parse_link_fragment(fragment: &str) -> Result<Vec<u8>, JsValue> {
    crate::sealed::parse_link_fragment(fragment)
        .map(|k| k.to_vec())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Verify the outer COSE_Sign1 signature of a `sealed.v1` artifact and extract
/// its metadata.
///
/// Returns a JS object:
/// `{ producer: string, content_hash: Uint8Array, created_at: string, wrap_count: number }`.
///
/// This function NEVER decrypts anything — it only verifies the envelope
/// signature and extracts the header fields.
#[wasm_bindgen]
pub fn verify_sealed(cose_bytes: &[u8]) -> Result<JsValue, JsValue> {
    let sv = crate::codec::sign::verify_sealed(cose_bytes).map_err(|e| JsValue::from_str(&e))?;

    #[derive(serde::Serialize)]
    struct SealedVerificationJs {
        producer: String,
        content_hash: Vec<u8>,
        created_at: String,
        wrap_count: usize,
    }
    let out = SealedVerificationJs {
        producer: sv.producer,
        content_hash: sv.content_hash,
        created_at: sv.created_at,
        wrap_count: sv.wrap_count,
    };
    serde_wasm_bindgen::to_value(&out)
        .map_err(|e| JsValue::from_str(&format!("verify_sealed: serialise failed: {e}")))
}

/// Rebuild one recall row from the signed artifact bytes stored on Arweave.
///
/// This is the client half of "Arweave is the source of truth": given the
/// COSE_Sign1 bytes of an item, the browser reconstructs the row — content,
/// tags, author, hash and embedding — with no server involved. Enumeration of
/// *which* items to fetch stays in JavaScript (the Arweave gateway GraphQL and
/// Solana memo history are plain HTTP), because the Rust discovery helpers need
/// `reqwest` and are native-only.
///
/// The signature is verified before any field is read, so a tampered or
/// unsigned blob can never inject a row. Unlike [`decompress_embedding`], this
/// takes no `dim` and no seed: it reads `metadata.embed_dim`, `turbo_bits` and
/// `turbo_seed` from the artifact itself, falling back to the legacy seed for
/// items written before that field existed. Callers therefore need no
/// out-of-band constant.
#[wasm_bindgen]
pub fn rebuild_row(cose_bytes: &[u8]) -> Result<JsValue, JsValue> {
    let row = crate::rebuild::rebuild_row_self_describing(cose_bytes)
        .map_err(|e| JsValue::from_str(&format!("rebuild_row: {e}")))?;
    let out = RebuiltRowJs {
        attestation_id: row.attestation_id,
        content: row.content,
        content_hash: row.content_hash,
        tags: row.tags,
        owner_pubkey: row.owner_pubkey,
        signer_pubkey: row.signer_pubkey,
        created_at: row.created_at,
        embedding: row.embedding,
        precision: match row.precision {
            crate::rebuild::Precision::F32 => "f32".to_string(),
            crate::rebuild::Precision::Compressed => "compressed".to_string(),
        },
    };
    serde_wasm_bindgen::to_value(&out)
        .map_err(|e| JsValue::from_str(&format!("rebuild_row: serialise failed: {e}")))
}

/// Client-sign plain or sealed A2A bindings. Recipients are pinned, signed cards.
#[cfg(feature = "a2a-experimental")]
#[wasm_bindgen]
pub fn prepare_a2a(
    keypair_json: JsValue,
    kind: &str,
    payload_json: &str,
    context_id: &str,
    prev_id: Option<String>,
    created_at: &str,
    recipients_json: Option<String>,
    chunk_size: Option<u32>,
) -> Result<Vec<u8>, JsValue> {
    if payload_json.len() > crate::codec::a2a::signed::MAX_A2A_BYTES / 2
        || recipients_json
            .as_ref()
            .is_some_and(|s| s.len() > crate::codec::a2a::signed::MAX_A2A_BYTES)
    {
        return Err(JsValue::from_str("A2A input too large"));
    }
    let kp = keypair_from_json(keypair_json_from_value(keypair_json)?)?;
    let payload =
        serde_json::from_str(payload_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let recipients: Option<Vec<crate::codec::a2a::signed::RecipientCard>> = recipients_json
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let result = if let Some(size) = chunk_size {
        let recipients = recipients
            .as_ref()
            .ok_or_else(|| JsValue::from_str("stream recipients required"))?;
        crate::codec::a2a::signed::prepare_signed_a2a_stream(
            &kp,
            kind,
            payload,
            context_id,
            prev_id,
            created_at,
            recipients,
            size as usize,
        )
    } else {
        crate::codec::a2a::signed::prepare_signed_a2a(
            &kp,
            kind,
            payload,
            context_id,
            prev_id,
            created_at,
            recipients.as_deref(),
        )
    };
    result.map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Verify every signature and sealed chain, then open locally for this identity.
/// Returns JSON `{payload, inner_signed}`.
#[cfg(feature = "a2a-experimental")]
#[wasm_bindgen]
pub fn open_a2a(
    keypair_json: JsValue,
    signed: &[u8],
    expected_author: &str,
    encryption_secret: Option<Vec<u8>>,
) -> Result<String, JsValue> {
    let kp = keypair_from_json(keypair_json_from_value(keypair_json)?)?;
    let secret = encryption_secret.map(zeroize::Zeroizing::new);
    let arr = secret
        .as_ref()
        .map(|s| s.as_slice().try_into())
        .transpose()
        .map_err(|_| JsValue::from_str("encryption secret must be 32 bytes"))?;
    let opened = crate::codec::a2a::signed::open_signed_a2a_full(signed, &kp, expected_author, arr)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    // `inner_signed` is the author's COSE over plaintext and recipients (hex), or null.
    serde_json::to_string(&serde_json::json!({
        "payload": opened.payload,
        "inner_signed": opened.inner_signed.map(hex::encode),
    }))
    .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Verify an inner sign-encrypt-sign binding that a reader presents.
#[cfg(feature = "a2a-experimental")]
#[wasm_bindgen]
pub fn verify_a2a_inner(inner_signed: &[u8], expected_author: &str) -> Result<String, JsValue> {
    let inner = crate::codec::a2a::signed::verify_a2a_inner(inner_signed, expected_author)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(&inner).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Verify public signatures and the sealed hash chain, without decrypting.
#[cfg(feature = "a2a-experimental")]
#[wasm_bindgen]
pub fn verify_a2a(signed: &[u8], expected_author: &str) -> Result<String, JsValue> {
    let v = crate::codec::a2a::signed::verify_signed_a2a(signed, Some(expected_author))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(
        &serde_json::json!({"binding":v.binding,"signer":v.signer,"content_hash":v.content_hash}),
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Derive the scalar using the same core function as native recipients.
#[wasm_bindgen]
pub fn x25519_secret_from_seed(seed: &[u8]) -> Result<Vec<u8>, JsValue> {
    let bytes: [u8; 32] = seed
        .try_into()
        .map_err(|_| JsValue::from_str("seed must be 32 bytes"))?;
    let key = ed25519_dalek::SigningKey::from_bytes(&bytes);
    Ok(crate::sealed::x25519_secret_from_ed25519(&key).to_vec())
}

/// Check signed lineage without decrypting or trusting local metadata.
#[cfg(feature = "a2a-experimental")]
#[wasm_bindgen]
pub fn verify_a2a_parent(child_hex: &str, parent_hex: &str) -> Result<(), JsValue> {
    use crate::codec::a2a::signed::{verify_parent_link, verify_signed_a2a};
    let child = hex::decode(child_hex).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let parent = hex::decode(parent_hex).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let child = verify_signed_a2a(&child, None).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let parent = verify_signed_a2a(&parent, None).map_err(|e| JsValue::from_str(&e.to_string()))?;
    verify_parent_link(&child, &parent).map_err(|e| JsValue::from_str(&e.to_string()))
}

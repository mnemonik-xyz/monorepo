//! High-level seal / open / grant / link API (Task 3).
//!
//! Composes the three low-level modules (`keys`, `content`, `wrap`) into the
//! operations used by the server, WASM client, and SDK:
//!
//! * [`seal_memory`]      — encrypt a memory JSON blob, wrap K for the author.
//! * [`open_memory`]      — unwrap K with the recipient's X25519 secret, decrypt.
//! * [`open_with_key`]    — decrypt when K is already known (bearer links/grants).
//! * [`make_grant`]       — build an unsigned GRANT_V1 CBOR artifact.
//! * [`open_grant`]       — extract K from a GRANT_V1 for a recipient.
//! * [`link_fragment`]    — encode K as a URL fragment `k=<base64url>`.
//! * [`parse_link_fragment`] — decode the URL fragment back to K.

use base64::Engine as _;
use rand_core::RngCore;
use zeroize::Zeroizing;

use crate::codec::canonical::to_canonical_cbor;
use crate::codec::schema::{GRANT_V1, SEALED_V1};
use crate::sealed::content::{
    decrypt_content, encrypt_content, key_commitment, pad_to_bucket, unpad,
};
use crate::sealed::keys::x25519_public_from_ed25519;
use crate::sealed::wrap::{unwrap_key, wrap_key};

/// Errors from the high-level seal API.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// CBOR encoding / schema error.
    #[error("CBOR codec error: {0}")]
    Codec(String),
    /// Content encryption or decryption failed.
    #[error("content error: {0}")]
    Content(#[from] crate::sealed::content::ContentError),
    /// Key-wrap / unwrap failed.
    #[error("wrap error: {0}")]
    Wrap(#[from] crate::sealed::wrap::WrapError),
    /// Key conversion (Ed25519 → X25519) failed.
    #[error("key conversion error: {0}")]
    KeyConvert(#[from] crate::sealed::keys::KeyError),
    /// The caller is not the intended recipient.
    #[error("not a recipient for this sealed memory")]
    NotARecipient,
    /// The CBOR payload is missing a required field or is malformed.
    #[error("malformed sealed artifact: {0}")]
    Malformed(String),
    /// The key commitment in the artifact does not match the unwrapped key.
    #[error("key commitment mismatch — key does not match sealed content")]
    CommitmentMismatch,
    /// Link fragment does not have the expected `k=` prefix.
    #[error("invalid link fragment: expected `k=<base64url>`, got `{0}`")]
    BadLinkFragment(String),
    /// Base64 decode error in link fragment.
    #[error("base64 decode error in link fragment: {0}")]
    BadBase64(String),
    /// Wrong length of key material.
    #[error("wrong key length: expected 32 bytes, got {0}")]
    WrongKeyLen(usize),
}

impl From<String> for SealError {
    fn from(s: String) -> Self {
        SealError::Codec(s)
    }
}

// ─── SealedArtifact ──────────────────────────────────────────────────────────

/// Output of [`seal_memory`].
///
/// The caller MUST sign `outer_cbor` with COSE_Sign1 (Ed25519) before storing
/// or publishing it.  `K` is returned so the caller can also build bearer links
/// or grant artifacts immediately after sealing.
pub struct SealedArtifact {
    /// Canonical CBOR payload of the SEALED_V1 artifact (unsigned; ready to be
    /// wrapped in COSE_Sign1 by the caller).
    pub outer_cbor: Vec<u8>,
    /// blake3 hash of `outer_cbor` (the value that goes on-chain / into Arweave).
    pub content_hash: Vec<u8>,
    /// The 32-byte symmetric content-encryption key.  Kept in [`Zeroizing`] so
    /// it is wiped when dropped.
    pub k: Zeroizing<[u8; 32]>,
}

// ─── seal_memory ─────────────────────────────────────────────────────────────

/// Seal an inner memory JSON blob.
///
/// # Parameters
/// - `inner_memory_json` — raw bytes of the inner memory artifact JSON.
/// - `author_ed25519_pub` — the author's Ed25519 public key (32 bytes).
/// - `artifact_id` — the outer artifact's ID (copied into the SEALED_V1 payload).
/// - `producer` — the author's DID / identity string.
/// - `created_at` — ISO 8601 creation timestamp.
/// - `rng` — a cryptographically secure random-number generator.
///
/// # Returns
/// A [`SealedArtifact`] whose `outer_cbor` must be signed before storage.
pub fn seal_memory(
    inner_memory_json: &[u8],
    author_ed25519_pub: &[u8; 32],
    artifact_id: &str,
    producer: &str,
    created_at: &str,
    rng: &mut impl RngCore,
) -> Result<SealedArtifact, SealError> {
    // 1. Generate K (32 random bytes) and nonce (24 random bytes).
    let mut k_bytes = [0u8; 32];
    rng.fill_bytes(&mut k_bytes);
    let k = Zeroizing::new(k_bytes);

    let mut nonce = [0u8; 24];
    rng.fill_bytes(&mut nonce);

    // 2. Pad + encrypt inner JSON.
    let padded = pad_to_bucket(inner_memory_json);
    let ct = encrypt_content(&k, &nonce, b"", &padded)?;

    // 3. Compute kc = key_commitment(K).
    let kc = key_commitment(&k);

    // 4. Derive recipient X25519 pub from author_ed25519_pub.
    let x25519_pub = x25519_public_from_ed25519(author_ed25519_pub)?;

    // 5. Compute ct_hash for the HPKE AAD (blake3 of ciphertext).
    let ct_hash_arr: [u8; 32] = *blake3::hash(&ct).as_bytes();

    // 6. Wrap K for the author.
    let wrap = wrap_key(&k, &x25519_pub, &ct_hash_arr, producer)?;

    // 7. Build the SEALED_V1 JSON (bytes_fields nonce/ct/kc as base64).
    let nonce_b64 = base64::engine::general_purpose::STANDARD.encode(nonce);
    let ct_b64 = base64::engine::general_purpose::STANDARD.encode(&ct);
    let kc_b64 = base64::engine::general_purpose::STANDARD.encode(kc);
    let enc_b64 = base64::engine::general_purpose::STANDARD.encode(&wrap.enc);
    let wk_b64 = base64::engine::general_purpose::STANDARD.encode(&wrap.wk);

    let artifact = serde_json::json!({
        "artifact_id": artifact_id,
        "type": "sealed",
        "schema_version": 1,
        "alg": "XChaCha20Poly1305",
        "nonce": nonce_b64,
        "ct": ct_b64,
        "kc": kc_b64,
        // wraps carries the HPKE enc+wk as a JSON object for the author slot
        "wraps": [{
            "enc": enc_b64,
            "wk": wk_b64,
        }],
        "created_at": created_at,
        "producer": producer,
    });

    // 8. Encode to canonical CBOR.
    let outer_cbor = to_canonical_cbor(&artifact, &SEALED_V1)?;

    // 9. Compute content_hash (blake3 of the canonical CBOR bytes).
    let content_hash = blake3::hash(&outer_cbor).as_bytes().to_vec();

    Ok(SealedArtifact {
        outer_cbor,
        content_hash,
        k,
    })
}

// ─── open helpers ────────────────────────────────────────────────────────────

/// Parse the canonical CBOR of a SEALED_V1 artifact back to a JSON value.
fn parse_sealed_cbor(outer_cbor: &[u8]) -> Result<serde_json::Value, SealError> {
    crate::codec::canonical::from_canonical_cbor(outer_cbor).map_err(SealError::Malformed)
}

/// Extract a base64-decoded bytes field from the parsed JSON payload.
fn get_bytes_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Vec<u8>, SealError> {
    let b64 = obj
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| SealError::Malformed(format!("missing or non-string field '{field}'")))?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| SealError::Malformed(format!("field '{field}' is not valid base64: {e}")))
}

// ─── open_memory ─────────────────────────────────────────────────────────────

/// Open a sealed memory using the recipient's X25519 secret key.
///
/// Parses the SEALED_V1 CBOR, finds a `wraps` entry whose `enc`/`wk` can be
/// successfully unwrapped with `x25519_secret`, verifies `kc`, decrypts, and
/// unpads the inner memory JSON.
pub fn open_memory(outer_cbor: &[u8], x25519_secret: &[u8; 32]) -> Result<Vec<u8>, SealError> {
    let payload = parse_sealed_cbor(outer_cbor)?;
    let obj = payload
        .as_object()
        .ok_or_else(|| SealError::Malformed("not a CBOR map".into()))?;

    let nonce_bytes = get_bytes_field(obj, "nonce")?;
    let ct_bytes = get_bytes_field(obj, "ct")?;
    let kc_bytes = get_bytes_field(obj, "kc")?;

    let producer = obj
        .get("producer")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SealError::Malformed("missing 'producer'".into()))?;

    // ct_hash used in HPKE AAD (blake3 of raw ciphertext bytes).
    let ct_hash: [u8; 32] = *blake3::hash(&ct_bytes).as_bytes();

    // Try each entry in `wraps`.
    let wraps = obj
        .get("wraps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| SealError::Malformed("missing or non-array 'wraps'".into()))?;

    let mut unwrapped_k: Option<Zeroizing<[u8; 32]>> = None;
    for entry in wraps {
        let entry_obj = match entry.as_object() {
            Some(o) => o,
            None => continue,
        };
        let enc_b64 = match entry_obj.get("enc").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let wk_b64 = match entry_obj.get("wk").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let enc = match base64::engine::general_purpose::STANDARD.decode(enc_b64) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let wk = match base64::engine::general_purpose::STANDARD.decode(wk_b64) {
            Ok(b) => b,
            Err(_) => continue,
        };
        match unwrap_key(&enc, &wk, x25519_secret, &ct_hash, producer) {
            Ok(k) => {
                unwrapped_k = Some(k);
                break;
            }
            Err(_) => continue,
        }
    }

    let k = unwrapped_k.ok_or(SealError::NotARecipient)?;

    // Verify key commitment.
    let expected_kc = key_commitment(&k);
    if kc_bytes != expected_kc {
        return Err(SealError::CommitmentMismatch);
    }

    // Decrypt and unpad.
    let nonce: [u8; 24] = nonce_bytes
        .try_into()
        .map_err(|_| SealError::Malformed("nonce is not 24 bytes".into()))?;
    let padded = decrypt_content(&k, &nonce, b"", &ct_bytes)?;
    let inner = unpad(&padded)?;
    Ok(inner)
}

// ─── open_with_key ───────────────────────────────────────────────────────────

/// Open a sealed memory when `K` is already known (bearer links / grants).
pub fn open_with_key(outer_cbor: &[u8], k: &[u8; 32]) -> Result<Vec<u8>, SealError> {
    let payload = parse_sealed_cbor(outer_cbor)?;
    let obj = payload
        .as_object()
        .ok_or_else(|| SealError::Malformed("not a CBOR map".into()))?;

    let nonce_bytes = get_bytes_field(obj, "nonce")?;
    let ct_bytes = get_bytes_field(obj, "ct")?;
    let kc_bytes = get_bytes_field(obj, "kc")?;

    // Verify key commitment.
    let expected_kc = key_commitment(k);
    if kc_bytes != expected_kc {
        return Err(SealError::CommitmentMismatch);
    }

    let nonce: [u8; 24] = nonce_bytes
        .try_into()
        .map_err(|_| SealError::Malformed("nonce is not 24 bytes".into()))?;
    let padded = decrypt_content(k, &nonce, b"", &ct_bytes)?;
    let inner = unpad(&padded)?;
    Ok(inner)
}

// ─── make_grant ──────────────────────────────────────────────────────────────

/// Build an unsigned GRANT_V1 CBOR artifact.
///
/// If `reader_pk` is `Some`, the content key `k` is wrapped for that recipient
/// (targeted grant). If `None`, an anonymous (bearer) grant is produced — the
/// `reader` field is omitted and `enc`/`wk` carry a bearer-key encoding (K is
/// stored as `enc` directly, `wk` is empty).
///
/// # Parameters
/// - `memory_hash`  — 32-byte hash of the sealed artifact this grant unlocks.
/// - `outer_cbor`   — the SEALED_V1 CBOR bytes (needed to extract `producer`).
/// - `k`            — the 32-byte content encryption key.
/// - `reader_pk`    — optional recipient X25519 public key (targeted grant).
/// - `author_did`   — the author's DID (becomes `producer` in the grant).
/// - `perms`        — optional permission token / expiry JSON string.
/// - `created_at`   — ISO 8601 timestamp.
pub fn make_grant(
    memory_hash: &[u8; 32],
    outer_cbor: &[u8],
    k: &[u8; 32],
    reader_pk: Option<&[u8; 32]>,
    author_did: &str,
    perms: Option<&str>,
    created_at: &str,
) -> Result<Vec<u8>, SealError> {
    let memory_hash_b64 = base64::engine::general_purpose::STANDARD.encode(memory_hash);

    let (enc_b64, wk_b64, reader_did): (String, String, Option<String>) =
        if let Some(rpk) = reader_pk {
            // Targeted grant: wrap K for the reader.
            // Use memory_hash as ct_hash in the HPKE AAD so that open_grant can
            // reconstruct the same AAD from the grant alone (no outer_cbor needed).
            let _ = outer_cbor; // not needed for AAD

            // reader_pk is an X25519 public key (32 bytes), not Ed25519.
            let wrap = wrap_key(k, rpk, memory_hash, author_did)?;

            let enc = base64::engine::general_purpose::STANDARD.encode(&wrap.enc);
            let wk = base64::engine::general_purpose::STANDARD.encode(&wrap.wk);
            // We don't have the reader DID here — caller must set it separately.
            // For now use a placeholder that the caller can replace, but the spec
            // says `reader` is optional, so we omit it when we don't know it.
            (enc, wk, None)
        } else {
            // Anonymous grant: store K directly as `enc`, empty `wk`.
            let enc = base64::engine::general_purpose::STANDARD.encode(k);
            let wk = base64::engine::general_purpose::STANDARD.encode(b"");
            (enc, wk, None)
        };

    let mut artifact = serde_json::json!({
        "type": "grant",
        "schema_version": 1,
        "memory_hash": memory_hash_b64,
        "enc": enc_b64,
        "wk": wk_b64,
        "created_at": created_at,
        "producer": author_did,
    });

    if let Some(r) = reader_did {
        artifact["reader"] = serde_json::Value::String(r);
    }
    if let Some(p) = perms {
        artifact["perms"] = serde_json::Value::String(p.to_string());
    }

    let cbor = to_canonical_cbor(&artifact, &GRANT_V1)?;
    Ok(cbor)
}

// ─── open_grant ──────────────────────────────────────────────────────────────

/// Extract the content key K from a GRANT_V1 artifact for a targeted recipient.
///
/// Parses the GRANT_V1 CBOR, uses the recipient's X25519 secret to unwrap the
/// content key, and returns it wrapped in [`Zeroizing`].
pub fn open_grant(
    grant_cbor: &[u8],
    x25519_secret: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, SealError> {
    let payload =
        crate::codec::canonical::from_canonical_cbor(grant_cbor).map_err(SealError::Malformed)?;
    let obj = payload
        .as_object()
        .ok_or_else(|| SealError::Malformed("not a CBOR map".into()))?;

    let enc = get_bytes_field(obj, "enc")?;
    let wk = get_bytes_field(obj, "wk")?;

    let producer = obj
        .get("producer")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SealError::Malformed("missing 'producer'".into()))?;

    let memory_hash_bytes = get_bytes_field(obj, "memory_hash")?;
    let memory_hash: [u8; 32] = memory_hash_bytes
        .try_into()
        .map_err(|_| SealError::Malformed("memory_hash is not 32 bytes".into()))?;

    // make_grant uses memory_hash as the HPKE AAD's ct_hash so open_grant
    // can reconstruct the same AAD without the outer_cbor.
    // An empty wk signals an anonymous / bearer grant: K is stored in enc.
    if wk.is_empty() {
        // Anonymous / bearer grant: K is stored directly in `enc`.
        if enc.len() != 32 {
            return Err(SealError::WrongKeyLen(enc.len()));
        }
        let mut k = [0u8; 32];
        k.copy_from_slice(&enc);
        return Ok(Zeroizing::new(k));
    }

    let k = unwrap_key(&enc, &wk, x25519_secret, &memory_hash, producer)?;
    Ok(k)
}

// ─── seal_chunk / open_chunk ─────────────────────────────────────────────────

/// Output of a [`seal_chunk`] call.
///
/// Each chunk is independently sealed with a fresh nonce under the same content
/// key `k`.  `hash` is `blake3(sealed_cbor)` and forms a link in the lineage
/// chain — callers should chain `prev_hash` ← `hash` ← `hash` … to get an
/// ordered, tamper-evident sequence.
pub struct SealedChunk {
    /// Canonical CBOR bytes of the chunk (nonce || ciphertext).
    ///
    /// Wire format (JSON-based CBOR map, schema-free):
    /// `{ "idx": u64, "nonce": bstr(24), "ct": bstr, "prev_hash": bstr(32) }`
    pub sealed_cbor: Vec<u8>,
    /// blake3 hash of `sealed_cbor` — links into the lineage chain.
    pub hash: [u8; 32],
}

/// Seal one ordered chunk of a streaming payload.
///
/// # Parameters
/// - `chunk` — raw plaintext bytes for this chunk.
/// - `k` — the 32-byte symmetric content-encryption key (shared across
///   all chunks in the stream, set by `seal_memory`).
/// - `idx` — 0-based sequence number; must be monotonically increasing.
/// - `prev_hash` — blake3 hash of the previous chunk's `sealed_cbor` (or the
///   `content_hash` of the opening `seal_memory` artifact for `idx == 0`).
/// - `rng` — cryptographically secure RNG for fresh nonce generation.
///
/// # Returns
/// A [`SealedChunk`] whose `hash` should become `prev_hash` for the next call.
pub fn seal_chunk(
    chunk: &[u8],
    k: &[u8; 32],
    idx: u64,
    prev_hash: &[u8; 32],
    rng: &mut impl RngCore,
) -> Result<SealedChunk, SealError> {
    // Fresh 24-byte nonce for this chunk.
    let mut nonce = [0u8; 24];
    rng.fill_bytes(&mut nonce);

    // AAD: prev_hash bound to the AEAD so swapping chunks is detectable.
    let ct = encrypt_content(k, &nonce, prev_hash.as_slice(), chunk)?;

    // Encode as a minimal JSON map (no schema overhead).
    let nonce_b64 = base64::engine::general_purpose::STANDARD.encode(nonce);
    let ct_b64 = base64::engine::general_purpose::STANDARD.encode(&ct);
    let prev_b64 = base64::engine::general_purpose::STANDARD.encode(prev_hash);

    let payload = serde_json::json!({
        "idx": idx,
        "nonce": nonce_b64,
        "ct": ct_b64,
        "prev_hash": prev_b64,
    });

    // Use ciborium directly for a compact, deterministic CBOR encoding.
    let mut sealed_cbor = Vec::new();
    ciborium::ser::into_writer(&payload, &mut sealed_cbor)
        .map_err(|e| SealError::Codec(format!("chunk cbor encode: {e}")))?;

    let hash = *blake3::hash(&sealed_cbor).as_bytes();
    Ok(SealedChunk { sealed_cbor, hash })
}

/// Open one sealed chunk and recover the plaintext.
///
/// Verifies that the `prev_hash` inside the CBOR matches the supplied
/// `expected_prev_hash` (preventing chunk reorder / splicing attacks), then
/// decrypts with `k`.
///
/// # Parameters
/// - `sealed_cbor`         — the `SealedChunk::sealed_cbor` bytes.
/// - `k`                   — the 32-byte content-encryption key.
/// - `expected_idx`        — the expected 0-based sequence number.
/// - `expected_prev_hash`  — must equal the `prev_hash` field inside the CBOR.
pub fn open_chunk(
    sealed_cbor: &[u8],
    k: &[u8; 32],
    expected_idx: u64,
    expected_prev_hash: &[u8; 32],
) -> Result<Vec<u8>, SealError> {
    // Decode CBOR.
    let value: serde_json::Value = ciborium::de::from_reader(sealed_cbor)
        .map_err(|e| SealError::Malformed(format!("chunk cbor decode: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| SealError::Malformed("chunk: not a map".into()))?;

    // Check sequence index.
    let idx = obj
        .get("idx")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| SealError::Malformed("chunk: missing 'idx'".into()))?;
    if idx != expected_idx {
        return Err(SealError::Malformed(format!(
            "chunk sequence mismatch: expected idx={expected_idx}, got {idx}"
        )));
    }

    // Decode fields.
    let nonce_bytes = get_bytes_field(obj, "nonce")?;
    let ct_bytes = get_bytes_field(obj, "ct")?;
    let prev_hash_bytes = get_bytes_field(obj, "prev_hash")?;

    // Verify prev_hash linkage.
    if prev_hash_bytes != expected_prev_hash.as_slice() {
        return Err(SealError::Malformed(
            "chunk prev_hash mismatch — chain broken or chunk spliced".into(),
        ));
    }

    let nonce: [u8; 24] = nonce_bytes
        .try_into()
        .map_err(|_| SealError::Malformed("chunk nonce is not 24 bytes".into()))?;
    let prev_arr: [u8; 32] = prev_hash_bytes
        .try_into()
        .map_err(|_| SealError::Malformed("chunk prev_hash not 32 bytes".into()))?;

    // Decrypt with prev_hash as AAD (must match what was used at seal time).
    let plaintext = decrypt_content(k, &nonce, prev_arr.as_slice(), &ct_bytes)?;
    Ok(plaintext)
}

// ─── link_fragment / parse_link_fragment ─────────────────────────────────────

/// Encode `K` as a URL fragment component: `k=<base64url-nopad>`.
pub fn link_fragment(k: &[u8; 32]) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(k);
    format!("k={encoded}")
}

/// Parse a URL fragment of the form `k=<base64url-nopad>` back to a 32-byte K.
pub fn parse_link_fragment(fragment: &str) -> Result<Zeroizing<[u8; 32]>, SealError> {
    let encoded = fragment
        .strip_prefix("k=")
        .ok_or_else(|| SealError::BadLinkFragment(fragment.to_string()))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|e| SealError::BadBase64(e.to_string()))?;
    if bytes.len() != 32 {
        return Err(SealError::WrongKeyLen(bytes.len()));
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&bytes);
    Ok(Zeroizing::new(k))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sealed::keys::x25519_secret_from_ed25519;
    use ed25519_dalek::SigningKey;

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[0x42u8; 32])
    }

    fn test_author_x25519(sk: &SigningKey) -> [u8; 32] {
        *x25519_secret_from_ed25519(sk)
    }

    fn seal_test_memory(inner: &[u8], sk: &SigningKey) -> SealedArtifact {
        let vk = sk.verifying_key();
        let vk_bytes = vk.to_bytes();
        let mut rng = rand_core::OsRng;
        seal_memory(
            inner,
            &vk_bytes,
            "art:test-sealed-1",
            "did:test:author",
            "2026-09-28T00:00:00Z",
            &mut rng,
        )
        .expect("seal_memory failed")
    }

    // ── seal + open round-trip ──────────────────────────────────────────────

    #[test]
    fn seal_open_round_trip() {
        let sk = test_signing_key();
        let inner = br#"{"type":"memory","content":"hello sealed world"}"#;
        let artifact = seal_test_memory(inner, &sk);

        let x25519_sk = test_author_x25519(&sk);
        let recovered = open_memory(&artifact.outer_cbor, &x25519_sk).expect("open_memory failed");
        assert_eq!(recovered, inner);
    }

    #[test]
    fn seal_open_empty_input() {
        let sk = test_signing_key();
        let inner = b"";
        let artifact = seal_test_memory(inner, &sk);
        let x25519_sk = test_author_x25519(&sk);
        let recovered = open_memory(&artifact.outer_cbor, &x25519_sk).unwrap();
        assert_eq!(recovered, inner);
    }

    #[test]
    fn seal_returns_k_that_matches_kc() {
        let sk = test_signing_key();
        let inner = b"test content";
        let artifact = seal_test_memory(inner, &sk);

        // Parse outer_cbor and check that kc == key_commitment(K).
        let payload = crate::codec::canonical::from_canonical_cbor(&artifact.outer_cbor).unwrap();
        let obj = payload.as_object().unwrap();
        let kc_b64 = obj["kc"].as_str().unwrap();
        let kc_bytes = base64::engine::general_purpose::STANDARD
            .decode(kc_b64)
            .unwrap();
        let expected_kc = key_commitment(&artifact.k);
        assert_eq!(kc_bytes, expected_kc);
    }

    // ── open_memory with wrong key → error ─────────────────────────────────

    #[test]
    fn open_memory_wrong_x25519_secret_fails() {
        let sk = test_signing_key();
        let inner = b"private memory";
        let artifact = seal_test_memory(inner, &sk);

        // Use a completely different X25519 secret.
        let wrong_sk = [0xFFu8; 32];
        let err = open_memory(&artifact.outer_cbor, &wrong_sk);
        assert!(err.is_err(), "wrong key must fail: {err:?}");
    }

    // ── open_with_key ───────────────────────────────────────────────────────

    #[test]
    fn open_with_key_works() {
        let sk = test_signing_key();
        let inner = b"bearer link content";
        let artifact = seal_test_memory(inner, &sk);

        // open_with_key uses K directly (from SealedArtifact).
        let recovered =
            open_with_key(&artifact.outer_cbor, &artifact.k).expect("open_with_key failed");
        assert_eq!(recovered, inner);
    }

    #[test]
    fn open_with_key_wrong_key_fails() {
        let sk = test_signing_key();
        let inner = b"some content";
        let artifact = seal_test_memory(inner, &sk);

        let wrong_k = [0xAAu8; 32];
        let err = open_with_key(&artifact.outer_cbor, &wrong_k);
        assert!(err.is_err());
    }

    // ── make_grant + open_grant round-trip (anonymous) ─────────────────────

    #[test]
    fn make_open_grant_anonymous_round_trip() {
        let sk = test_signing_key();
        let inner = b"grant test content";
        let artifact = seal_test_memory(inner, &sk);

        let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();

        // Anonymous grant.
        let grant_cbor = make_grant(
            &memory_hash,
            &artifact.outer_cbor,
            &artifact.k,
            None,
            "did:test:author",
            None,
            "2026-09-28T00:00:00Z",
        )
        .expect("make_grant failed");

        // open_grant with any X25519 secret (anonymous grant ignores it).
        let k_recovered = open_grant(&grant_cbor, &[0u8; 32]).expect("open_grant failed");
        assert_eq!(*k_recovered, *artifact.k);
    }

    #[test]
    fn anonymous_grant_has_no_reader_field() {
        let sk = test_signing_key();
        let inner = b"anon grant";
        let artifact = seal_test_memory(inner, &sk);
        let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();

        let grant_cbor = make_grant(
            &memory_hash,
            &artifact.outer_cbor,
            &artifact.k,
            None,
            "did:test:author",
            None,
            "2026-09-28T00:00:00Z",
        )
        .unwrap();

        let payload = crate::codec::canonical::from_canonical_cbor(&grant_cbor).unwrap();
        let obj = payload.as_object().unwrap();
        assert!(
            obj.get("reader").is_none(),
            "anonymous grant must have no 'reader' field"
        );
    }

    // ── make_grant + open_grant round-trip (targeted) ──────────────────────

    #[test]
    fn make_open_grant_targeted_round_trip() {
        use hpke::kem::{Kem as KemTrait, X25519HkdfSha256};
        use hpke::Serializable;

        let sk = test_signing_key();
        let inner = b"targeted grant content";
        let artifact = seal_test_memory(inner, &sk);
        let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();

        // Generate a reader X25519 keypair.
        let (reader_sk_hpke, reader_pk_hpke) = X25519HkdfSha256::gen_keypair(&mut rand_core::OsRng);
        let reader_sk_bytes: [u8; 32] = reader_sk_hpke.to_bytes().into();
        let reader_pk_bytes: [u8; 32] = reader_pk_hpke.to_bytes().into();

        let grant_cbor = make_grant(
            &memory_hash,
            &artifact.outer_cbor,
            &artifact.k,
            Some(&reader_pk_bytes),
            "did:test:author",
            None,
            "2026-09-28T00:00:00Z",
        )
        .expect("make_grant targeted failed");

        let k_recovered =
            open_grant(&grant_cbor, &reader_sk_bytes).expect("open_grant targeted failed");
        assert_eq!(*k_recovered, *artifact.k);
    }

    // ── link_fragment + parse_link_fragment round-trip ─────────────────────

    #[test]
    fn link_fragment_round_trip() {
        let k = [0x55u8; 32];
        let fragment = link_fragment(&k);
        assert!(fragment.starts_with("k="), "fragment must start with k=");
        let k2 = parse_link_fragment(&fragment).expect("parse_link_fragment failed");
        assert_eq!(k, *k2);
    }

    #[test]
    fn parse_link_fragment_bad_prefix() {
        let err = parse_link_fragment("nope=abc");
        assert!(matches!(err, Err(SealError::BadLinkFragment(_))));
    }

    #[test]
    fn parse_link_fragment_bad_base64() {
        let err = parse_link_fragment("k=!!!");
        assert!(matches!(err, Err(SealError::BadBase64(_))));
    }

    #[test]
    fn parse_link_fragment_wrong_length() {
        // 16 bytes encoded → wrong length
        let short = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 16]);
        let fragment = format!("k={short}");
        let err = parse_link_fragment(&fragment);
        assert!(matches!(err, Err(SealError::WrongKeyLen(_))));
    }
}

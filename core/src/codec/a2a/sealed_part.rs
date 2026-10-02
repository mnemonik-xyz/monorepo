//! Sealed-memory A2A DataPart — tech-spec §9.1 (Task 14).
//!
//! ## Media type
//!
//! `application/vnd.mnemonic.sealed+cbor` — carries a sealed memory inside an
//! A2A DataPart. The `data` field of the DataPart is a JSON object with two
//! fields:
//!
//! ```json
//! {
//!   "sealed": "<base64-encoded SEALED_V1 CBOR bytes>",
//!   "grants": ["<base64-encoded GRANT_V1 CBOR>", ...]
//! }
//! ```
//!
//! ## Recipient key resolution
//!
//! The sender must call [`recipient_x25519_from_agent_card`] to extract the
//! recipient's X25519 public key from an AgentCard that carries an
//! `x-mnemonic.ed25519_pubkey_base58` extension (Decision 3 / Task 4).
//!
//! ## Seal/open helpers
//!
//! [`seal_a2a_data_part`] seals a memory for a recipient and returns a ready
//! `Part::Data` carrying the sealed payload.  [`open_a2a_data_part`] extracts
//! and opens the sealed CBOR from such a part.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::extension::XMnemonicExtension;
use super::part::Part;

/// Media type for a sealed-memory A2A DataPart.
pub const SEALED_CBOR_MEDIA_TYPE: &str = "application/vnd.mnemonic.sealed+cbor";

/// Wire payload carried in a sealed DataPart's `data` field.
///
/// `sealed` is the canonical CBOR of a `SEALED_V1` artifact (base64-encoded).
/// `grants` is a list of `GRANT_V1` CBOR blobs (base64-encoded) that wrap the
/// content key for one or more authorised readers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedPartPayload {
    /// Base64-encoded SEALED_V1 CBOR bytes.
    pub sealed: String,
    /// Base64-encoded GRANT_V1 CBOR blobs, one per authorised reader.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<String>,
}

/// Error variants for sealed A2A part operations.
#[derive(Debug, thiserror::Error)]
pub enum SealedPartError {
    /// The Part variant is not `Data`.
    #[error("expected a Data part, got a different kind")]
    NotADataPart,
    /// The DataPart's `mimeType` is not [`SEALED_CBOR_MEDIA_TYPE`].
    #[error("expected mimeType {SEALED_CBOR_MEDIA_TYPE}, got {0:?}")]
    WrongMediaType(Option<String>),
    /// The DataPart's `data` field cannot be deserialized as [`SealedPartPayload`].
    #[error("sealed part payload deserialize error: {0}")]
    DeserializePayload(String),
    /// Base64 decode error on the `sealed` field.
    #[error("sealed field base64 error: {0}")]
    BadBase64Sealed(String),
    /// Base64 decode error on a `grants` entry.
    #[error("grants[{0}] base64 error: {1}")]
    BadBase64Grant(usize, String),
    /// An error from the core sealed-memory layer.
    #[error("seal error: {0}")]
    Seal(#[from] crate::sealed::SealError),
    /// AgentCard does not carry an `x-mnemonic` extension.
    #[error("AgentCard has no x-mnemonic extension")]
    NoExtension,
    /// AgentCard `ed25519_pubkey_base58` is not valid base58 / wrong length.
    #[error("ed25519_pubkey_base58 is not valid: {0}")]
    BadEd25519Pubkey(String),
    /// The Ed25519 → X25519 key conversion failed.
    #[error("key conversion error: {0}")]
    KeyConvert(String),
}

// ─── Recipient key from AgentCard ─────────────────────────────────────────────

/// Extract the recipient's X25519 public key from an A2A AgentCard JSON value.
///
/// Looks up the `x-mnemonic` extension, reads `ed25519_pubkey_base58`, decodes
/// the base58-encoded Ed25519 public key, and converts it to the corresponding
/// X25519 public key via the standard birational map.
///
/// # Errors
/// - [`SealedPartError::NoExtension`] — no `x-mnemonic` extension on the card.
/// - [`SealedPartError::BadEd25519Pubkey`] — base58 decode fails or length ≠ 32.
/// - [`SealedPartError::KeyConvert`] — the Ed25519 → X25519 conversion fails
///   (e.g. small-order point).
pub fn recipient_x25519_from_agent_card(agent_card: &Value) -> Result<[u8; 32], SealedPartError> {
    // Extract the x-mnemonic extension.
    let ext: XMnemonicExtension = {
        let extensions = agent_card.get("extensions");
        let arr = extensions
            .and_then(|v| v.as_array())
            .ok_or(SealedPartError::NoExtension)?;

        let maybe = arr.iter().find_map(|e| {
            if e.get("uri").and_then(|u| u.as_str())
                == Some(crate::codec::a2a::extension::EXTENSION_URI)
            {
                serde_json::from_value::<XMnemonicExtension>(e.clone()).ok()
            } else {
                None
            }
        });
        maybe.ok_or(SealedPartError::NoExtension)?
    };

    // Decode base58 → 32-byte Ed25519 pubkey.
    let raw = bs58::decode(&ext.ed25519_pubkey_base58)
        .into_vec()
        .map_err(|e| SealedPartError::BadEd25519Pubkey(e.to_string()))?;
    if raw.len() != 32 {
        return Err(SealedPartError::BadEd25519Pubkey(format!(
            "expected 32 bytes, got {}",
            raw.len()
        )));
    }
    let mut ed_bytes = [0u8; 32];
    ed_bytes.copy_from_slice(&raw);

    // Convert Ed25519 → X25519.
    crate::sealed::keys::x25519_public_from_ed25519(&ed_bytes)
        .map_err(|e| SealedPartError::KeyConvert(e.to_string()))
}

// ─── Build / parse sealed DataPart ────────────────────────────────────────────

/// Build a `Part::Data` carrying sealed-memory payload.
///
/// `sealed_cbor` is the raw `SEALED_V1` CBOR bytes (the `outer_cbor` field of
/// a [`crate::sealed::SealedArtifact`]). `grants` is an optional list of
/// `GRANT_V1` CBOR blobs to attach.
///
/// # Returns
/// A `Part::Data` whose `mimeType` is [`SEALED_CBOR_MEDIA_TYPE`].
pub fn build_sealed_data_part(sealed_cbor: &[u8], grants: &[Vec<u8>]) -> Part {
    let sealed_b64 = base64::engine::general_purpose::STANDARD.encode(sealed_cbor);
    let grants_b64: Vec<String> = grants
        .iter()
        .map(|g| base64::engine::general_purpose::STANDARD.encode(g))
        .collect();

    let payload = SealedPartPayload {
        sealed: sealed_b64,
        grants: grants_b64,
    };

    Part::Data {
        data: serde_json::to_value(&payload).expect("SealedPartPayload always serializes"),
        mime_type: Some(SEALED_CBOR_MEDIA_TYPE.to_string()),
    }
}

/// Extract `(sealed_cbor, grants)` from a `Part::Data` carrying
/// [`SEALED_CBOR_MEDIA_TYPE`].
///
/// # Errors
/// - [`SealedPartError::NotADataPart`] — the part is not `Data`.
/// - [`SealedPartError::WrongMediaType`] — the `mimeType` is wrong.
/// - [`SealedPartError::DeserializePayload`] — the `data` field cannot be parsed.
/// - [`SealedPartError::BadBase64Sealed`] — the `sealed` string is not valid base64.
/// - [`SealedPartError::BadBase64Grant`] — a `grants` entry is not valid base64.
pub fn extract_sealed_data_part(part: &Part) -> Result<(Vec<u8>, Vec<Vec<u8>>), SealedPartError> {
    let (data, mime_type) = match part {
        Part::Data { data, mime_type } => (data, mime_type),
        _ => return Err(SealedPartError::NotADataPart),
    };

    match mime_type.as_deref() {
        Some(SEALED_CBOR_MEDIA_TYPE) => {}
        other => return Err(SealedPartError::WrongMediaType(other.map(str::to_string))),
    }

    let payload: SealedPartPayload = serde_json::from_value(data.clone())
        .map_err(|e| SealedPartError::DeserializePayload(e.to_string()))?;

    let sealed_cbor = base64::engine::general_purpose::STANDARD
        .decode(&payload.sealed)
        .map_err(|e| SealedPartError::BadBase64Sealed(e.to_string()))?;

    let grants: Vec<Vec<u8>> = payload
        .grants
        .iter()
        .enumerate()
        .map(|(i, g)| {
            base64::engine::general_purpose::STANDARD
                .decode(g)
                .map_err(|e| SealedPartError::BadBase64Grant(i, e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok((sealed_cbor, grants))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::a2a::extension::build_x_mnemonic_extension;
    use crate::sealed::keys::{x25519_public_from_ed25519, x25519_secret_from_ed25519};
    use crate::sealed::{seal_memory, SealError};
    use ed25519_dalek::SigningKey;
    use serde_json::json;

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[0x42u8; 32])
    }

    fn agent_card_for(sk: &SigningKey) -> Value {
        let vk = sk.verifying_key();
        let pk_b58 = bs58::encode(vk.to_bytes()).into_string();
        json!({
            "name": "TestAgent",
            "url": "https://agent.example.com",
            "extensions": [
                build_x_mnemonic_extension(&pk_b58, None)
            ]
        })
    }

    // ── recipient key extraction ─────────────────────────────────────────────

    #[test]
    fn recipient_x25519_extracted_from_card() {
        let sk = test_signing_key();
        let card = agent_card_for(&sk);
        let x25519 =
            recipient_x25519_from_agent_card(&card).expect("must extract X25519 from valid card");
        // Should match the direct derivation.
        let expected = x25519_public_from_ed25519(&sk.verifying_key().to_bytes()).unwrap();
        assert_eq!(x25519, expected);
    }

    #[test]
    fn no_extension_returns_error() {
        let card = json!({ "name": "NoExtAgent", "url": "https://x.com" });
        assert!(matches!(
            recipient_x25519_from_agent_card(&card),
            Err(SealedPartError::NoExtension)
        ));
    }

    #[test]
    fn bad_base58_returns_error() {
        let card = json!({
            "name": "BadKeyAgent",
            "url": "https://x.com",
            "extensions": [{
                "uri": crate::codec::a2a::extension::EXTENSION_URI,
                "ed25519_pubkey_base58": "!!!notbase58!!!",
                "conformance_version": "1"
            }]
        });
        assert!(matches!(
            recipient_x25519_from_agent_card(&card),
            Err(SealedPartError::BadEd25519Pubkey(_))
        ));
    }

    // ── build + extract sealed DataPart ──────────────────────────────────────

    #[test]
    fn build_extract_sealed_part_round_trip() {
        let fake_cbor = b"fake-sealed-cbor-bytes-here";
        let fake_grant = b"fake-grant-cbor-bytes";
        let part = build_sealed_data_part(fake_cbor, &[fake_grant.to_vec()]);

        let (got_cbor, got_grants) = extract_sealed_data_part(&part).expect("extract must succeed");
        assert_eq!(got_cbor, fake_cbor);
        assert_eq!(got_grants.len(), 1);
        assert_eq!(got_grants[0], fake_grant);
    }

    #[test]
    fn extract_wrong_media_type_fails() {
        let part = Part::Data {
            data: json!({ "sealed": "YQ==", "grants": [] }),
            mime_type: Some("application/json".to_string()),
        };
        assert!(matches!(
            extract_sealed_data_part(&part),
            Err(SealedPartError::WrongMediaType(_))
        ));
    }

    #[test]
    fn extract_non_data_part_fails() {
        let part = Part::Text {
            text: "hello".to_string(),
        };
        assert!(matches!(
            extract_sealed_data_part(&part),
            Err(SealedPartError::NotADataPart)
        ));
    }

    // ── full seal/open via A2A DataPart ──────────────────────────────────────

    /// T14 conformance vector: seal + open by recipient.
    #[test]
    fn seal_open_by_recipient_via_data_part() {
        let author_sk = test_signing_key();
        let recipient_sk = SigningKey::from_bytes(&[0x77u8; 32]);

        // Derive recipient X25519 keypair.
        let recipient_x25519_pub =
            x25519_public_from_ed25519(&recipient_sk.verifying_key().to_bytes()).unwrap();
        let recipient_x25519_sec = x25519_secret_from_ed25519(&recipient_sk);

        // Seal a memory for the author; then build a grant for the recipient.
        let inner = b"sealed memory for recipient via A2A";
        let author_vk = author_sk.verifying_key();
        let author_ed_bytes = author_vk.to_bytes();
        let mut rng = rand_core::OsRng;
        let artifact = seal_memory(
            inner,
            &author_ed_bytes,
            "art:t14-test",
            "did:test:author",
            "2026-09-28T00:00:00Z",
            &mut rng,
        )
        .expect("seal_memory failed");

        // Build a grant for the recipient.
        let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();
        let grant_cbor = crate::sealed::make_grant(
            &memory_hash,
            &artifact.outer_cbor,
            &artifact.k,
            Some(&recipient_x25519_pub),
            "did:test:author",
            None,
            "2026-09-28T00:00:00Z",
        )
        .expect("make_grant failed");

        // Wrap in an A2A DataPart.
        let part = build_sealed_data_part(&artifact.outer_cbor, &[grant_cbor]);

        // Extract and open.
        let (sealed_cbor, grants) = extract_sealed_data_part(&part).unwrap();
        // Open via the grant (recipient uses open_grant + open_with_key).
        let k_from_grant = crate::sealed::open_grant(&grants[0], &recipient_x25519_sec)
            .expect("open_grant failed");
        let recovered = crate::sealed::open_with_key(&sealed_cbor, &k_from_grant)
            .expect("open_with_key failed");
        assert_eq!(recovered, inner);
    }

    /// T14 conformance vector: non-recipient cannot open.
    #[test]
    fn non_recipient_cannot_open() {
        let author_sk = test_signing_key();
        let recipient_sk = SigningKey::from_bytes(&[0x77u8; 32]);
        let outsider_sk = SigningKey::from_bytes(&[0xBBu8; 32]);

        let recipient_x25519_pub =
            x25519_public_from_ed25519(&recipient_sk.verifying_key().to_bytes()).unwrap();
        let outsider_x25519_sec = x25519_secret_from_ed25519(&outsider_sk);

        let inner = b"secret";
        let author_vk = author_sk.verifying_key();
        let author_ed_bytes = author_vk.to_bytes();
        let mut rng = rand_core::OsRng;
        let artifact = seal_memory(
            inner,
            &author_ed_bytes,
            "art:t14-nonrecipient",
            "did:test:author",
            "2026-09-28T00:00:00Z",
            &mut rng,
        )
        .unwrap();

        let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();
        let grant_cbor = crate::sealed::make_grant(
            &memory_hash,
            &artifact.outer_cbor,
            &artifact.k,
            Some(&recipient_x25519_pub),
            "did:test:author",
            None,
            "2026-09-28T00:00:00Z",
        )
        .unwrap();

        // Outsider tries to open the grant — must fail.
        let result = crate::sealed::open_grant(&grant_cbor, &outsider_x25519_sec);
        assert!(result.is_err(), "non-recipient must not open the grant");
    }

    /// T14 conformance vector: tampered sealed ciphertext fails.
    #[test]
    fn tampered_sealed_cbor_fails() {
        let sk = test_signing_key();
        let inner = b"tamper test";
        let vk = sk.verifying_key().to_bytes();
        let x25519_sk = x25519_secret_from_ed25519(&sk);
        let mut rng = rand_core::OsRng;
        let artifact = seal_memory(
            inner,
            &vk,
            "art:t14-tamper",
            "did:test:author",
            "2026-09-28T00:00:00Z",
            &mut rng,
        )
        .unwrap();

        let mut tampered = artifact.outer_cbor.clone();
        // Flip bits somewhere in the middle of the ciphertext.
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0xFF;

        let result = crate::sealed::open_memory(&tampered, &x25519_sk);
        assert!(result.is_err(), "tampered ciphertext must fail to open");
    }

    /// T14 conformance vector: seal_chunk / open_chunk chain.
    #[test]
    fn seal_open_chunk_chain() {
        use crate::sealed::{open_chunk, seal_chunk};

        let k = [0x42u8; 32];
        let root_hash = [0x00u8; 32]; // genesis prev_hash
        let mut rng = rand_core::OsRng;

        let chunks: &[&[u8]] = &[b"chunk 0 data", b"chunk 1 data", b"chunk 2 data"];
        let mut sealed_chunks = Vec::new();
        let mut prev = root_hash;

        for (idx, &chunk) in chunks.iter().enumerate() {
            let sc = seal_chunk(chunk, &k, idx as u64, &prev, &mut rng).expect("seal_chunk failed");
            prev = sc.hash;
            sealed_chunks.push(sc);
        }

        // Open all chunks in order.
        let mut prev = root_hash;
        for (idx, sc) in sealed_chunks.iter().enumerate() {
            let pt = open_chunk(&sc.sealed_cbor, &k, idx as u64, &prev).expect("open_chunk failed");
            assert_eq!(pt, chunks[idx]);
            prev = sc.hash;
        }
    }

    /// T14 conformance vector: wrong chunk order is rejected.
    #[test]
    fn open_chunk_wrong_order_fails() {
        use crate::sealed::seal_chunk;

        let k = [0x42u8; 32];
        let root_hash = [0x00u8; 32];
        let mut rng = rand_core::OsRng;

        let c0 = seal_chunk(b"chunk 0", &k, 0, &root_hash, &mut rng).unwrap();
        let c1 = seal_chunk(b"chunk 1", &k, 1, &c0.hash, &mut rng).unwrap();

        // Opening c1 before c0's hash is known should fail (wrong prev_hash).
        let result = crate::sealed::open_chunk(&c1.sealed_cbor, &k, 1, &root_hash);
        assert!(
            result.is_err(),
            "opening chunk 1 with wrong prev_hash must fail"
        );
    }

    /// T14 conformance vector: tampered chunk ciphertext fails.
    #[test]
    fn tampered_chunk_ciphertext_fails() {
        use crate::sealed::seal_chunk;

        let k = [0x42u8; 32];
        let root_hash = [0x00u8; 32];
        let mut rng = rand_core::OsRng;

        let mut sc = seal_chunk(b"chunk data", &k, 0, &root_hash, &mut rng).unwrap();
        // Corrupt the middle of the sealed CBOR (likely hits the ciphertext).
        let mid = sc.sealed_cbor.len() / 2;
        sc.sealed_cbor[mid] ^= 0xFF;

        // Decoding may fail as malformed CBOR or as a content error.
        let result = crate::sealed::open_chunk(&sc.sealed_cbor, &k, 0, &root_hash);
        assert!(result.is_err(), "tampered chunk must fail to open");
    }
}

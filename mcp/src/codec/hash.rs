//! Content hashing — blake3 over canonical CBOR bytes.
//!
//! The content hash is the single value anchored on-chain (Solana SPL Memo).
//! It is computed AFTER CBOR canonicalization, BEFORE COSE wrapping.
//!
//! ```text
//! artifact JSON → canonical CBOR → blake3 hash → anchor on Solana
//!                                              → COSE sign (week 2)
//! ```

use super::canonical::to_canonical_cbor;
use super::schema::ArtifactSchema;

/// Compute blake3 hash of a JSON artifact's canonical CBOR representation.
///
/// Returns lowercase hex string (64 chars).
pub fn hash_artifact(artifact: &serde_json::Value, schema: &ArtifactSchema) -> Result<String, String> {
    let cbor_bytes = to_canonical_cbor(artifact, schema)?;
    Ok(hash_bytes(&cbor_bytes))
}

/// Compute blake3 hash of raw bytes. Returns lowercase hex string.
pub fn hash_bytes(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Verify that raw bytes match an expected blake3 hash.
pub fn verify_hash(data: &[u8], expected_hex: &str) -> bool {
    hash_bytes(data) == expected_hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::schema::*;

    fn sample_artifact() -> serde_json::Value {
        serde_json::json!({
            "artifact_id": "art:hash-test",
            "type": "memory",
            "schema_version": 1,
            "content": "hash test content",
            "producer": "did:sol:abc",
            "created_at": "2026-04-14T00:00:00Z",
        })
    }

    #[test]
    fn test_hash_deterministic() {
        let artifact = sample_artifact();
        let h1 = hash_artifact(&artifact, &MEMORY_V1).unwrap();
        let h2 = hash_artifact(&artifact, &MEMORY_V1).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_is_blake3() {
        let artifact = sample_artifact();
        let cbor_bytes = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        let direct_hash = blake3::hash(&cbor_bytes).to_hex().to_string();
        let codec_hash = hash_artifact(&artifact, &MEMORY_V1).unwrap();
        assert_eq!(direct_hash, codec_hash);
    }

    #[test]
    fn test_hash_length() {
        let h = hash_artifact(&sample_artifact(), &MEMORY_V1).unwrap();
        assert_eq!(h.len(), 64, "blake3 hex hash should be 64 chars");
    }

    #[test]
    fn test_hash_changes_with_content() {
        let a = serde_json::json!({
            "artifact_id": "art:A", "type": "memory", "schema_version": 1,
            "content": "content A", "producer": "p", "created_at": "2026-01-01T00:00:00Z",
        });
        let b = serde_json::json!({
            "artifact_id": "art:A", "type": "memory", "schema_version": 1,
            "content": "content B", "producer": "p", "created_at": "2026-01-01T00:00:00Z",
        });
        assert_ne!(
            hash_artifact(&a, &MEMORY_V1).unwrap(),
            hash_artifact(&b, &MEMORY_V1).unwrap(),
        );
    }

    #[test]
    fn test_verify_hash() {
        let data = b"hello blake3";
        let h = hash_bytes(data);
        assert!(verify_hash(data, &h));
        assert!(!verify_hash(b"wrong", &h));
    }

    #[test]
    fn test_hash_consistency_with_canonical_cbor() {
        // Hash must equal blake3(to_canonical_cbor(artifact))
        let artifact = sample_artifact();
        let cbor = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        let expected = hash_bytes(&cbor);
        let actual = hash_artifact(&artifact, &MEMORY_V1).unwrap();
        assert_eq!(expected, actual);
    }
}

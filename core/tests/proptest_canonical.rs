//! Property-based tests for CBOR canonicalization determinism.
//!
//! Uses proptest to generate random artifact payloads and verify that
//! `mnemonic_core::codec::canonical::to_canonical_cbor` always produces
//! identical bytes for the same input.

use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::codec::hash::hash_bytes;
use mnemonic_core::codec::schema::MEMORY_V1;
use proptest::prelude::*;

proptest! {
    #[test]
    fn canonical_cbor_is_deterministic(
        artifact_id in "[a-z0-9:]{5,20}",
        content in ".{1,200}",
        producer in "[a-zA-Z0-9:._]{5,30}",
        tag1 in "[a-z]{1,10}",
        tag2 in "[a-z]{1,10}",
    ) {
        let artifact = serde_json::json!({
            "artifact_id": artifact_id,
            "type": "memory",
            "schema_version": 1,
            "content": content,
            "producer": producer,
            "created_at": "2026-04-14T00:00:00Z",
            "tags": [tag1, tag2],
        });

        let bytes1 = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        let bytes2 = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        prop_assert_eq!(bytes1, bytes2, "canonical CBOR must be deterministic");
    }

    #[test]
    fn hash_is_deterministic(
        content in ".{1,500}",
    ) {
        let artifact = serde_json::json!({
            "artifact_id": "art:proptest",
            "type": "memory",
            "schema_version": 1,
            "content": content,
            "producer": "test",
            "created_at": "2026-01-01T00:00:00Z",
        });

        let bytes1 = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        let bytes2 = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        let h1 = hash_bytes(&bytes1);
        let h2 = hash_bytes(&bytes2);
        prop_assert_eq!(h1, h2, "blake3(canonical_cbor) must be deterministic");
    }

    #[test]
    fn different_content_different_hash(
        content_a in "[a-z]{10,50}",
        content_b in "[A-Z]{10,50}",
    ) {
        let a = serde_json::json!({
            "artifact_id": "art:a", "type": "memory", "schema_version": 1,
            "content": content_a, "producer": "p", "created_at": "2026-01-01T00:00:00Z",
        });
        let b = serde_json::json!({
            "artifact_id": "art:a", "type": "memory", "schema_version": 1,
            "content": content_b, "producer": "p", "created_at": "2026-01-01T00:00:00Z",
        });

        let bytes_a = to_canonical_cbor(&a, &MEMORY_V1).unwrap();
        let bytes_b = to_canonical_cbor(&b, &MEMORY_V1).unwrap();

        // content_a matches [a-z], content_b matches [A-Z] -- they're always different
        prop_assert_ne!(hash_bytes(&bytes_a), hash_bytes(&bytes_b));
    }
}

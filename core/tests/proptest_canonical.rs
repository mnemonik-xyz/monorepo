//! Property-based tests for CBOR canonicalization determinism.
//!
//! Uses proptest to generate random artifact payloads and verify that
//! `mnemonic_core::codec::canonical::to_canonical_cbor` always produces
//! identical bytes for the same input.

use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::codec::hash::hash_bytes;
use mnemonic_core::codec::schema::MEMORY_V1;
use mnemonic_core::codec::sign::sign_artifact;
use proptest::prelude::*;
use solana_sdk::signature::Keypair;

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
    fn sign_artifact_content_hash_matches_blake3_of_canonical_cbor(
        content in ".{1,500}",
    ) {
        // Cross-layer check: the content_hash recorded by sign_artifact must
        // equal blake3 of the canonical CBOR bytes that sign_artifact itself
        // produced. Stronger than "hash is deterministic" -- it wires the
        // canonical codec, the hash layer, and the sign pipeline together.
        let artifact = serde_json::json!({
            "artifact_id": "art:proptest",
            "type": "memory",
            "schema_version": 1,
            "content": content,
            "producer": "test",
            "created_at": "2026-01-01T00:00:00Z",
        });

        let kp = Keypair::new();
        let signed = sign_artifact(&artifact, &MEMORY_V1, &kp).unwrap();

        // 1. content_hash equals blake3 of the canonical CBOR sign_artifact stored.
        prop_assert_eq!(
            &signed.content_hash,
            &hash_bytes(&signed.canonical_cbor),
            "content_hash must be blake3(canonical_cbor)"
        );

        // 2. Recomputing canonical CBOR independently yields the same bytes/hash.
        let cbor_again = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        prop_assert_eq!(
            &signed.canonical_cbor,
            &cbor_again,
            "sign_artifact's canonical_cbor must match a fresh canonicalization"
        );
        prop_assert_eq!(
            &signed.content_hash,
            &hash_bytes(&cbor_again),
            "content_hash must match blake3 of independently canonicalized bytes"
        );
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

        // Ranges [a-z] and [A-Z] are disjoint; content_a and content_b can never be equal.
        prop_assert_ne!(hash_bytes(&bytes_a), hash_bytes(&bytes_b));
    }
}

//! Week 4 -- Integration tests for the full CBOR/COSE pipeline.
//!
//! Tests the complete artifact lifecycle:
//! sign -> serialize -> deserialize -> verify -> hash consistency
//!
//! All helpers come from `mnemonic_core::codec` directly -- no inline duplicates.

use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::codec::hash::hash_bytes;
use mnemonic_core::codec::schema::{
    get_schema, AGENT_STATE_V1, MEMORY_V1, RAG_CONTEXT_V1, RAG_RESULT_V1, RECEIPT_V1,
};
use mnemonic_core::codec::sign::{sign_artifact, verify_artifact};
use solana_sdk::signature::{Keypair, Signer};

#[test]
fn test_full_sign_verify_roundtrip() {
    let kp = Keypair::new();
    let artifact = serde_json::json!({
        "artifact_id": "art:integration-1",
        "type": "memory",
        "schema_version": 1,
        "content": "Integration test: full CBOR/COSE round-trip",
        "producer": format!("did:sol:{}", kp.pubkey()),
        "created_at": "2026-04-14T12:00:00Z",
        "tags": ["integration", "week4"],
    });

    // 1. Canonical CBOR
    let cbor = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
    assert!(!cbor.is_empty());

    // 2. blake3 hash
    let hash = hash_bytes(&cbor);
    assert_eq!(hash.len(), 64);

    // 3. Sign (COSE_Sign1)
    let signed = sign_artifact(&artifact, &MEMORY_V1, &kp).unwrap();
    assert!(signed.cose_bytes.len() > cbor.len()); // COSE adds overhead
    assert_eq!(signed.content_hash, hash);
    assert_eq!(signed.canonical_cbor, cbor);

    // 4. Verify
    let result = verify_artifact(&signed.cose_bytes, Some(&signed.content_hash)).unwrap();
    assert!(result.valid, "COSE signature must verify");
    assert!(result.cose_signature);
    assert!(result.content_integrity);
    assert!(result.algorithm_valid);
    assert_eq!(result.signer, kp.pubkey().to_string());
    assert_eq!(result.payload, cbor, "payload must be the canonical CBOR");

    // 5. Hash consistency
    assert_eq!(
        hash_bytes(&result.payload),
        hash,
        "hash must match after round-trip"
    );
}

#[test]
fn test_determinism_across_multiple_keypairs() {
    let artifact = serde_json::json!({
        "artifact_id": "art:determinism",
        "type": "memory",
        "schema_version": 1,
        "content": "Same content, different signers",
        "producer": "did:sol:placeholder",
        "created_at": "2026-04-14T00:00:00Z",
    });

    let cbor1 = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
    let cbor2 = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
    assert_eq!(cbor1, cbor2, "canonical CBOR must be identical");

    let hash1 = hash_bytes(&cbor1);
    let hash2 = hash_bytes(&cbor2);
    assert_eq!(hash1, hash2, "blake3 hash must be identical");

    // Sign with two different keypairs -- hashes same, signatures different
    let kp1 = Keypair::new();
    let kp2 = Keypair::new();
    let s1 = sign_artifact(&artifact, &MEMORY_V1, &kp1).unwrap();
    let s2 = sign_artifact(&artifact, &MEMORY_V1, &kp2).unwrap();
    assert_ne!(s1.cose_bytes, s2.cose_bytes, "different signers produce different COSE");
    assert_eq!(s1.content_hash, s2.content_hash, "content hashes must match");

    let r1 = verify_artifact(&s1.cose_bytes, Some(&s1.content_hash)).unwrap();
    let r2 = verify_artifact(&s2.cose_bytes, Some(&s2.content_hash)).unwrap();
    assert!(r1.valid && r2.valid);
    assert_ne!(r1.signer, r2.signer);
    assert_eq!(r1.payload, r2.payload, "payloads must be identical");
}

#[test]
fn test_all_artifact_schemas() {
    let kp = Keypair::new();
    let schemas: &[(&str, &mnemonic_core::codec::schema::ArtifactSchema)] = &[
        ("rag.context", &RAG_CONTEXT_V1),
        ("rag.result", &RAG_RESULT_V1),
        ("agent.state", &AGENT_STATE_V1),
        ("receipt", &RECEIPT_V1),
        ("memory", &MEMORY_V1),
    ];

    for (type_name, schema) in schemas {
        // Confirm get_schema() lookup matches our static reference
        assert!(
            get_schema(type_name, 1).is_some(),
            "{type_name}: schema registry lookup failed"
        );

        let artifact = serde_json::json!({
            "artifact_id": format!("art:{type_name}-w4"),
            "type": type_name,
            "schema_version": 1,
            "content": format!("week 4 test for {type_name}"),
            "producer": format!("did:sol:{}", kp.pubkey()),
            "created_at": "2026-04-14T00:00:00Z",
        });

        let cbor = to_canonical_cbor(&artifact, schema).unwrap();
        let hash = hash_bytes(&cbor);

        let signed = sign_artifact(&artifact, schema, &kp).expect(type_name);
        let result = verify_artifact(&signed.cose_bytes, Some(&signed.content_hash)).expect(type_name);

        assert!(result.valid, "{type_name}: COSE verification failed");
        assert_eq!(result.payload, cbor, "{type_name}: payload mismatch");
        assert_eq!(hash_bytes(&result.payload), hash, "{type_name}: hash mismatch");
    }
}

#[test]
fn test_cbor_is_smaller_than_json() {
    let artifact = serde_json::json!({
        "artifact_id": "art:size-test",
        "type": "memory",
        "schema_version": 1,
        "content": "A moderately long piece of content that represents a typical memory attestation with enough text to show compression benefits of CBOR over JSON encoding.",
        "producer": "did:sol:7xKXtg2CabcdefghijklmnopqrstuvwxyzABCDEFGH",
        "created_at": "2026-04-14T12:34:56Z",
        "tags": ["benchmark", "size", "comparison"],
        "metadata": {"source": "integration_test", "version": 1},
    });

    let json_bytes = serde_json::to_vec(&artifact).unwrap();
    let cbor_bytes = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();

    println!(
        "JSON: {} bytes, CBOR: {} bytes, ratio: {:.2}x",
        json_bytes.len(),
        cbor_bytes.len(),
        json_bytes.len() as f64 / cbor_bytes.len() as f64
    );

    // CBOR must be strictly smaller than JSON for a payload of this shape.
    // Savings come from removing quote/colon/comma delimiters around every value
    // and encoding the ISO-8601 `created_at` as a CBOR tag-1 epoch integer.
    assert!(
        cbor_bytes.len() < json_bytes.len(),
        "CBOR ({}) should be strictly smaller than JSON ({})",
        cbor_bytes.len(),
        json_bytes.len()
    );
}

#[test]
fn test_tampered_cose_detected() {
    let kp = Keypair::new();
    let artifact = serde_json::json!({
        "artifact_id": "art:tamper-test",
        "type": "memory",
        "schema_version": 1,
        "content": "original content",
        "producer": "did:sol:test",
        "created_at": "2026-04-14T00:00:00Z",
    });

    let signed = sign_artifact(&artifact, &MEMORY_V1, &kp).unwrap();
    let mut cose_bytes = signed.cose_bytes.clone();

    // Tamper with COSE bytes (flip a byte in the payload area)
    let mid = cose_bytes.len() / 2;
    cose_bytes[mid] ^= 0xFF;

    // Verification should fail (either parse error or sig mismatch).
    // verify_artifact returns Err on parse failure, Ok with valid=false on signature mismatch.
    match verify_artifact(&cose_bytes, Some(&signed.content_hash)) {
        Ok(result) => assert!(!result.valid, "tampered COSE should not verify"),
        Err(_) => { /* parse failure is also acceptable */ }
    }
}

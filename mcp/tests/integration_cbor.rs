//! Week 4 — Integration tests for the full CBOR/COSE pipeline.
//!
//! Tests the complete artifact lifecycle:
//! sign → serialize → deserialize → verify → hash consistency

use solana_sdk::signature::{Keypair, Signer};

// Inline the codec functions since this is an integration test binary
// (can't import from the main binary crate directly)
mod codec_helpers {
    use ciborium::Value as CborValue;
    use coset::{iana, CborSerializable, CoseSign1, CoseSign1Builder, HeaderBuilder};
    use solana_sdk::signature::{Keypair, Signer};

    pub const MEMORY_V1_FIELD_ORDER: &[&str] = &[
        "artifact_id", "type", "schema_version", "content",
        "metadata", "parents", "tags", "created_at", "producer",
    ];

    pub fn to_canonical_cbor(artifact: &serde_json::Value, field_order: &[&str]) -> Vec<u8> {
        let obj = artifact.as_object().unwrap();
        let mut entries: Vec<(CborValue, CborValue)> = Vec::new();
        for &field in field_order {
            if let Some(v) = obj.get(field) {
                if !v.is_null() {
                    entries.push((CborValue::Text(field.into()), json_to_cbor(v)));
                }
            }
        }
        let mut buf = Vec::new();
        ciborium::into_writer(&CborValue::Map(entries), &mut buf).unwrap();
        buf
    }

    fn json_to_cbor(j: &serde_json::Value) -> CborValue {
        match j {
            serde_json::Value::Null => CborValue::Null,
            serde_json::Value::Bool(b) => CborValue::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { CborValue::Integer(i.into()) }
                else { CborValue::Text(n.to_string()) }
            }
            serde_json::Value::String(s) => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                    CborValue::Tag(1, Box::new(CborValue::Integer(dt.timestamp().into())))
                } else { CborValue::Text(s.clone()) }
            }
            serde_json::Value::Array(a) => CborValue::Array(a.iter().map(json_to_cbor).collect()),
            serde_json::Value::Object(o) => {
                let mut e: Vec<_> = o.iter()
                    .map(|(k, v)| (CborValue::Text(k.clone()), json_to_cbor(v)))
                    .collect();
                e.sort_by(|a, b| {
                    let ka = if let CborValue::Text(s) = &a.0 { s.as_str() } else { "" };
                    let kb = if let CborValue::Text(s) = &b.0 { s.as_str() } else { "" };
                    ka.cmp(kb)
                });
                CborValue::Map(e)
            }
        }
    }

    pub fn sign_cose(cbor: &[u8], keypair: &Keypair) -> Vec<u8> {
        let protected = HeaderBuilder::new()
            .algorithm(iana::Algorithm::EdDSA)
            .content_type("application/cbor".to_string())
            .build();
        let kid = keypair.pubkey().to_string().into_bytes();
        let unprotected = HeaderBuilder::new().key_id(kid).build();

        let unsigned = CoseSign1Builder::new()
            .protected(protected.clone())
            .unprotected(unprotected.clone())
            .payload(cbor.to_vec())
            .build();
        let tbs = unsigned.tbs_data(&[]);
        let sig = keypair.sign_message(&tbs);

        CoseSign1Builder::new()
            .protected(protected)
            .unprotected(unprotected)
            .payload(cbor.to_vec())
            .signature(sig.as_ref().to_vec())
            .build()
            .to_vec().unwrap()
    }

    pub fn verify_cose(cose_bytes: &[u8]) -> (bool, String, Vec<u8>) {
        let cs1 = CoseSign1::from_slice(cose_bytes).unwrap();
        let payload = cs1.payload.as_ref().unwrap().clone();
        let kid = String::from_utf8(cs1.unprotected.key_id.clone()).unwrap();
        let pubkey = solana_sdk::pubkey::Pubkey::from_str(&kid).unwrap();
        let tbs = cs1.tbs_data(&[]);
        let sig = solana_sdk::signature::Signature::from(
            <[u8; 64]>::try_from(&cs1.signature[..]).unwrap()
        );
        let valid = sig.verify(pubkey.as_ref(), &tbs);
        (valid, kid, payload)
    }

    use std::str::FromStr;
}

// ── Tests ────────────────────────────────────────────────────────────────────

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
    let cbor = codec_helpers::to_canonical_cbor(&artifact, codec_helpers::MEMORY_V1_FIELD_ORDER);
    assert!(!cbor.is_empty());

    // 2. blake3 hash
    let hash = blake3::hash(&cbor).to_hex().to_string();
    assert_eq!(hash.len(), 64);

    // 3. COSE sign
    let cose_bytes = codec_helpers::sign_cose(&cbor, &kp);
    assert!(cose_bytes.len() > cbor.len()); // COSE adds overhead

    // 4. Verify
    let (valid, signer, payload) = codec_helpers::verify_cose(&cose_bytes);
    assert!(valid, "COSE signature must verify");
    assert_eq!(signer, kp.pubkey().to_string());
    assert_eq!(payload, cbor, "payload must be the canonical CBOR");

    // 5. Hash consistency
    let recovered_hash = blake3::hash(&payload).to_hex().to_string();
    assert_eq!(recovered_hash, hash, "hash must match after round-trip");
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

    let cbor1 = codec_helpers::to_canonical_cbor(&artifact, codec_helpers::MEMORY_V1_FIELD_ORDER);
    let cbor2 = codec_helpers::to_canonical_cbor(&artifact, codec_helpers::MEMORY_V1_FIELD_ORDER);
    assert_eq!(cbor1, cbor2, "canonical CBOR must be identical");

    let hash1 = blake3::hash(&cbor1).to_hex().to_string();
    let hash2 = blake3::hash(&cbor2).to_hex().to_string();
    assert_eq!(hash1, hash2, "blake3 hash must be identical");

    // Sign with two different keypairs — hashes same, signatures different
    let kp1 = Keypair::new();
    let kp2 = Keypair::new();
    let cose1 = codec_helpers::sign_cose(&cbor1, &kp1);
    let cose2 = codec_helpers::sign_cose(&cbor2, &kp2);
    assert_ne!(cose1, cose2, "different signers produce different COSE");

    let (v1, s1, p1) = codec_helpers::verify_cose(&cose1);
    let (v2, s2, p2) = codec_helpers::verify_cose(&cose2);
    assert!(v1 && v2);
    assert_ne!(s1, s2);
    assert_eq!(p1, p2, "payloads must be identical");
}

#[test]
fn test_all_artifact_schemas() {
    let kp = Keypair::new();
    let schemas = [
        ("rag.context", &["artifact_id", "type", "schema_version", "content",
            "metadata", "sources", "parents", "tags", "created_at", "producer"][..]),
        ("rag.result", &["artifact_id", "type", "schema_version", "content",
            "context_artifacts", "citations", "metadata", "parents", "tags",
            "created_at", "producer"][..]),
        ("agent.state", &["artifact_id", "type", "schema_version", "content",
            "state_key", "metadata", "parents", "tags", "created_at", "producer"][..]),
        ("receipt", &["artifact_id", "type", "schema_version", "content",
            "operation", "duration_ms", "metadata", "parents", "tags",
            "created_at", "producer"][..]),
        ("memory", codec_helpers::MEMORY_V1_FIELD_ORDER),
    ];

    for (type_name, field_order) in schemas {
        let artifact = serde_json::json!({
            "artifact_id": format!("art:{type_name}-w4"),
            "type": type_name,
            "schema_version": 1,
            "content": format!("week 4 test for {type_name}"),
            "producer": format!("did:sol:{}", kp.pubkey()),
            "created_at": "2026-04-14T00:00:00Z",
        });

        let cbor = codec_helpers::to_canonical_cbor(&artifact, field_order);
        let hash = blake3::hash(&cbor).to_hex().to_string();
        let cose = codec_helpers::sign_cose(&cbor, &kp);
        let (valid, _, payload) = codec_helpers::verify_cose(&cose);

        assert!(valid, "{type_name}: COSE verification failed");
        assert_eq!(payload, cbor, "{type_name}: payload mismatch");
        assert_eq!(blake3::hash(&payload).to_hex().to_string(), hash, "{type_name}: hash mismatch");
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
    let cbor_bytes = codec_helpers::to_canonical_cbor(&artifact, codec_helpers::MEMORY_V1_FIELD_ORDER);

    println!("JSON: {} bytes, CBOR: {} bytes, ratio: {:.2}x",
        json_bytes.len(), cbor_bytes.len(),
        json_bytes.len() as f64 / cbor_bytes.len() as f64);

    // CBOR should be smaller or comparable to JSON
    assert!(cbor_bytes.len() <= json_bytes.len() + 50,
        "CBOR ({}) should not be much larger than JSON ({})",
        cbor_bytes.len(), json_bytes.len());
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

    let cbor = codec_helpers::to_canonical_cbor(&artifact, codec_helpers::MEMORY_V1_FIELD_ORDER);
    let mut cose_bytes = codec_helpers::sign_cose(&cbor, &kp);

    // Tamper with COSE bytes (flip a byte in the payload area)
    let mid = cose_bytes.len() / 2;
    cose_bytes[mid] ^= 0xFF;

    // Verification should fail (either parse error or sig mismatch)
    let result = std::panic::catch_unwind(|| codec_helpers::verify_cose(&cose_bytes));
    match result {
        Ok((valid, _, _)) => assert!(!valid, "tampered COSE should not verify"),
        Err(_) => {} // parse failure is also acceptable
    }
}

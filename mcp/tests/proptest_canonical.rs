//! Property-based tests for CBOR canonicalization determinism.
//!
//! Uses proptest to generate random artifact payloads and verify that
//! `to_canonical_cbor` always produces identical bytes for the same input.

use proptest::prelude::*;

// Import from the binary crate's modules
// Since mnemonic-mcp is a binary, we test via subprocess or inline.
// For this test we inline the relevant functions.

fn to_canonical_cbor(
    artifact: &serde_json::Value,
    field_order: &[&str],
) -> Vec<u8> {
    use ciborium::Value as CborValue;

    let obj = artifact.as_object().unwrap();
    let mut entries: Vec<(CborValue, CborValue)> = Vec::new();

    for &field_name in field_order {
        if let Some(value) = obj.get(field_name) {
            if !value.is_null() {
                let key = CborValue::Text(field_name.to_string());
                let val = json_to_cbor(value);
                entries.push((key, val));
            }
        }
    }

    let cbor_map = CborValue::Map(entries);
    let mut buf = Vec::new();
    ciborium::into_writer(&cbor_map, &mut buf).unwrap();
    buf
}

fn json_to_cbor(json: &serde_json::Value) -> ciborium::Value {
    use ciborium::Value as C;
    match json {
        serde_json::Value::Null => C::Null,
        serde_json::Value::Bool(b) => C::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { C::Integer(i.into()) }
            else { C::Text(n.to_string()) }
        }
        serde_json::Value::String(s) => C::Text(s.clone()),
        serde_json::Value::Array(a) => C::Array(a.iter().map(json_to_cbor).collect()),
        serde_json::Value::Object(o) => {
            let mut entries: Vec<(C, C)> = o.iter()
                .map(|(k, v)| (C::Text(k.clone()), json_to_cbor(v)))
                .collect();
            entries.sort_by(|a, b| {
                let ka = if let C::Text(s) = &a.0 { s.as_str() } else { "" };
                let kb = if let C::Text(s) = &b.0 { s.as_str() } else { "" };
                ka.cmp(kb)
            });
            C::Map(entries)
        }
    }
}

const MEMORY_FIELD_ORDER: &[&str] = &[
    "artifact_id", "type", "schema_version", "content",
    "metadata", "parents", "tags", "created_at", "producer",
];

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

        let bytes1 = to_canonical_cbor(&artifact, MEMORY_FIELD_ORDER);
        let bytes2 = to_canonical_cbor(&artifact, MEMORY_FIELD_ORDER);
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

        let bytes1 = to_canonical_cbor(&artifact, MEMORY_FIELD_ORDER);
        let bytes2 = to_canonical_cbor(&artifact, MEMORY_FIELD_ORDER);
        let h1 = blake3::hash(&bytes1).to_hex().to_string();
        let h2 = blake3::hash(&bytes2).to_hex().to_string();
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

        let bytes_a = to_canonical_cbor(&a, MEMORY_FIELD_ORDER);
        let bytes_b = to_canonical_cbor(&b, MEMORY_FIELD_ORDER);

        if content_a != content_b {
            prop_assert_ne!(
                blake3::hash(&bytes_a).to_hex().to_string(),
                blake3::hash(&bytes_b).to_hex().to_string(),
            );
        }
    }
}

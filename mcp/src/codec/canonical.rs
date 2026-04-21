//! Canonical CBOR encoding — deterministic serialization for verifiable artifacts.
//!
//! Guarantees:
//! - Same logical artifact → same bytes → same hash, always
//! - Fields ordered per schema's `cbor_field_order` (not alphabetical)
//! - Uses deterministic CBOR (RFC 8949 §4.2): sorted map keys, no indefinite lengths
//! - Timestamps encoded as CBOR tag 1 (epoch-based datetime)
//! - Null/missing optional fields are omitted (not encoded as CBOR null)

use ciborium::Value as CborValue;
use serde_json::Value as JsonValue;

use super::schema::ArtifactSchema;

/// Convert a JSON artifact to canonical CBOR bytes using the schema's field order.
///
/// This function is **pure and deterministic** — calling it N times on the same
/// input always produces identical bytes.
pub fn to_canonical_cbor(artifact: &JsonValue, schema: &ArtifactSchema) -> Result<Vec<u8>, String> {
    let obj = artifact.as_object()
        .ok_or_else(|| "artifact must be a JSON object".to_string())?;

    // Build CBOR map with fields in schema-defined order.
    // Only include fields that are present and non-null.
    let mut entries: Vec<(CborValue, CborValue)> = Vec::new();

    for &field_name in schema.cbor_field_order {
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
    ciborium::into_writer(&cbor_map, &mut buf)
        .map_err(|e| format!("CBOR serialization failed: {e}"))?;

    Ok(buf)
}

/// Deserialize canonical CBOR bytes back to a JSON value.
pub fn from_canonical_cbor(bytes: &[u8]) -> Result<JsonValue, String> {
    let cbor: CborValue = ciborium::from_reader(bytes)
        .map_err(|e| format!("CBOR deserialization failed: {e}"))?;

    Ok(cbor_to_json(&cbor))
}

/// Convert a serde_json::Value to a ciborium::Value.
fn json_to_cbor(json: &JsonValue) -> CborValue {
    match json {
        JsonValue::Null => CborValue::Null,
        JsonValue::Bool(b) => CborValue::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                CborValue::Integer(i.into())
            } else if let Some(u) = n.as_u64() {
                CborValue::Integer(u.into())
            } else {
                // Avoid floats in canonical encoding — encode as text
                CborValue::Text(n.to_string())
            }
        }
        JsonValue::String(s) => {
            // Try to parse ISO 8601 timestamps and encode as CBOR tag 1 (epoch)
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                CborValue::Tag(1, Box::new(CborValue::Integer(dt.timestamp().into())))
            } else {
                CborValue::Text(s.clone())
            }
        }
        JsonValue::Array(arr) => {
            CborValue::Array(arr.iter().map(json_to_cbor).collect())
        }
        JsonValue::Object(obj) => {
            // For nested objects: use sorted keys (deterministic CBOR)
            let mut entries: Vec<(CborValue, CborValue)> = obj.iter()
                .map(|(k, v)| (CborValue::Text(k.clone()), json_to_cbor(v)))
                .collect();
            entries.sort_by(|a, b| {
                // Sort by key text (canonical CBOR requires sorted map keys)
                let ka = if let CborValue::Text(s) = &a.0 { s.as_str() } else { "" };
                let kb = if let CborValue::Text(s) = &b.0 { s.as_str() } else { "" };
                ka.cmp(kb)
            });
            CborValue::Map(entries)
        }
    }
}

/// Convert a ciborium::Value back to a serde_json::Value.
fn cbor_to_json(cbor: &CborValue) -> JsonValue {
    match cbor {
        CborValue::Null => JsonValue::Null,
        CborValue::Bool(b) => JsonValue::Bool(*b),
        CborValue::Integer(i) => {
            let n: i128 = (*i).into();
            if let Ok(i64_val) = i64::try_from(n) {
                JsonValue::Number(i64_val.into())
            } else {
                JsonValue::String(n.to_string())
            }
        }
        CborValue::Float(f) => {
            serde_json::Number::from_f64(*f)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        CborValue::Text(s) => JsonValue::String(s.clone()),
        CborValue::Bytes(b) => {
            JsonValue::String(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, b,
            ))
        }
        CborValue::Array(arr) => {
            JsonValue::Array(arr.iter().map(cbor_to_json).collect())
        }
        CborValue::Map(entries) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in entries {
                let key = match k {
                    CborValue::Text(s) => s.clone(),
                    _ => format!("{:?}", k),
                };
                obj.insert(key, cbor_to_json(v));
            }
            JsonValue::Object(obj)
        }
        CborValue::Tag(1, inner) => {
            // CBOR tag 1 = epoch timestamp → convert back to ISO 8601
            if let CborValue::Integer(epoch) = inner.as_ref() {
                let n: i128 = (*epoch).into();
                if let Some(dt) = chrono::DateTime::from_timestamp(n as i64, 0) {
                    return JsonValue::String(dt.to_rfc3339());
                }
            }
            cbor_to_json(inner)
        }
        CborValue::Tag(_, inner) => cbor_to_json(inner),
        _ => JsonValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::schema::*;

    fn sample_artifact() -> JsonValue {
        serde_json::json!({
            "artifact_id": "art:01JTEST",
            "type": "rag.context",
            "schema_version": 1,
            "content": "The quick brown fox jumps over the lazy dog",
            "producer": "did:sol:7xKXtg2C...",
            "created_at": "2026-04-14T12:00:00Z",
            "tags": ["test", "demo"],
            "metadata": {"source": "unit_test"},
        })
    }

    #[test]
    fn test_canonical_cbor_deterministic() {
        let artifact = sample_artifact();
        let bytes1 = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        let bytes2 = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        assert_eq!(bytes1, bytes2, "canonical CBOR must be deterministic");
    }

    #[test]
    fn test_canonical_cbor_deterministic_1000x() {
        let artifact = sample_artifact();
        let reference = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        for i in 0..1000 {
            let bytes = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
            assert_eq!(bytes, reference, "determinism failed at iteration {i}");
        }
    }

    #[test]
    fn test_roundtrip_json_cbor_json() {
        let artifact = sample_artifact();
        let cbor_bytes = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        let recovered = from_canonical_cbor(&cbor_bytes).unwrap();

        // Check key fields survived the round-trip
        assert_eq!(recovered["artifact_id"], "art:01JTEST");
        assert_eq!(recovered["type"], "rag.context");
        assert_eq!(recovered["content"], "The quick brown fox jumps over the lazy dog");
        assert_eq!(recovered["schema_version"], 1);
    }

    #[test]
    fn test_field_order_matches_schema() {
        let artifact = sample_artifact();
        let cbor_bytes = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        let cbor: CborValue = ciborium::from_reader(&cbor_bytes[..]).unwrap();

        if let CborValue::Map(entries) = cbor {
            let keys: Vec<&str> = entries.iter()
                .filter_map(|(k, _)| if let CborValue::Text(s) = k { Some(s.as_str()) } else { None })
                .collect();

            // Verify ordering matches schema (for present fields only)
            let expected: Vec<&str> = RAG_CONTEXT_V1.cbor_field_order.iter()
                .copied()
                .filter(|f| keys.contains(f))
                .collect();
            assert_eq!(keys, expected, "CBOR field order must match schema");
        } else {
            panic!("expected CBOR map");
        }
    }

    #[test]
    fn test_optional_fields_omitted_when_null() {
        let artifact = serde_json::json!({
            "artifact_id": "art:minimal",
            "type": "rag.context",
            "schema_version": 1,
            "content": "minimal",
            "producer": "did:sol:abc",
            "created_at": "2026-04-14T00:00:00Z",
        });

        let cbor_bytes = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        let recovered = from_canonical_cbor(&cbor_bytes).unwrap();

        // Optional fields should not be present
        assert!(recovered.get("parents").is_none());
        assert!(recovered.get("metadata").is_none());
        assert!(recovered.get("tags").is_none());
    }

    #[test]
    fn test_different_artifacts_different_bytes() {
        let a = serde_json::json!({
            "artifact_id": "art:A", "type": "memory", "schema_version": 1,
            "content": "content A", "producer": "p1", "created_at": "2026-01-01T00:00:00Z",
        });
        let b = serde_json::json!({
            "artifact_id": "art:B", "type": "memory", "schema_version": 1,
            "content": "content B", "producer": "p1", "created_at": "2026-01-01T00:00:00Z",
        });

        let bytes_a = to_canonical_cbor(&a, &MEMORY_V1).unwrap();
        let bytes_b = to_canonical_cbor(&b, &MEMORY_V1).unwrap();
        assert_ne!(bytes_a, bytes_b);
    }

    #[test]
    fn test_timestamp_encoded_as_cbor_tag1() {
        let artifact = sample_artifact();
        let cbor_bytes = to_canonical_cbor(&artifact, &RAG_CONTEXT_V1).unwrap();
        let cbor: CborValue = ciborium::from_reader(&cbor_bytes[..]).unwrap();

        if let CborValue::Map(entries) = cbor {
            let created_at = entries.iter()
                .find(|(k, _)| matches!(k, CborValue::Text(s) if s == "created_at"))
                .map(|(_, v)| v);
            assert!(
                matches!(created_at, Some(CborValue::Tag(1, _))),
                "created_at should be CBOR tag 1 (epoch timestamp)"
            );
        }
    }
}

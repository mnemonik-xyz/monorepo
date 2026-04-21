//! Artifact schema registry — versioned, immutable schemas for typed artifacts.
//!
//! Each schema defines:
//! - required and optional fields
//! - canonical CBOR field order (deterministic serialization)
//! - type identifier and version
//!
//! Schemas are immutable once published. A version bump is required for any
//! field addition. Field order MUST NOT change within a schema version.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum parent references per artifact.
pub const MAX_PARENTS: usize = 16;
/// Maximum DAG depth for cycle detection and traversal.
pub const MAX_DEPTH: usize = 64;

/// Parent reference — links an artifact to its parent(s) in the DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRef {
    pub artifact_id: String,
    /// Optional semantic role: "context", "state", "trigger", "dependency"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Artifact type identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArtifactType {
    #[serde(rename = "rag.context")]
    RagContext,
    #[serde(rename = "rag.result")]
    RagResult,
    #[serde(rename = "agent.state")]
    AgentState,
    #[serde(rename = "receipt")]
    Receipt,
    #[serde(rename = "memory")]
    Memory,
}

impl ArtifactType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RagContext => "rag.context",
            Self::RagResult => "rag.result",
            Self::AgentState => "agent.state",
            Self::Receipt => "receipt",
            Self::Memory => "memory",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "rag.context" => Some(Self::RagContext),
            "rag.result" => Some(Self::RagResult),
            "agent.state" => Some(Self::AgentState),
            "receipt" => Some(Self::Receipt),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }
}

/// Schema definition for an artifact type.
#[derive(Debug, Clone)]
pub struct ArtifactSchema {
    pub artifact_type: ArtifactType,
    pub version: u32,
    pub required_fields: &'static [&'static str],
    pub optional_fields: &'static [&'static str],
    /// Canonical CBOR field order — determines serialization byte sequence.
    /// This order MUST NOT change within a schema version.
    pub cbor_field_order: &'static [&'static str],
}

// ── Schema definitions ──────────────────────────────────────────────────────

/// rag.context.v1 — retrieved chunks + source references
pub const RAG_CONTEXT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::RagContext,
    version: 1,
    required_fields: &["artifact_id", "type", "schema_version", "content", "producer", "created_at"],
    optional_fields: &["parents", "metadata", "tags", "sources"],
    cbor_field_order: &[
        "artifact_id", "type", "schema_version", "content",
        "metadata", "sources", "parents", "tags", "created_at", "producer",
    ],
};

/// rag.result.v1 — answer + context_artifact refs + citations
pub const RAG_RESULT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::RagResult,
    version: 1,
    required_fields: &["artifact_id", "type", "schema_version", "content", "producer", "created_at"],
    optional_fields: &["context_artifacts", "citations", "parents", "metadata", "tags"],
    cbor_field_order: &[
        "artifact_id", "type", "schema_version", "content",
        "context_artifacts", "citations", "metadata", "parents", "tags",
        "created_at", "producer",
    ],
};

/// agent.state.v1 — memory snapshot with parent state ref
pub const AGENT_STATE_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::AgentState,
    version: 1,
    required_fields: &["artifact_id", "type", "schema_version", "content", "producer", "created_at"],
    optional_fields: &["parents", "metadata", "tags", "state_key"],
    cbor_field_order: &[
        "artifact_id", "type", "schema_version", "content",
        "state_key", "metadata", "parents", "tags", "created_at", "producer",
    ],
};

/// receipt.v1 — execution/retrieval receipt
pub const RECEIPT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Receipt,
    version: 1,
    required_fields: &["artifact_id", "type", "schema_version", "content", "producer", "created_at"],
    optional_fields: &["parents", "metadata", "tags", "operation", "duration_ms"],
    cbor_field_order: &[
        "artifact_id", "type", "schema_version", "content",
        "operation", "duration_ms", "metadata", "parents", "tags",
        "created_at", "producer",
    ],
};

/// memory.v1 — backward-compatible with existing sign_memory attestations
pub const MEMORY_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Memory,
    version: 1,
    required_fields: &["artifact_id", "type", "schema_version", "content", "producer", "created_at"],
    optional_fields: &["parents", "metadata", "tags"],
    cbor_field_order: &[
        "artifact_id", "type", "schema_version", "content",
        "metadata", "parents", "tags", "created_at", "producer",
    ],
};

/// Look up schema by type string and version.
pub fn get_schema(artifact_type: &str, version: u32) -> Option<&'static ArtifactSchema> {
    match (artifact_type, version) {
        ("rag.context", 1) => Some(&RAG_CONTEXT_V1),
        ("rag.result", 1) => Some(&RAG_RESULT_V1),
        ("agent.state", 1) => Some(&AGENT_STATE_V1),
        ("receipt", 1) => Some(&RECEIPT_V1),
        ("memory", 1) => Some(&MEMORY_V1),
        _ => None,
    }
}

/// Validate that an artifact JSON object has all required fields for its schema.
pub fn validate_artifact(artifact: &serde_json::Value, schema: &ArtifactSchema) -> Result<(), String> {
    let obj = artifact.as_object()
        .ok_or_else(|| "artifact must be a JSON object".to_string())?;

    for &field in schema.required_fields {
        if !obj.contains_key(field) || obj[field].is_null() {
            return Err(format!("missing required field: {field}"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_lookup() {
        assert!(get_schema("rag.context", 1).is_some());
        assert!(get_schema("rag.result", 1).is_some());
        assert!(get_schema("agent.state", 1).is_some());
        assert!(get_schema("receipt", 1).is_some());
        assert!(get_schema("memory", 1).is_some());
        assert!(get_schema("unknown", 1).is_none());
        assert!(get_schema("rag.context", 2).is_none());
    }

    #[test]
    fn test_validate_artifact() {
        let valid = serde_json::json!({
            "artifact_id": "art:test",
            "type": "rag.context",
            "schema_version": 1,
            "content": "test content",
            "producer": "did:sol:abc",
            "created_at": "2026-04-14T00:00:00Z",
        });
        assert!(validate_artifact(&valid, &RAG_CONTEXT_V1).is_ok());

        let missing = serde_json::json!({
            "artifact_id": "art:test",
            "type": "rag.context",
        });
        assert!(validate_artifact(&missing, &RAG_CONTEXT_V1).is_err());
    }

    #[test]
    fn test_artifact_type_strings() {
        assert_eq!(ArtifactType::RagContext.as_str(), "rag.context");
        assert_eq!(ArtifactType::from_str("receipt"), Some(ArtifactType::Receipt));
        assert_eq!(ArtifactType::from_str("invalid"), None);
    }

    #[test]
    fn test_cbor_field_order_covers_required() {
        for schema in [&RAG_CONTEXT_V1, &RAG_RESULT_V1, &AGENT_STATE_V1, &RECEIPT_V1, &MEMORY_V1] {
            for &field in schema.required_fields {
                assert!(
                    schema.cbor_field_order.contains(&field),
                    "schema {:?} v{}: required field '{}' not in cbor_field_order",
                    schema.artifact_type, schema.version, field,
                );
            }
        }
    }
}

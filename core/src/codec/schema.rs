//! Artifact schema registry -- versioned, immutable schemas for typed artifacts.
//!
//! Each schema defines:
//! - required and optional fields
//! - canonical CBOR field order (deterministic serialization)
//! - type identifier and version
//!
//! Schemas are immutable once published. A version bump is required for any
//! field addition. Field order MUST NOT change within a schema version.

use serde::{Deserialize, Serialize};

/// Maximum parent references per artifact.
pub const MAX_PARENTS: usize = 16;
/// Maximum DAG depth for cycle detection and traversal.
pub const MAX_DEPTH: usize = 64;

/// Parent reference -- links an artifact to its parent(s) in the DAG.
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
    /// A published blog post (a signed PUBLIC attestation).
    #[serde(rename = "post")]
    Post,
    /// One ordered, hash-linked step in an agent trajectory.
    #[cfg(feature = "trajectory-experimental")]
    #[serde(rename = "step")]
    Step,
    /// An independent judge's correctness/quality verdict over a step.
    #[cfg(feature = "trajectory-experimental")]
    #[serde(rename = "verdict")]
    Verdict,
    /// Anchored summary of a trajectory (batch root + coverage).
    #[cfg(feature = "trajectory-experimental")]
    #[serde(rename = "trajectory")]
    Trajectory,
    /// A principal-signed typed mandate (AP2-aligned Intent Mandate).
    #[cfg(feature = "correspondence-experimental")]
    #[serde(rename = "intent")]
    Intent,
    /// An agent action that references an Intent + carries a correspondence cert.
    #[cfg(feature = "correspondence-experimental")]
    #[serde(rename = "action")]
    Action,
}

impl ArtifactType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RagContext => "rag.context",
            Self::RagResult => "rag.result",
            Self::AgentState => "agent.state",
            Self::Receipt => "receipt",
            Self::Memory => "memory",
            Self::Post => "post",
            #[cfg(feature = "trajectory-experimental")]
            Self::Step => "step",
            #[cfg(feature = "trajectory-experimental")]
            Self::Verdict => "verdict",
            #[cfg(feature = "trajectory-experimental")]
            Self::Trajectory => "trajectory",
            #[cfg(feature = "correspondence-experimental")]
            Self::Intent => "intent",
            #[cfg(feature = "correspondence-experimental")]
            Self::Action => "action",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "rag.context" => Some(Self::RagContext),
            "rag.result" => Some(Self::RagResult),
            "agent.state" => Some(Self::AgentState),
            "receipt" => Some(Self::Receipt),
            "memory" => Some(Self::Memory),
            "post" => Some(Self::Post),
            #[cfg(feature = "trajectory-experimental")]
            "step" => Some(Self::Step),
            #[cfg(feature = "trajectory-experimental")]
            "verdict" => Some(Self::Verdict),
            #[cfg(feature = "trajectory-experimental")]
            "trajectory" => Some(Self::Trajectory),
            #[cfg(feature = "correspondence-experimental")]
            "intent" => Some(Self::Intent),
            #[cfg(feature = "correspondence-experimental")]
            "action" => Some(Self::Action),
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
    /// Canonical CBOR field order -- determines serialization byte sequence.
    /// This order MUST NOT change within a schema version.
    pub cbor_field_order: &'static [&'static str],
}

// -- Schema definitions --

/// rag.context.v1 -- retrieved chunks + source references
pub const RAG_CONTEXT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::RagContext,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "producer",
        "created_at",
    ],
    optional_fields: &["parents", "metadata", "tags", "sources"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "metadata",
        "sources",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// rag.result.v1 -- answer + context_artifact refs + citations
pub const RAG_RESULT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::RagResult,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "producer",
        "created_at",
    ],
    optional_fields: &[
        "context_artifacts",
        "citations",
        "parents",
        "metadata",
        "tags",
    ],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "context_artifacts",
        "citations",
        "metadata",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// agent.state.v1 -- memory snapshot with parent state ref
pub const AGENT_STATE_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::AgentState,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "producer",
        "created_at",
    ],
    optional_fields: &["parents", "metadata", "tags", "state_key"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "state_key",
        "metadata",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// receipt.v1 -- execution/retrieval receipt
pub const RECEIPT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Receipt,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "producer",
        "created_at",
    ],
    optional_fields: &["parents", "metadata", "tags", "operation", "duration_ms"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "operation",
        "duration_ms",
        "metadata",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// memory.v1 -- backward-compatible with existing sign_memory attestations
pub const MEMORY_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Memory,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "producer",
        "created_at",
    ],
    optional_fields: &["parents", "metadata", "tags"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "metadata",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// post.v1 -- a blog post IS a signed PUBLIC attestation (Decision 5 / 8).
///
/// Reuses the same CBOR/COSE_Sign1/blake3 pipeline as every other artifact:
/// there is no parallel post store, and authorship is provable via the
/// COSE_Sign1 Ed25519 signer. The post body is carried in the standard
/// `content` field (same slot `sign_memory` uses), so `content_hash` commits
/// to the rendered-markdown source exactly as for a memory. The post-specific
/// fields — `title`, `slug`, `published_at`, and the optional human-readable
/// `author` — sit alongside it; `producer` remains the cryptographic signer
/// identity (distinct from the display `author`).
pub const POST_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Post,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "producer",
        "created_at",
        "title",
        "slug",
        "published_at",
    ],
    optional_fields: &["parents", "metadata", "tags", "author"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "title",
        "slug",
        "content",
        "author",
        "published_at",
        "metadata",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// step.v1 -- one ordered, hash-linked step in an agent trajectory.
///
/// `prev_hash` is REQUIRED in `cbor_field_order` (it is part of the signed
/// payload — that is what makes the chain link tamper-evident) but it is a
/// nullable field at the value level: the genesis step (`seq == 0`) carries
/// `prev_hash: null`. `seq` and `trajectory_id` are required.
#[cfg(feature = "trajectory-experimental")]
pub const STEP_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Step,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "trajectory_id",
        "seq",
        "producer",
        "created_at",
    ],
    optional_fields: &["prev_hash", "verdict_hash", "parents", "metadata", "tags"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "trajectory_id",
        "seq",
        "prev_hash",
        "verdict_hash",
        "metadata",
        "parents",
        "tags",
        "created_at",
        "producer",
    ],
};

/// verdict.v1 -- an independent judge's verdict over a step. Signed by the judge
/// identity, which MUST differ from the step `producer` (enforced at attest
/// time). `status` ∈ {"pass","concern","reject"}. `proof_ref` optionally binds
/// an external correctness proof (zkML/TEE/opML/OCP) by hash.
#[cfg(feature = "trajectory-experimental")]
pub const VERDICT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Verdict,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "step_hash",
        "status",
        "judge",
        "created_at",
    ],
    optional_fields: &["score", "proof_ref", "proof_kind", "rationale", "tags"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "step_hash",
        "status",
        "score",
        "proof_ref",
        "proof_kind",
        "rationale",
        "tags",
        "created_at",
        "judge",
    ],
};

/// trajectory.v1 -- anchored summary of a trajectory (or one checkpoint of it).
/// `batch_root` is the order-preserving Merkle root over the steps' content
/// hashes (== the Arweave bundle manifest root). `prev_root` links this
/// checkpoint to the prior one (root-of-roots).
#[cfg(feature = "trajectory-experimental")]
pub const TRAJECTORY_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Trajectory,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "trajectory_id",
        "step_count",
        "batch_root",
        "producer",
        "created_at",
    ],
    optional_fields: &[
        "verdict_coverage",
        "chain_valid",
        "is_final",
        "prev_root",
        "tags",
    ],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "trajectory_id",
        "step_count",
        "batch_root",
        "prev_root",
        "verdict_coverage",
        "chain_valid",
        "is_final",
        "tags",
        "created_at",
        "producer",
    ],
};

/// intent.v1 -- a principal-signed typed mandate (AP2-aligned Intent Mandate).
/// `constraints` carries the typed policy (limits, allowlist roots, a `policy_id`
/// naming the guest program + its params). `intent_hash = blake3(canonical_cbor)`
/// is what an `action.intent_ref` points back to.
#[cfg(feature = "correspondence-experimental")]
pub const INTENT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Intent,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "constraints",
        "producer",
        "created_at",
    ],
    optional_fields: &["expiry", "nonce", "metadata", "tags"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "constraints",
        "expiry",
        "nonce",
        "metadata",
        "tags",
        "created_at",
        "producer",
    ],
};

/// action.v1 -- an agent action bound to an Intent. `intent_ref` MUST equal the
/// referenced `INTENT_V1.content_hash`. `knowledge_refs` lists the hashes of the
/// memories the agent retrieved at decision time (the knowledge link). The
/// correspondence certificate rides in `metadata.correspondence` (so it is part
/// of the signed payload without a new top-level field).
#[cfg(feature = "correspondence-experimental")]
pub const ACTION_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Action,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "intent_ref",
        "producer",
        "created_at",
    ],
    optional_fields: &["knowledge_refs", "metadata", "tags"],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "content",
        "intent_ref",
        "knowledge_refs",
        "metadata",
        "tags",
        "created_at",
        "producer",
    ],
};

/// The pre-certificate field set whose blake3 IS the `action_commitment` bound
/// into the proof's public inputs. It deliberately EXCLUDES `metadata` (which
/// carries the cert) — that is the circularity fix: the proof commits to the
/// action's content, not to the envelope that contains the proof. `to_canonical_cbor`
/// only emits fields listed here, so passing the full action artifact is safe.
#[cfg(feature = "correspondence-experimental")]
pub const ACTION_COMMIT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Action,
    version: 1,
    required_fields: &["content", "intent_ref", "producer", "created_at"],
    optional_fields: &["knowledge_refs"],
    cbor_field_order: &[
        "content",
        "intent_ref",
        "knowledge_refs",
        "created_at",
        "producer",
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
        ("post", 1) => Some(&POST_V1),
        #[cfg(feature = "trajectory-experimental")]
        ("step", 1) => Some(&STEP_V1),
        #[cfg(feature = "trajectory-experimental")]
        ("verdict", 1) => Some(&VERDICT_V1),
        #[cfg(feature = "trajectory-experimental")]
        ("trajectory", 1) => Some(&TRAJECTORY_V1),
        #[cfg(feature = "correspondence-experimental")]
        ("intent", 1) => Some(&INTENT_V1),
        #[cfg(feature = "correspondence-experimental")]
        ("action", 1) => Some(&ACTION_V1),
        _ => None,
    }
}

/// Validate that an artifact JSON object has all required fields for its schema.
pub fn validate_artifact(
    artifact: &serde_json::Value,
    schema: &ArtifactSchema,
) -> Result<(), String> {
    let obj = artifact
        .as_object()
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
    fn test_post_v1_validates_required_fields() {
        // A POST_V1 artifact missing a post-specific required field is rejected.
        let full = serde_json::json!({
            "artifact_id": "art:post-1",
            "type": "post",
            "schema_version": 1,
            "content": "# Hello\n\nbody markdown",
            "producer": "did:sol:abc",
            "created_at": "2026-06-27T00:00:00Z",
            "title": "Hello",
            "slug": "hello",
            "published_at": "2026-06-27T00:00:00Z",
        });
        assert!(validate_artifact(&full, &POST_V1).is_ok());

        let missing_slug = serde_json::json!({
            "artifact_id": "art:post-1",
            "type": "post",
            "schema_version": 1,
            "content": "body",
            "producer": "did:sol:abc",
            "created_at": "2026-06-27T00:00:00Z",
            "title": "Hello",
            "published_at": "2026-06-27T00:00:00Z",
        });
        assert!(validate_artifact(&missing_slug, &POST_V1).is_err());
    }

    #[test]
    fn test_artifact_type_strings() {
        assert_eq!(ArtifactType::RagContext.as_str(), "rag.context");
        assert_eq!(
            ArtifactType::from_str("receipt"),
            Some(ArtifactType::Receipt)
        );
        assert_eq!(ArtifactType::from_str("invalid"), None);
    }

    #[cfg(feature = "trajectory-experimental")]
    #[test]
    fn trajectory_schema_lookup() {
        assert!(get_schema("step", 1).is_some());
        assert!(get_schema("verdict", 1).is_some());
        assert!(get_schema("trajectory", 1).is_some());
        assert_eq!(ArtifactType::from_str("step"), Some(ArtifactType::Step));
        assert_eq!(ArtifactType::Verdict.as_str(), "verdict");
    }

    #[cfg(feature = "trajectory-experimental")]
    #[test]
    fn step_prev_hash_in_field_order() {
        // prev_hash MUST be in the signed payload for the chain link to be
        // tamper-evident, even though it is value-nullable for the genesis step.
        assert!(STEP_V1.cbor_field_order.contains(&"prev_hash"));
        assert!(STEP_V1.required_fields.contains(&"trajectory_id"));
        assert!(STEP_V1.required_fields.contains(&"seq"));
    }

    #[cfg(feature = "trajectory-experimental")]
    #[test]
    fn trajectory_schemas_field_order_covers_required() {
        for schema in [&STEP_V1, &VERDICT_V1, &TRAJECTORY_V1] {
            for &field in schema.required_fields {
                assert!(
                    schema.cbor_field_order.contains(&field),
                    "schema {:?}: required field '{}' not in cbor_field_order",
                    schema.artifact_type,
                    field,
                );
            }
        }
    }

    #[test]
    fn test_cbor_field_order_covers_required() {
        for schema in [
            &RAG_CONTEXT_V1,
            &RAG_RESULT_V1,
            &AGENT_STATE_V1,
            &RECEIPT_V1,
            &MEMORY_V1,
        ] {
            for &field in schema.required_fields {
                assert!(
                    schema.cbor_field_order.contains(&field),
                    "schema {:?} v{}: required field '{}' not in cbor_field_order",
                    schema.artifact_type,
                    schema.version,
                    field,
                );
            }
        }
    }
}

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
    /// An encrypted, sealed memory artifact (AEAD-wrapped payload).
    #[serde(rename = "sealed")]
    Sealed,
    /// A decryption grant -- wraps the content key for a specific reader.
    #[serde(rename = "grant")]
    Grant,
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
    /// An A2A Task object (A2A v1.0.0-rc).
    #[cfg(feature = "a2a-experimental")]
    #[serde(rename = "a2a.task")]
    A2aTask,
    /// An A2A Message object (A2A v1.0.0-rc).
    #[cfg(feature = "a2a-experimental")]
    #[serde(rename = "a2a.message")]
    A2aMessage,
    /// An A2A Artifact object (A2A v1.0.0-rc).
    #[cfg(feature = "a2a-experimental")]
    #[serde(rename = "a2a.artifact")]
    A2aArtifact,
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
            Self::Sealed => "sealed",
            Self::Grant => "grant",
            #[cfg(feature = "trajectory-experimental")]
            Self::Step => "step",
            #[cfg(feature = "trajectory-experimental")]
            Self::Verdict => "verdict",
            #[cfg(feature = "trajectory-experimental")]
            Self::Trajectory => "trajectory",
            #[cfg(feature = "a2a-experimental")]
            Self::A2aTask => "a2a.task",
            #[cfg(feature = "a2a-experimental")]
            Self::A2aMessage => "a2a.message",
            #[cfg(feature = "a2a-experimental")]
            Self::A2aArtifact => "a2a.artifact",
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
            "sealed" => Some(Self::Sealed),
            "grant" => Some(Self::Grant),
            #[cfg(feature = "trajectory-experimental")]
            "step" => Some(Self::Step),
            #[cfg(feature = "trajectory-experimental")]
            "verdict" => Some(Self::Verdict),
            #[cfg(feature = "trajectory-experimental")]
            "trajectory" => Some(Self::Trajectory),
            #[cfg(feature = "a2a-experimental")]
            "a2a.task" => Some(Self::A2aTask),
            #[cfg(feature = "a2a-experimental")]
            "a2a.message" => Some(Self::A2aMessage),
            #[cfg(feature = "a2a-experimental")]
            "a2a.artifact" => Some(Self::A2aArtifact),
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
    /// Fields that carry raw binary data and must encode as CBOR bstr.
    /// In JSON they are represented as standard base64 (no URL encoding, no padding
    /// stripping). `to_canonical_cbor` decodes them before encoding as bstr;
    /// `from_canonical_cbor` re-encodes them as base64 on the way out.
    pub bytes_fields: &'static [&'static str],
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
    bytes_fields: &[],
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
    bytes_fields: &[],
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
    bytes_fields: &[],
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
    bytes_fields: &[],
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
    // `visibility` and `anchor` are additive and OPTIONAL on purpose
    // (work/arweave-as-source-of-truth, D-1). `to_canonical_cbor` writes only
    // present, non-null fields, so an artifact that omits them hashes exactly
    // as it did before they existed — no `MEMORY_V2`, no migration, no moved
    // `content_hash`. Only anchored writes populate them, because an anchored
    // artifact is verified from its Arweave bytes and never rebuilt from
    // columns (`rebuild_content_hash` keeps the pre-existing field set).
    // NEVER reorder `cbor_field_order` and never promote these to required:
    // either change would move every existing hash.
    optional_fields: &["parents", "metadata", "tags", "visibility", "anchor"],
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
        "visibility",
        "anchor",
    ],
    bytes_fields: &[],
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
    bytes_fields: &[],
};

/// sealed.v1 -- AEAD-encrypted memory artifact.
///
/// The plaintext payload is encrypted and stored in `ct` (ciphertext bstr).
/// `nonce` is the AEAD nonce (bstr). `kc` is the content key commitment
/// (e.g. blake3 hash of the plaintext key, bstr). `wraps` lists grant
/// artifact_ids that carry the encrypted content key for authorised readers.
/// `alg` names the AEAD algorithm (e.g. "AES-256-GCM").
pub const SEALED_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Sealed,
    version: 1,
    required_fields: &[
        "artifact_id",
        "type",
        "schema_version",
        "alg",
        "nonce",
        "ct",
        "kc",
        "wraps",
        "created_at",
        "producer",
    ],
    optional_fields: &[],
    cbor_field_order: &[
        "artifact_id",
        "type",
        "schema_version",
        "alg",
        "nonce",
        "ct",
        "kc",
        "wraps",
        "created_at",
        "producer",
    ],
    bytes_fields: &["nonce", "ct", "kc"],
};

/// grant.v1 -- decryption grant wrapping a content key for a reader.
///
/// `enc` is the wrapped (encrypted) content key bstr. `wk` is the wrapping
/// key identifier / ephemeral public key material (bstr). `memory_hash` is
/// the blake3 hash of the sealed artifact this grant unlocks. `reader` is
/// the DID of the authorised reader (optional -- grants may be broadcast).
/// `perms` carries an optional permission token / expiry JSON object.
pub const GRANT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::Grant,
    version: 1,
    required_fields: &[
        "type",
        "schema_version",
        "memory_hash",
        "enc",
        "wk",
        "created_at",
        "producer",
    ],
    optional_fields: &["reader", "perms"],
    cbor_field_order: &[
        "type",
        "schema_version",
        "memory_hash",
        "enc",
        "wk",
        "reader",
        "perms",
        "created_at",
        "producer",
    ],
    bytes_fields: &["enc", "wk"],
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
    bytes_fields: &[],
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
    bytes_fields: &[],
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
    bytes_fields: &[],
};

/// a2a.task.v1 -- A2A Task object (A2A v1.0.0-rc).
///
/// Required fields match the A2A spec wire format exactly.
/// `history` and `artifacts` are optional arrays of nested objects.
#[cfg(feature = "a2a-experimental")]
pub const A2A_TASK_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::A2aTask,
    version: 1,
    required_fields: &["id", "contextId", "status"],
    optional_fields: &["history", "artifacts"],
    cbor_field_order: &["id", "contextId", "status", "history", "artifacts"],
    bytes_fields: &[],
};

/// a2a.message.v1 -- A2A Message object (A2A v1.0.0-rc).
///
/// `role` ∈ {"user","agent"}. `parts` is a required array of Part objects.
#[cfg(feature = "a2a-experimental")]
pub const A2A_MESSAGE_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::A2aMessage,
    version: 1,
    required_fields: &["role", "parts", "messageId"],
    optional_fields: &["taskId", "contextId"],
    cbor_field_order: &["role", "parts", "messageId", "taskId", "contextId"],
    bytes_fields: &[],
};

/// a2a.artifact.v1 -- A2A Artifact object (A2A v1.0.0-rc).
///
/// `parts` is required. `artifactId` and `name` are optional.
#[cfg(feature = "a2a-experimental")]
pub const A2A_ARTIFACT_V1: ArtifactSchema = ArtifactSchema {
    artifact_type: ArtifactType::A2aArtifact,
    version: 1,
    required_fields: &["parts"],
    optional_fields: &["artifactId", "name"],
    cbor_field_order: &["artifactId", "name", "parts"],
    bytes_fields: &[],
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
        ("sealed", 1) => Some(&SEALED_V1),
        ("grant", 1) => Some(&GRANT_V1),
        #[cfg(feature = "trajectory-experimental")]
        ("step", 1) => Some(&STEP_V1),
        #[cfg(feature = "trajectory-experimental")]
        ("verdict", 1) => Some(&VERDICT_V1),
        #[cfg(feature = "trajectory-experimental")]
        ("trajectory", 1) => Some(&TRAJECTORY_V1),
        #[cfg(feature = "a2a-experimental")]
        ("a2a.task", 1) => Some(&A2A_TASK_V1),
        #[cfg(feature = "a2a-experimental")]
        ("a2a.message", 1) => Some(&A2A_MESSAGE_V1),
        #[cfg(feature = "a2a-experimental")]
        ("a2a.artifact", 1) => Some(&A2A_ARTIFACT_V1),
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

    // -- sealed.v1 + grant.v1 tests --

    #[test]
    fn sealed_v1_schema_lookup() {
        let s = get_schema("sealed", 1).expect("sealed v1 must be registered");
        assert_eq!(s.version, 1);
        assert!(matches!(s.artifact_type, ArtifactType::Sealed));
    }

    #[test]
    fn grant_v1_schema_lookup() {
        let s = get_schema("grant", 1).expect("grant v1 must be registered");
        assert_eq!(s.version, 1);
        assert!(matches!(s.artifact_type, ArtifactType::Grant));
    }

    #[test]
    fn sealed_v1_field_order_stable() {
        // cbor_field_order must be exactly the canonical order specified in the task.
        let expected = [
            "artifact_id",
            "type",
            "schema_version",
            "alg",
            "nonce",
            "ct",
            "kc",
            "wraps",
            "created_at",
            "producer",
        ];
        assert_eq!(SEALED_V1.cbor_field_order, &expected[..]);
    }

    #[test]
    fn grant_v1_field_order_stable() {
        let expected = [
            "type",
            "schema_version",
            "memory_hash",
            "enc",
            "wk",
            "reader",
            "perms",
            "created_at",
            "producer",
        ];
        assert_eq!(GRANT_V1.cbor_field_order, &expected[..]);
    }

    #[test]
    fn sealed_v1_required_field_order_covers_cbor_order() {
        for &field in SEALED_V1.required_fields {
            assert!(
                SEALED_V1.cbor_field_order.contains(&field),
                "sealed v1: required field '{}' not in cbor_field_order",
                field
            );
        }
    }

    #[test]
    fn grant_v1_required_field_order_covers_cbor_order() {
        for &field in GRANT_V1.required_fields {
            assert!(
                GRANT_V1.cbor_field_order.contains(&field),
                "grant v1: required field '{}' not in cbor_field_order",
                field
            );
        }
    }

    #[test]
    fn sealed_v1_missing_required_field_rejected() {
        // Missing `ct` -- must fail validation.
        let art = serde_json::json!({
            "artifact_id": "art:sealed-1",
            "type": "sealed",
            "schema_version": 1,
            "alg": "AES-256-GCM",
            "nonce": "AAAAAAAAAAAAAAAA",
            // ct is intentionally absent
            "kc": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "wraps": [],
            "created_at": "2026-09-28T00:00:00Z",
            "producer": "did:sol:test",
        });
        let err = validate_artifact(&art, &SEALED_V1);
        assert!(err.is_err(), "missing 'ct' must be rejected");
        assert!(err.unwrap_err().contains("ct"));
    }

    #[test]
    fn grant_v1_missing_required_field_rejected() {
        // Missing `enc` -- must fail.
        let art = serde_json::json!({
            "type": "grant",
            "schema_version": 1,
            "memory_hash": "abc123",
            // enc absent
            "wk": "AAAA",
            "created_at": "2026-09-28T00:00:00Z",
            "producer": "did:sol:test",
        });
        let err = validate_artifact(&art, &GRANT_V1);
        assert!(err.is_err(), "missing 'enc' must be rejected");
        assert!(err.unwrap_err().contains("enc"));
    }

    #[test]
    fn memory_v1_golden_fixtures_unchanged() {
        // Ensure existing MEMORY_V1 schema is not disturbed.
        assert_eq!(MEMORY_V1.version, 1);
        assert!(matches!(MEMORY_V1.artifact_type, ArtifactType::Memory));
        // Field order hasn't changed.
        let expected = [
            "artifact_id",
            "type",
            "schema_version",
            "content",
            "metadata",
            "parents",
            "tags",
            "created_at",
            "producer",
            "visibility",
            "anchor",
        ];
        assert_eq!(MEMORY_V1.cbor_field_order, &expected[..]);
        // Required fields unchanged.
        assert!(MEMORY_V1.required_fields.contains(&"content"));
        assert!(MEMORY_V1.required_fields.contains(&"artifact_id"));
        assert!(MEMORY_V1.bytes_fields.is_empty());
    }
}

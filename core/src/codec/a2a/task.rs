//! A2A Task type -- Rust mirror of the A2A v1.0.0-rc Task wire object.
//!
//! Field names match the A2A spec exactly (camelCase as in the JSON wire format).
//! `to_jcs_bytes` produces RFC 8785 canonical JSON; `to_canonical_envelope` wraps
//! the JCS bytes in a COSE_Sign1 envelope.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;

use super::part::Part;
use crate::codec::sign::sign_cose;

/// A2A Task status string (open, working, completed, cancelled, failed, unknown).
pub type TaskStatus = String;

/// An A2A Task artifact (different from the mnemonik Artifact -- this is the
/// A2A spec's nested artifact inside a Task).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskArtifact {
    #[serde(rename = "artifactId", skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub parts: Vec<Part>,
}

/// A2A Task object (A2A v1.0.0-rc).
///
/// Rust field names are snake_case; serialized JSON uses the A2A camelCase wire
/// names via `#[serde(rename)]` so the wire format is byte-for-byte spec compliant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(rename = "contextId")]
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<TaskArtifact>>,
}

impl Task {
    /// Serialize to RFC 8785 JCS canonical JSON bytes.
    ///
    /// Calling this twice on the same value always produces identical bytes.
    pub fn to_jcs_bytes(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_value(self).context("Task: serde_json::to_value")?;
        serde_jcs::to_vec(&json).context("Task: serde_jcs::to_vec")
    }

    /// Build a COSE_Sign1 envelope over the JCS payload, signed with `keypair`.
    ///
    /// `prev_id` is reserved for future hash-linking; currently unused but
    /// accepted to keep the API stable across spec revisions.
    pub fn to_canonical_envelope(
        &self,
        keypair: &Keypair,
        _prev_id: Option<&str>,
    ) -> Result<Vec<u8>> {
        let jcs = self.to_jcs_bytes()?;
        sign_cose(&jcs, keypair).map_err(|e| anyhow::anyhow!("Task: COSE sign failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task() -> Task {
        Task {
            id: "task-001".to_string(),
            context_id: "ctx-abc".to_string(),
            status: "working".to_string(),
            history: None,
            artifacts: None,
        }
    }

    #[test]
    fn roundtrip_jcs_stable() {
        let task = sample_task();
        let b1 = task.to_jcs_bytes().unwrap();
        let b2 = task.to_jcs_bytes().unwrap();
        assert_eq!(b1, b2, "JCS output must be stable across two calls");
    }

    #[test]
    fn roundtrip_envelope_signs_and_verifies() {
        use crate::codec::sign::verify_artifact;
        use solana_sdk::signature::{Keypair, Signer};

        let kp = Keypair::new();
        let task = sample_task();
        let envelope = task.to_canonical_envelope(&kp, None).unwrap();

        let result = verify_artifact(&envelope, None).expect("verify_artifact must not error");
        assert!(result.cose_signature, "Ed25519 signature must verify");
        assert!(result.valid, "full verification must pass");
        assert_eq!(
            result.signer,
            kp.pubkey().to_string(),
            "signer must match keypair pubkey"
        );
    }

    #[test]
    fn field_order_invariance() {
        // Build the same logical task twice, relying on JCS to canonicalize.
        // We test that the JCS output of two tasks with identical logical content
        // but constructed via different field insertion order is identical.
        // serde_json::Value construction is insertion-ordered, so we use two
        // different JSON strings parsed into Value and then re-serialized.
        let json_a = serde_json::json!({
            "id": "task-001",
            "contextId": "ctx-abc",
            "status": "working"
        });
        let json_b = serde_json::json!({
            "status": "working",
            "id": "task-001",
            "contextId": "ctx-abc"
        });

        let bytes_a = serde_jcs::to_vec(&json_a).unwrap();
        let bytes_b = serde_jcs::to_vec(&json_b).unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "JCS must produce identical bytes regardless of insertion order"
        );
    }
}

//! A2A Artifact type -- Rust mirror of the A2A v1.0.0-rc Artifact wire object.
//!
//! Field names match the A2A spec exactly. `to_jcs_bytes` produces RFC 8785
//! canonical JSON; `to_canonical_envelope` wraps in a COSE_Sign1 envelope.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;

use super::part::Part;
use crate::codec::sign::sign_cose;

/// A2A Artifact object (A2A v1.0.0-rc).
///
/// Named `A2aArtifact` to avoid collision with the existing mnemonik
/// `ArtifactSchema` and related types in the codebase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct A2aArtifact {
    #[serde(rename = "artifactId", skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub parts: Vec<Part>,
}

impl A2aArtifact {
    /// Serialize to RFC 8785 JCS canonical JSON bytes.
    pub fn to_jcs_bytes(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_value(self).context("A2aArtifact: serde_json::to_value")?;
        serde_jcs::to_vec(&json).context("A2aArtifact: serde_jcs::to_vec")
    }

    /// Build a COSE_Sign1 envelope over the JCS payload.
    pub fn to_canonical_envelope(
        &self,
        keypair: &Keypair,
        _prev_id: Option<&str>,
    ) -> Result<Vec<u8>> {
        let jcs = self.to_jcs_bytes()?;
        sign_cose(&jcs, keypair).map_err(|e| anyhow::anyhow!("A2aArtifact: COSE sign failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::a2a::part::Part;

    fn sample_artifact() -> A2aArtifact {
        A2aArtifact {
            artifact_id: Some("art-001".to_string()),
            name: Some("report".to_string()),
            parts: vec![Part::Text {
                text: "artifact content".to_string(),
            }],
        }
    }

    #[test]
    fn roundtrip_jcs_stable() {
        let art = sample_artifact();
        let b1 = art.to_jcs_bytes().unwrap();
        let b2 = art.to_jcs_bytes().unwrap();
        assert_eq!(b1, b2, "A2aArtifact JCS bytes must be stable");
    }

    #[test]
    fn roundtrip_envelope_signs_and_verifies() {
        use crate::codec::sign::verify_artifact;
        use solana_sdk::signature::Keypair;

        let kp = Keypair::new();
        let art = sample_artifact();
        let envelope = art.to_canonical_envelope(&kp, None).unwrap();

        let result = verify_artifact(&envelope, None).expect("verify must not error");
        assert!(result.cose_signature, "signature must verify");
        assert!(result.valid);
    }
}

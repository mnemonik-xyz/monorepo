//! A2A Message type -- Rust mirror of the A2A v1.0.0-rc Message wire object.
//!
//! Field names match the A2A spec exactly (camelCase). `to_jcs_bytes` produces
//! RFC 8785 canonical JSON; `to_canonical_envelope` wraps in a COSE_Sign1 envelope.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;

use super::part::Part;
use crate::codec::sign::sign_cose;

/// A2A Message object (A2A v1.0.0-rc).
///
/// Rust field names are snake_case; serialized JSON uses the A2A camelCase wire
/// names via `#[serde(rename)]`.
/// `role` ∈ {"user", "agent"}. `parts` is the list of content parts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub parts: Vec<Part>,
    #[serde(rename = "messageId")]
    pub message_id: String,
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
}

impl Message {
    /// Serialize to RFC 8785 JCS canonical JSON bytes.
    pub fn to_jcs_bytes(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_value(self).context("Message: serde_json::to_value")?;
        serde_jcs::to_vec(&json).context("Message: serde_jcs::to_vec")
    }

    /// Build a COSE_Sign1 envelope over the JCS payload.
    pub fn to_canonical_envelope(
        &self,
        keypair: &Keypair,
        _prev_id: Option<&str>,
    ) -> Result<Vec<u8>> {
        let jcs = self.to_jcs_bytes()?;
        sign_cose(&jcs, keypair).map_err(|e| anyhow::anyhow!("Message: COSE sign failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::a2a::part::Part;

    fn sample_message() -> Message {
        Message {
            role: "user".to_string(),
            parts: vec![Part::Text {
                text: "hello agent".to_string(),
            }],
            message_id: "msg-001".to_string(),
            task_id: Some("task-001".to_string()),
            context_id: None,
        }
    }

    #[test]
    fn roundtrip_jcs_stable() {
        let msg = sample_message();
        let b1 = msg.to_jcs_bytes().unwrap();
        let b2 = msg.to_jcs_bytes().unwrap();
        assert_eq!(b1, b2, "Message JCS bytes must be stable");
    }

    #[test]
    fn roundtrip_envelope_signs_and_verifies() {
        use crate::codec::sign::verify_artifact;
        use solana_sdk::signature::{Keypair, Signer as _};

        let kp = Keypair::new();
        let msg = sample_message();
        let envelope = msg.to_canonical_envelope(&kp, None).unwrap();

        let result = verify_artifact(&envelope, None).expect("verify must not error");
        assert!(result.cose_signature, "signature must verify");
        assert!(result.valid);
    }
}

//! Wave 1 mock prover + stub evidence. No real cryptography — the seams that let
//! the correspondence stack run end-to-end before the zigz backend (Wave 2) and
//! zkTLS evidence (Wave 3) land.

use mnemonic_core::correspondence::PolicyCertificate;

/// Authenticated external evidence for an action.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// Commitment the proof's public inputs bind to.
    pub commitment: String,
    /// Reference to the evidence attestation bytes (zkTLS transcript, …).
    pub proof_ref: String,
}

/// Source of authenticated evidence. Wave 3 implements this with zkTLS
/// (TLSNotary) over a merchant/PSP endpoint; Wave 1 uses [`StubEvidence`].
pub trait EvidenceSource {
    fn collect(&self, action_commitment: &str) -> Result<Evidence, String>;
}

/// Trusted stub — derives a deterministic commitment from the action. NOT
/// secure; it is the `EvidenceSource` seam so the stack runs before notary ops.
#[derive(Debug, Default, Clone)]
pub struct StubEvidence;

impl EvidenceSource for StubEvidence {
    fn collect(&self, action_commitment: &str) -> Result<Evidence, String> {
        let commitment = format!(
            "ev:{}",
            &blake3::hash(action_commitment.as_bytes()).to_hex()[..16]
        );
        let proof_ref = format!(
            "evpf:{}",
            &blake3::hash(commitment.as_bytes()).to_hex()[..16]
        );
        Ok(Evidence {
            commitment,
            proof_ref,
        })
    }
}

/// Produces a correspondence certificate from the bound public inputs.
pub trait Prover {
    fn prove(
        &self,
        intent_hash: &str,
        action_commitment: &str,
        evidence: &Evidence,
        policy_id: &str,
    ) -> Result<PolicyCertificate, String>;
}

/// Mock prover — emits a structurally valid certificate whose public inputs bind
/// `(intent_hash, action_commitment, evidence_commitment)`, with a fake proof.
/// The real binding (action_commitment recompute) is still enforced by the core
/// verifier, so this exercises the full envelope path. Swapped for zigz in Wave 2.
#[derive(Debug, Default, Clone)]
pub struct MockProver;

impl Prover for MockProver {
    fn prove(
        &self,
        intent_hash: &str,
        action_commitment: &str,
        evidence: &Evidence,
        policy_id: &str,
    ) -> Result<PolicyCertificate, String> {
        let public_inputs = vec![
            intent_hash.to_string(),
            action_commitment.to_string(),
            evidence.commitment.clone(),
        ];
        let proof_ref = format!(
            "pf:{}",
            &blake3::hash(public_inputs.concat().as_bytes()).to_hex()[..32]
        );
        Ok(PolicyCertificate {
            intent_hash: intent_hash.to_string(),
            action_commitment: action_commitment.to_string(),
            evidence_commitment: evidence.commitment.clone(),
            policy_id: policy_id.to_string(),
            params_hash: "params:mock".to_string(),
            public_inputs,
            proof_kind: "mock".to_string(),
            proof_ref,
            backend: "mock".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnemonic_core::codec::schema::{ACTION_V1, INTENT_V1};
    use mnemonic_core::codec::sign::sign_artifact;
    use mnemonic_core::correspondence::{action_commitment, verify_correspondence, MockVerifier};
    use solana_sdk::signature::{Keypair, Signer};

    #[test]
    fn produced_cert_verifies_end_to_end() {
        let kp = Keypair::new();
        let producer = format!("did:sol:{}", kp.pubkey());

        // Principal signs an Intent.
        let intent = serde_json::json!({
            "artifact_id": "art:intent", "type": "intent", "schema_version": 1,
            "constraints": { "cap": 1000, "policy_id": "payment_mandate_v1" },
            "producer": producer, "created_at": "2026-06-30T00:00:00Z",
        });
        let si = sign_artifact(&intent, &INTENT_V1, &kp).unwrap();

        // Agent builds an Action referencing the intent.
        let action = serde_json::json!({
            "artifact_id": "art:action", "type": "action", "schema_version": 1,
            "content": "{\"amount\":900}", "intent_ref": si.content_hash,
            "producer": producer, "created_at": "2026-06-30T00:01:00Z",
        });
        let ac = action_commitment(&action).unwrap();

        // PRODUCE: stub evidence + mock proof.
        let ev = StubEvidence.collect(&ac).unwrap();
        let cert = MockProver
            .prove(&si.content_hash, &ac, &ev, "payment_mandate_v1")
            .unwrap();

        // Sign the action and VERIFY the produced certificate via core.
        let sa = sign_artifact(&action, &ACTION_V1, &kp).unwrap();
        let v =
            verify_correspondence(&si.cose_bytes, &sa.cose_bytes, &cert, &MockVerifier).unwrap();

        assert!(v.safe(), "produced certificate should verify: {v:?}");
        assert_eq!(v.policy_valid(), Some(true));
    }
}

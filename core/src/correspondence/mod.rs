//! Intent–action correspondence — **verify-only** layer.
//!
//! Proves that an agent's signed ACTION corresponds to a principal's signed
//! INTENT, given authenticated evidence. This module NEVER produces proofs
//! (the "no prover in core/" rule); production lives in the `mnemonic-prover`
//! crate. Here we only:
//!   - compute the `action_commitment` (the circularity-safe content binding),
//!   - re-verify the five independent checks at `verify` time.
//!
//! Pure (codec only) so it runs client-side / wasm and against any backend.
//! See `work/research/computation-proof/tech-spec.md`.

use serde::{Deserialize, Serialize};

use crate::codec::canonical::{from_canonical_cbor, to_canonical_cbor};
use crate::codec::hash::hash_bytes;
use crate::codec::schema::ACTION_COMMIT_V1;
use crate::codec::sign::verify_artifact;

/// A correspondence certificate: binds a proof that `action ⊨ intent` given
/// authenticated evidence. Rides in `action.metadata.correspondence`; the proof
/// bytes `π` live off-envelope (durable store), referenced by `proof_ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyCertificate {
    pub intent_hash: String,
    pub action_commitment: String,
    pub evidence_commitment: String,
    pub policy_id: String,
    pub params_hash: String,
    /// Field elements / hex commitments the proof binds, incl. action_commitment.
    pub public_inputs: Vec<String>,
    /// "zigz" | "snark" | "zktls" | "mock"
    pub proof_kind: String,
    /// blake3 of the proof bytes π.
    pub proof_ref: String,
    pub backend: String,
}

/// Per-check tri-state result. `None` = not present / not checked — mirrors the
/// `chain_valid: Option<bool>` convention.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorrespondenceVerification {
    pub intent_sig: Option<bool>,
    pub action_sig: Option<bool>,
    pub intent_link: Option<bool>,
    pub correspondence_proof: Option<bool>,
    pub evidence_proof: Option<bool>,
}

impl CorrespondenceVerification {
    /// True iff every check is present and passed.
    pub fn safe(&self) -> bool {
        matches!(self.intent_sig, Some(true))
            && matches!(self.action_sig, Some(true))
            && matches!(self.intent_link, Some(true))
            && matches!(self.correspondence_proof, Some(true))
            && matches!(self.evidence_proof, Some(true))
    }

    /// The single `policy_valid` tri-state a `verify` caller surfaces alongside
    /// authorship/integrity: `None` = no cert, `Some(false)` = proof/binding
    /// failed, `Some(true)` = verified.
    pub fn policy_valid(&self) -> Option<bool> {
        self.correspondence_proof
    }
}

/// The swappable proof backend. The structural content-binding
/// (`action_commitment` recompute) is enforced by [`verify_correspondence`]
/// itself; this trait only re-checks the cryptographic proof + evidence. Wave 1
/// ships [`MockVerifier`]; Wave 2 adds a pure-Rust zigz verifier.
pub trait CorrespondenceVerifier {
    /// Re-verify the policy proof π against the certificate's public inputs.
    fn verify_proof(&self, cert: &PolicyCertificate) -> bool;
    /// Re-verify the evidence attestation (zkTLS, …) against evidence_commitment.
    fn verify_evidence(&self, cert: &PolicyCertificate) -> bool;
}

/// Test/dev verifier. Accepts a well-formed certificate; the real cryptographic
/// content-binding is still enforced by [`verify_correspondence`] (the
/// `action_commitment` recompute), so a tampered action fails even here.
#[derive(Debug, Default, Clone)]
pub struct MockVerifier;

impl CorrespondenceVerifier for MockVerifier {
    fn verify_proof(&self, cert: &PolicyCertificate) -> bool {
        !cert.proof_ref.is_empty() && !cert.public_inputs.is_empty()
    }
    fn verify_evidence(&self, cert: &PolicyCertificate) -> bool {
        !cert.evidence_commitment.is_empty()
    }
}

/// `action_commitment` = blake3 over the canonical CBOR of the action's
/// PRE-certificate fields (content, intent_ref, knowledge_refs, created_at,
/// producer). Excludes `metadata` → no fixed-point with the cert it will carry.
/// Stable across a canonical-CBOR round trip (timestamps normalise to epoch), so
/// the prover and the verifier compute the same value. `action` may be the full
/// ACTION_V1 JSON — extra fields are ignored.
pub fn action_commitment(action: &serde_json::Value) -> Result<String, String> {
    let cbor = to_canonical_cbor(action, &ACTION_COMMIT_V1)?;
    Ok(hash_bytes(&cbor))
}

/// Run the five independent correspondence checks — no re-execution, no private
/// witness. Inputs: the signed INTENT and ACTION COSE_Sign1 envelopes, the
/// certificate (parsed from `action.metadata.correspondence`), and a proof
/// backend.
pub fn verify_correspondence(
    intent_cose: &[u8],
    action_cose: &[u8],
    cert: &PolicyCertificate,
    verifier: &dyn CorrespondenceVerifier,
) -> Result<CorrespondenceVerification, String> {
    let mut out = CorrespondenceVerification::default();

    // 1 + 2 — authorship / integrity of both envelopes.
    let iv = verify_artifact(intent_cose, None)?;
    out.intent_sig = Some(iv.cose_signature && iv.content_integrity && iv.algorithm_valid);

    let av = verify_artifact(action_cose, None)?;
    out.action_sig = Some(av.cose_signature && av.content_integrity && av.algorithm_valid);

    // Recover the action JSON from its signed payload.
    let action_json = from_canonical_cbor(&av.payload)?;

    // 3 — intent linkage: action.intent_ref == intent.content_hash, and the cert
    //     commits to that same intent hash.
    let intent_ref = action_json
        .get("intent_ref")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    out.intent_link = Some(intent_ref == iv.content_hash && cert.intent_hash == iv.content_hash);

    // 4 — correspondence proof: recompute action_commitment (the real binding),
    //     require it to match the cert + appear in public inputs, THEN re-verify
    //     the proof via the backend.
    let recomputed = action_commitment(&action_json)?;
    let binding_ok = recomputed == cert.action_commitment
        && cert.public_inputs.iter().any(|pi| pi == &recomputed);
    out.correspondence_proof = Some(binding_ok && verifier.verify_proof(cert));

    // 5 — evidence attestation.
    out.evidence_proof = Some(verifier.verify_evidence(cert));

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::schema::{ACTION_V1, INTENT_V1};
    use crate::codec::sign::sign_artifact;
    use solana_sdk::signature::{Keypair, Signer};

    fn build_signed(kp: &Keypair) -> (Vec<u8>, Vec<u8>, PolicyCertificate, serde_json::Value) {
        let producer = format!("did:sol:{}", kp.pubkey());

        let intent = serde_json::json!({
            "artifact_id": "art:intent-1",
            "type": "intent",
            "schema_version": 1,
            "constraints": { "cap": 1000, "policy_id": "payment_mandate_v1" },
            "producer": producer,
            "created_at": "2026-06-30T00:00:00Z",
        });
        let si = sign_artifact(&intent, &INTENT_V1, kp).unwrap();

        let action = serde_json::json!({
            "artifact_id": "art:action-1",
            "type": "action",
            "schema_version": 1,
            "content": "{\"amount\":900}",
            "intent_ref": si.content_hash,
            "producer": producer,
            "created_at": "2026-06-30T00:01:00Z",
        });
        let ac = action_commitment(&action).unwrap();
        let sa = sign_artifact(&action, &ACTION_V1, kp).unwrap();

        let cert = PolicyCertificate {
            intent_hash: si.content_hash.clone(),
            action_commitment: ac.clone(),
            evidence_commitment: "ev:merchant-receipt".to_string(),
            policy_id: "payment_mandate_v1".to_string(),
            params_hash: "params:1".to_string(),
            public_inputs: vec![
                si.content_hash.clone(),
                ac,
                "ev:merchant-receipt".to_string(),
            ],
            proof_kind: "mock".to_string(),
            proof_ref: "pf:deadbeef".to_string(),
            backend: "mock".to_string(),
        };
        (si.cose_bytes, sa.cose_bytes, cert, action)
    }

    #[test]
    fn end_to_end_all_checks_pass() {
        let kp = Keypair::new();
        let (intent_cose, action_cose, cert, _) = build_signed(&kp);
        let v = verify_correspondence(&intent_cose, &action_cose, &cert, &MockVerifier).unwrap();
        assert_eq!(v.intent_sig, Some(true));
        assert_eq!(v.action_sig, Some(true));
        assert_eq!(v.intent_link, Some(true));
        assert_eq!(v.correspondence_proof, Some(true));
        assert_eq!(v.evidence_proof, Some(true));
        assert!(v.safe());
        assert_eq!(v.policy_valid(), Some(true));
    }

    #[test]
    fn tampered_action_commitment_fails_binding() {
        let kp = Keypair::new();
        let (intent_cose, action_cose, mut cert, _) = build_signed(&kp);
        // An attacker swaps the committed action_commitment — binding must fail
        // even though the mock proof itself is "well-formed".
        cert.action_commitment = "0".repeat(64);
        let v = verify_correspondence(&intent_cose, &action_cose, &cert, &MockVerifier).unwrap();
        assert_eq!(v.correspondence_proof, Some(false));
        assert!(!v.safe());
    }

    #[test]
    fn wrong_intent_ref_fails_link() {
        let kp = Keypair::new();
        let producer = format!("did:sol:{}", kp.pubkey());
        // Action points at a different intent hash than the one supplied.
        let intent = serde_json::json!({
            "artifact_id": "art:intent-2", "type": "intent", "schema_version": 1,
            "constraints": { "cap": 1 }, "producer": producer,
            "created_at": "2026-06-30T00:00:00Z",
        });
        let si = sign_artifact(&intent, &INTENT_V1, &kp).unwrap();
        let action = serde_json::json!({
            "artifact_id": "art:action-2", "type": "action", "schema_version": 1,
            "content": "x", "intent_ref": "f".repeat(64), "producer": producer,
            "created_at": "2026-06-30T00:01:00Z",
        });
        let ac = action_commitment(&action).unwrap();
        let sa = sign_artifact(&action, &ACTION_V1, &kp).unwrap();
        let cert = PolicyCertificate {
            intent_hash: si.content_hash.clone(),
            action_commitment: ac.clone(),
            evidence_commitment: "ev".into(),
            policy_id: "p".into(),
            params_hash: "h".into(),
            public_inputs: vec![ac],
            proof_kind: "mock".into(),
            proof_ref: "pf".into(),
            backend: "mock".into(),
        };
        let v =
            verify_correspondence(&si.cose_bytes, &sa.cose_bytes, &cert, &MockVerifier).unwrap();
        assert_eq!(v.intent_link, Some(false));
        assert!(!v.safe());
    }
}

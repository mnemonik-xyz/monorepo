//! COSE_Sign1 signing and verification for verifiable artifacts.
//!
//! Uses RFC 9052 COSE_Sign1 structure with Ed25519 (COSE alg: -8).
//! The existing Solana Ed25519 keypair is reused — no new key material needed.

use coset::{
    iana, CborSerializable, CoseSign1, CoseSign1Builder, HeaderBuilder,
};
use solana_sdk::signature::{Keypair, Signer};

use super::canonical::to_canonical_cbor;
use super::hash::hash_bytes;
use super::schema::ArtifactSchema;

/// Result of signing an artifact.
#[derive(Debug)]
pub struct SignedArtifact {
    /// COSE_Sign1 serialized bytes.
    pub cose_bytes: Vec<u8>,
    /// blake3 hash of canonical CBOR payload (anchored on-chain).
    pub content_hash: String,
    /// Canonical CBOR payload bytes.
    pub canonical_cbor: Vec<u8>,
}

/// Sign an artifact JSON using COSE_Sign1 with Ed25519 keypair.
///
/// Pipeline: JSON → canonical CBOR → blake3 hash → COSE_Sign1
pub fn sign_artifact(
    artifact: &serde_json::Value,
    schema: &ArtifactSchema,
    keypair: &Keypair,
) -> Result<SignedArtifact, String> {
    let canonical_cbor = to_canonical_cbor(artifact, schema)?;
    let content_hash = hash_bytes(&canonical_cbor);

    // Protected header: algorithm = EdDSA, content_type = application/cbor
    let protected = HeaderBuilder::new()
        .algorithm(iana::Algorithm::EdDSA)
        .content_type("application/cbor".to_string())
        .build();

    // Unprotected header: kid = Solana pubkey base58
    let kid = keypair.pubkey().to_string().into_bytes();
    let unprotected = HeaderBuilder::new()
        .key_id(kid)
        .build();

    // Build unsigned COSE_Sign1 to compute Sig_structure
    let unsigned = CoseSign1Builder::new()
        .protected(protected.clone())
        .unprotected(unprotected.clone())
        .payload(canonical_cbor.clone())
        .build();

    // Compute Sig_structure (the bytes to sign per RFC 9052 §4.4)
    let tbs = unsigned.tbs_data(&[]);

    // Sign with Ed25519
    let signature = keypair.sign_message(&tbs);

    // Rebuild with signature
    let signed = CoseSign1Builder::new()
        .protected(protected)
        .unprotected(unprotected)
        .payload(canonical_cbor.clone())
        .signature(signature.as_ref().to_vec())
        .build();

    let cose_bytes = signed.to_vec()
        .map_err(|e| format!("COSE serialization failed: {e}"))?;

    Ok(SignedArtifact {
        cose_bytes,
        content_hash,
        canonical_cbor,
    })
}

/// Verification result.
#[derive(Debug)]
pub struct VerificationResult {
    pub valid: bool,
    pub content_integrity: bool,
    pub cose_signature: bool,
    pub algorithm_valid: bool,
    pub content_hash: String,
    pub signer: String,
    pub payload: Vec<u8>,
}

/// Verify a COSE_Sign1 signed artifact.
pub fn verify_artifact(
    cose_bytes: &[u8],
    expected_hash: Option<&str>,
) -> Result<VerificationResult, String> {
    let cose_sign1 = CoseSign1::from_slice(cose_bytes)
        .map_err(|e| format!("invalid COSE_Sign1: {e}"))?;

    let payload = cose_sign1.payload.as_ref()
        .ok_or_else(|| "COSE_Sign1 has no payload".to_string())?;

    // Extract signer from kid
    let kid_bytes = &cose_sign1.unprotected.key_id;
    let signer = String::from_utf8(kid_bytes.clone())
        .unwrap_or_else(|_| "<invalid kid>".to_string());

    // Parse pubkey for signature verification
    let pubkey = solana_sdk::pubkey::Pubkey::from_str(&signer)
        .map_err(|e| format!("invalid signer pubkey: {e}"))?;

    // Compute Sig_structure for verification
    let sig_structure = cose_sign1.tbs_data(&[]);

    // Verify Ed25519 signature
    let sig_valid = if cose_sign1.signature.len() == 64 {
        let sig = solana_sdk::signature::Signature::from(
            <[u8; 64]>::try_from(&cose_sign1.signature[..]).unwrap()
        );
        sig.verify(pubkey.as_ref(), &sig_structure)
    } else {
        false
    };

    // Check content hash
    let actual_hash = hash_bytes(payload);
    let hash_valid = expected_hash
        .map(|expected| expected == actual_hash)
        .unwrap_or(true);

    // Check algorithm
    let alg_valid = cose_sign1.protected.header.alg
        == Some(coset::RegisteredLabelWithPrivate::Assigned(iana::Algorithm::EdDSA));

    Ok(VerificationResult {
        valid: sig_valid && hash_valid && alg_valid,
        content_integrity: hash_valid,
        cose_signature: sig_valid,
        algorithm_valid: alg_valid,
        content_hash: actual_hash,
        signer,
        payload: payload.clone(),
    })
}

use std::str::FromStr;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::schema::*;

    fn sample_artifact() -> serde_json::Value {
        serde_json::json!({
            "artifact_id": "art:cose-test",
            "type": "memory",
            "schema_version": 1,
            "content": "COSE signing test content",
            "producer": "did:sol:test",
            "created_at": "2026-04-14T12:00:00Z",
            "tags": ["cose", "test"],
        })
    }

    #[test]
    fn test_sign_produces_cose_bytes() {
        let kp = Keypair::new();
        let signed = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp).unwrap();
        assert!(!signed.cose_bytes.is_empty());
        assert_eq!(signed.content_hash.len(), 64);
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        let kp = Keypair::new();
        let signed = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp).unwrap();
        let result = verify_artifact(&signed.cose_bytes, Some(&signed.content_hash)).unwrap();

        assert!(result.valid);
        assert!(result.content_integrity);
        assert!(result.cose_signature);
        assert!(result.algorithm_valid);
        assert_eq!(result.signer, kp.pubkey().to_string());
    }

    #[test]
    fn test_verify_detects_wrong_hash() {
        let kp = Keypair::new();
        let signed = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp).unwrap();
        let result = verify_artifact(&signed.cose_bytes, Some("wrong_hash")).unwrap();

        assert!(!result.valid);
        assert!(!result.content_integrity);
        assert!(result.cose_signature); // sig is fine, hash mismatch
    }

    #[test]
    fn test_verify_without_expected_hash() {
        let kp = Keypair::new();
        let signed = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp).unwrap();
        let result = verify_artifact(&signed.cose_bytes, None).unwrap();
        assert!(result.valid);
    }

    #[test]
    fn test_content_hash_is_blake3_of_cbor() {
        let kp = Keypair::new();
        let signed = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp).unwrap();
        assert_eq!(signed.content_hash, hash_bytes(&signed.canonical_cbor));
    }

    #[test]
    fn test_different_keypairs_same_hash() {
        let kp1 = Keypair::new();
        let kp2 = Keypair::new();
        let s1 = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp1).unwrap();
        let s2 = sign_artifact(&sample_artifact(), &MEMORY_V1, &kp2).unwrap();
        assert_eq!(s1.content_hash, s2.content_hash);
        assert_ne!(s1.cose_bytes, s2.cose_bytes);
    }

    #[test]
    fn test_all_schemas_sign_verify() {
        let kp = Keypair::new();
        for (schema, name) in [
            (&RAG_CONTEXT_V1, "rag.context"), (&RAG_RESULT_V1, "rag.result"),
            (&AGENT_STATE_V1, "agent.state"), (&RECEIPT_V1, "receipt"),
            (&MEMORY_V1, "memory"),
        ] {
            let artifact = serde_json::json!({
                "artifact_id": format!("art:{name}"), "type": name,
                "schema_version": 1, "content": "test",
                "producer": "p", "created_at": "2026-01-01T00:00:00Z",
            });
            let signed = sign_artifact(&artifact, schema, &kp).expect(name);
            let result = verify_artifact(&signed.cose_bytes, Some(&signed.content_hash)).expect(name);
            assert!(result.valid, "{name} failed");
        }
    }
}

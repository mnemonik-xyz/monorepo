//! Round-trip test for A2aArtifact: attest → recall → verify envelope.

mod common;
use common::InMemoryA2aStore;

use mnemonic_a2a::{attest_artifact, recall_by_context, verify_a2a_attestation, A2aArtifact};
use mnemonic_core::codec::a2a::Part;
use solana_sdk::signature::{Keypair, Signer};

fn sample_artifact() -> A2aArtifact {
    A2aArtifact {
        artifact_id: Some("art-test-001".to_string()),
        name: Some("test-report".to_string()),
        parts: vec![Part::Text {
            text: "artifact payload for testing".to_string(),
        }],
    }
}

#[test]
fn artifact_roundtrip_attest_recall_verify() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let ctx = "ctx-art-rt-001";
    let art = sample_artifact();

    // Attest.
    let id = attest_artifact(&store, &art, ctx, &kp, None).expect("attest_artifact must succeed");
    assert!(!id.is_empty());

    // Recall.
    let rows = recall_by_context(&store, ctx, None).expect("recall_by_context must succeed");
    assert_eq!(rows.len(), 1);

    let row = &rows[0];
    assert_eq!(row.attestation_id, id);
    assert_eq!(row.signer_pubkey, kp.pubkey().to_string());

    // Verify COSE_Sign1.
    let cose_bytes = hex::decode(&row.arweave_tx).expect("arweave_tx must be hex COSE bytes");
    let verified = verify_a2a_attestation(&cose_bytes, Some(&kp.pubkey().to_string()))
        .expect("verify must not error");
    assert!(verified.valid);

    // Payload must contain the artifact fields.
    let payload_json: serde_json::Value = serde_json::from_slice(&verified.payload).unwrap();
    assert_eq!(payload_json["artifactId"], "art-test-001");
    assert_eq!(payload_json["name"], "test-report");
}

#[test]
fn artifact_two_in_same_context() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let ctx = "ctx-art-two";

    let art1 = A2aArtifact {
        artifact_id: Some("art-1".to_string()),
        name: None,
        parts: vec![Part::Text {
            text: "first".to_string(),
        }],
    };
    let art2 = A2aArtifact {
        artifact_id: Some("art-2".to_string()),
        name: None,
        parts: vec![Part::Text {
            text: "second".to_string(),
        }],
    };

    attest_artifact(&store, &art1, ctx, &kp, None).unwrap();
    attest_artifact(&store, &art2, ctx, &kp, None).unwrap();

    let rows = recall_by_context(&store, ctx, None).unwrap();
    assert_eq!(rows.len(), 2, "both artifacts must be recalled");
}

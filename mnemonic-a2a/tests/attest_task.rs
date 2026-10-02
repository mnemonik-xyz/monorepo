//! Round-trip test: build Task → attest → recall by context → verify envelope
//! → assert content matches.

mod common;
use common::InMemoryA2aStore;

use mnemonic_a2a::{attest_task, recall_by_context, verify_a2a_attestation, Task};
use solana_sdk::signature::{Keypair, Signer};

fn sample_task(ctx: &str) -> Task {
    Task {
        id: "task-roundtrip-001".to_string(),
        context_id: ctx.to_string(),
        status: "working".to_string(),
        history: None,
        artifacts: None,
    }
}

#[test]
fn task_roundtrip_attest_recall_verify() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let ctx = "ctx-task-rt-001";
    let task = sample_task(ctx);

    // Attest.
    let id = attest_task(&store, &task, &kp, None).expect("attest_task must succeed");
    assert!(!id.is_empty(), "attestation_id must be non-empty");

    // Recall.
    let rows = recall_by_context(&store, ctx, None).expect("recall_by_context must succeed");
    assert_eq!(rows.len(), 1, "exactly one row expected");

    let row = &rows[0];
    assert_eq!(row.attestation_id, id);
    assert_eq!(row.signer_pubkey, kp.pubkey().to_string());

    // The stored content is the JCS of the task — verify it round-trips.
    let content: serde_json::Value =
        serde_json::from_str(&row.content).expect("stored content must be valid JSON");
    assert_eq!(content["id"], "task-roundtrip-001");
    assert_eq!(content["contextId"], ctx);
    assert_eq!(content["status"], "working");

    // The `arweave_tx` column holds the hex-encoded COSE_Sign1 envelope.
    let cose_bytes = hex::decode(&row.arweave_tx).expect("arweave_tx must be valid hex COSE bytes");
    let verified = verify_a2a_attestation(&cose_bytes, Some(&kp.pubkey().to_string()))
        .expect("verify must not error");
    assert!(verified.valid, "COSE_Sign1 must verify");
    assert_eq!(verified.signer, kp.pubkey().to_string());

    // Payload round-trip: the signed payload equals the JCS bytes.
    let payload_json: serde_json::Value =
        serde_json::from_slice(&verified.payload).expect("payload must be valid JSON");
    assert_eq!(payload_json["id"], "task-roundtrip-001");
}

#[test]
fn attest_task_wrong_pubkey_fails_verification() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let other_kp = Keypair::new();
    let task = sample_task("ctx-wrong-pk");

    let id = attest_task(&store, &task, &kp, None).unwrap();
    let rows = recall_by_context(&store, "ctx-wrong-pk", None).unwrap();
    let row = rows.iter().find(|r| r.attestation_id == id).unwrap();

    let cose_bytes = hex::decode(&row.arweave_tx).unwrap();
    let verified =
        verify_a2a_attestation(&cose_bytes, Some(&other_kp.pubkey().to_string())).unwrap();
    assert!(
        !verified.valid,
        "verification must fail when expected_pubkey does not match signer"
    );
}

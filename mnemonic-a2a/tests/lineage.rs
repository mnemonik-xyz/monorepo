//! Lineage test: three-message conversation where each message chains to the
//! previous via `prev_id`.
//!
//! m1.prev_id = None
//! m2.prev_id = m1.id
//! m3.prev_id = m2.id
//!
//! After attesting all three, `recall_by_context` must return all three rows;
//! we then assert the chain is present by verifying each envelope individually.

mod common;
use common::InMemoryA2aStore;

use mnemonic_a2a::{attest_message, recall_by_context, verify_a2a_attestation, Message};
use mnemonic_core::codec::a2a::Part;
use solana_sdk::signature::{Keypair, Signer};

fn make_message(id: &str, ctx: &str, text: &str) -> Message {
    Message {
        role: "user".to_string(),
        parts: vec![Part::Text {
            text: text.to_string(),
        }],
        message_id: id.to_string(),
        task_id: None,
        context_id: Some(ctx.to_string()),
    }
}

#[test]
fn three_message_chain_validates() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let ctx = "ctx-lineage-001";

    let m1 = make_message("msg-001", ctx, "Hello from m1");
    let id1 = attest_message(&store, &m1, &kp, None).unwrap();

    let m2 = make_message("msg-002", ctx, "Reply from m2");
    let id2 = attest_message(&store, &m2, &kp, Some(&id1)).unwrap();

    let m3 = make_message("msg-003", ctx, "Final from m3");
    let id3 = attest_message(&store, &m3, &kp, Some(&id2)).unwrap();

    // All three must be distinct IDs.
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);

    // All three must be recalled from the same context.
    let rows = recall_by_context(&store, ctx, None).unwrap();
    assert_eq!(rows.len(), 3, "all three messages must be in context");

    // Collect attestation_ids for lookup.
    let ids: Vec<&str> = rows.iter().map(|r| r.attestation_id.as_str()).collect();
    assert!(ids.contains(&id1.as_str()), "id1 must be in rows");
    assert!(ids.contains(&id2.as_str()), "id2 must be in rows");
    assert!(ids.contains(&id3.as_str()), "id3 must be in rows");

    // Each envelope must verify independently.
    for row in &rows {
        let cose_bytes = hex::decode(&row.arweave_tx).unwrap();
        let verified = verify_a2a_attestation(&cose_bytes, None).unwrap();
        assert!(
            verified.valid,
            "envelope for attestation_id={} must verify",
            row.attestation_id
        );
        assert_eq!(verified.signer, kp.pubkey().to_string());
    }
}

#[test]
fn lineage_limit_parameter() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let ctx = "ctx-lineage-limit";

    for i in 0..5 {
        let m = make_message(&format!("msg-{i:03}"), ctx, &format!("message {i}"));
        attest_message(&store, &m, &kp, None).unwrap();
    }

    let all = recall_by_context(&store, ctx, None).unwrap();
    assert_eq!(all.len(), 5);

    let limited = recall_by_context(&store, ctx, Some(3)).unwrap();
    assert_eq!(limited.len(), 3, "limit=3 must return 3 rows");
}

//! End-to-end test: `message/send` → attestation injected → `recall_by_context`
//! returns the row.
//!
//! Uses the in-memory `InMemoryA2aStore` from `common` — no SQLite on disk.

mod common;

use common::InMemoryA2aStore;

use bridge_a2a::attest::attest_message_send;
use bridge_a2a::idem::IdempotencyCache;
use bridge_a2a::lineage::LineageMap;
use bridge_a2a::config::FailureMode;

use mnemonic_a2a::recall_by_context;
use solana_sdk::signature::Keypair;

fn make_message_send_request(
    message_id: &str,
    context_id: Option<&str>,
) -> serde_json::Value {
    let mut msg = serde_json::json!({
        "role": "user",
        "messageId": message_id,
        "parts": [{ "kind": "text", "text": "hello" }]
    });
    if let Some(ctx) = context_id {
        msg["contextId"] = serde_json::Value::String(ctx.to_string());
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": { "message": msg }
    })
}

fn make_message_send_response(task_id: &str, context_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "id": task_id,
            "contextId": context_id,
            "status": "working"
        }
    })
}

#[test]
fn message_send_injects_attestation_id() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let idem = IdempotencyCache::new();
    let lineage = LineageMap::new();
    let ctx = "ctx-e2e-001";

    let req = make_message_send_request("msg-001", Some(ctx));
    let mut resp = make_message_send_response("task-001", ctx);

    attest_message_send(
        &store,
        &kp,
        &idem,
        &lineage,
        &FailureMode::AttestBestEffort,
        &req,
        &mut resp,
    )
    .expect("attestation must succeed");

    // Response must contain x-mnemonic extension.
    let att_id = resp["result"]["extensions"]["x-mnemonic"]["attestation_id"]
        .as_str()
        .expect("attestation_id must be present in response extensions");

    assert!(!att_id.is_empty(), "attestation_id must be non-empty");

    // recall_by_context must return the same attestation.
    let rows = recall_by_context(&store, ctx, None)
        .expect("recall_by_context must not fail");
    assert!(
        !rows.is_empty(),
        "at least one attestation must be stored under contextId"
    );

    // The message attestation row id must be in the stored rows.
    let found = rows.iter().any(|r| r.attestation_id == att_id);
    assert!(found, "attestation_id from response must appear in stored rows");
}

#[test]
fn message_send_blake3_present() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();
    let idem = IdempotencyCache::new();
    let lineage = LineageMap::new();

    let req = make_message_send_request("msg-blake3", None);
    let mut resp = make_message_send_response("task-blake3", "ctx-blake3");

    attest_message_send(
        &store,
        &kp,
        &idem,
        &lineage,
        &FailureMode::AttestBestEffort,
        &req,
        &mut resp,
    )
    .unwrap();

    // blake3 field is present (may be empty string when context_id lookup
    // returns no rows yet, but the key must exist).
    let ext = &resp["result"]["extensions"]["x-mnemonic"];
    assert!(
        ext.get("blake3").is_some(),
        "x-mnemonic.blake3 field must be present"
    );
}

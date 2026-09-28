//! Strict failure mode: when the store write fails, `attest-strict` must cause
//! `attest_message_send` / `attest_tasks_get` to return `Err`.  In
//! `attest-best-effort` mode the same failure must be silently absorbed.

mod common;

use common::InMemoryA2aStore;

use bridge_a2a::attest::{attest_message_send, attest_tasks_get};
use bridge_a2a::config::FailureMode;
use bridge_a2a::idem::IdempotencyCache;
use bridge_a2a::lineage::LineageMap;
use solana_sdk::signature::Keypair;

fn message_send_req(msg_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": msg_id,
                "contextId": "ctx-strict",
                "parts": [{ "kind": "text", "text": "strict test" }]
            }
        }
    })
}

fn message_send_resp() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "id": "task-strict",
            "contextId": "ctx-strict",
            "status": "working"
        }
    })
}

fn tasks_get_req() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tasks/get",
        "params": { "taskId": "task-strict" }
    })
}

fn tasks_get_resp_completed() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": {
            "id": "task-strict",
            "contextId": "ctx-strict",
            "status": "completed"
        }
    })
}

#[test]
fn strict_mode_message_send_fails_on_write_error() {
    let store = InMemoryA2aStore::new();
    store.set_fail_writes(true); // inject failure

    let kp = Keypair::new();
    let idem = IdempotencyCache::new();
    let lineage = LineageMap::new();

    let req = message_send_req("msg-strict-1");
    let mut resp = message_send_resp();

    let result = attest_message_send(
        &store,
        &kp,
        &idem,
        &lineage,
        &FailureMode::AttestStrict,
        &req,
        &mut resp,
    );

    assert!(
        result.is_err(),
        "attest-strict must return Err when storage write fails"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("attest-strict"),
        "error message must mention attest-strict, got: {msg}"
    );
}

#[test]
fn best_effort_mode_message_send_succeeds_on_write_error() {
    let store = InMemoryA2aStore::new();
    store.set_fail_writes(true); // inject failure

    let kp = Keypair::new();
    let idem = IdempotencyCache::new();
    let lineage = LineageMap::new();

    let req = message_send_req("msg-best-effort-1");
    let mut resp = message_send_resp();

    let result = attest_message_send(
        &store,
        &kp,
        &idem,
        &lineage,
        &FailureMode::AttestBestEffort,
        &req,
        &mut resp,
    );

    assert!(
        result.is_ok(),
        "attest-best-effort must not propagate storage write failure"
    );
    // The response must NOT have an x-mnemonic extension (attestation silently
    // skipped).
    assert!(
        resp["result"]["extensions"]["x-mnemonic"].is_null()
            || resp["result"].get("extensions").is_none(),
        "best-effort skipped attestation must not inject x-mnemonic"
    );
}

#[test]
fn strict_mode_tasks_get_fails_on_write_error() {
    let store = InMemoryA2aStore::new();
    store.set_fail_writes(true);

    let kp = Keypair::new();
    let idem = IdempotencyCache::new();
    let lineage = LineageMap::new();

    let req = tasks_get_req();
    let mut resp = tasks_get_resp_completed();

    let result = attest_tasks_get(
        &store,
        &kp,
        &idem,
        &lineage,
        &FailureMode::AttestStrict,
        &req,
        &mut resp,
    );

    assert!(
        result.is_err(),
        "attest-strict tasks/get must return Err when storage write fails"
    );
}

#[test]
fn best_effort_mode_tasks_get_succeeds_on_write_error() {
    let store = InMemoryA2aStore::new();
    store.set_fail_writes(true);

    let kp = Keypair::new();
    let idem = IdempotencyCache::new();
    let lineage = LineageMap::new();

    let req = tasks_get_req();
    let mut resp = tasks_get_resp_completed();

    let result = attest_tasks_get(
        &store,
        &kp,
        &idem,
        &lineage,
        &FailureMode::AttestBestEffort,
        &req,
        &mut resp,
    );

    assert!(
        result.is_ok(),
        "attest-best-effort tasks/get must not propagate storage failure"
    );
}

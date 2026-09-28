//! Integration tests for `mnemonic_attest_a2a` and `mnemonic_recall_a2a`
//! (a2a-bridge Task 5).
//!
//! Tests:
//! 1. `attest_a2a_kind_task_returns_attestation_id` — call `mnemonic_attest_a2a`
//!    with `kind="task"` and a minimal Task payload; response carries
//!    `attestation_id`, `blake3`, `cose_envelope_hex`.
//! 2. `recall_a2a_returns_attested_row` — after attesting, `mnemonic_recall_a2a`
//!    with the same `context_id` returns the row.
//! 3. `attest_a2a_payment_required_without_bearer` — on an `x402` deploy,
//!    `mnemonic_attest_a2a` without auth → 402.
//! 4. `recall_a2a_free_without_bearer` — `mnemonic_recall_a2a` without auth →
//!    200 (read-only, free, allowed by the allowlist).

mod _helpers;

use _helpers::TestServer;
use serde_json::json;

// ── Minimal A2A Task payload ─────────────────────────────────────────────────

fn minimal_task(context_id: &str) -> serde_json::Value {
    // Task wire shape: id (string), contextId (string), status (string).
    // The A2A v1 Task type uses TaskStatus = String (e.g. "submitted").
    json!({
        "id": "task-001",
        "contextId": context_id,
        "status": "submitted"
    })
}

// ── 1. attest kind=task returns attestation_id ────────────────────────────────

#[tokio::test]
async fn attest_a2a_kind_task_returns_attestation_id() {
    let server = TestServer::builder().build();
    let sub = server.server_pubkey();
    let ctx = "ctx-test-001";

    let resp = server
        .call_tool(
            Some(&sub),
            "mnemonic_attest_a2a",
            json!({
                "kind": "task",
                "payload": minimal_task(ctx),
                "context_id": ctx,
            }),
        )
        .await;

    assert_eq!(
        resp.status,
        axum::http::StatusCode::OK,
        "attest_a2a must return 200: {:?}",
        resp.envelope
    );
    assert!(
        resp.envelope["error"].is_null(),
        "attest_a2a must not return a JSON-RPC error: {:?}",
        resp.envelope
    );

    // Unwrap the MCP content envelope.
    let text = resp.envelope["result"]["content"][0]["text"]
        .as_str()
        .expect("result.content[0].text must be a string");
    let result: serde_json::Value =
        serde_json::from_str(text).expect("content text must be JSON");

    assert!(
        result["attestation_id"].is_string(),
        "result must carry attestation_id; got {result:?}"
    );
    assert!(
        result["blake3"].is_string(),
        "result must carry blake3; got {result:?}"
    );
    assert!(
        result["cose_envelope_hex"].is_string(),
        "result must carry cose_envelope_hex; got {result:?}"
    );
    assert!(
        !result["attestation_id"].as_str().unwrap().is_empty(),
        "attestation_id must be non-empty"
    );
}

// ── 2. recall_a2a returns the row attested above ──────────────────────────────

#[tokio::test]
async fn recall_a2a_returns_attested_row() {
    let server = TestServer::builder().build();
    let sub = server.server_pubkey();
    let ctx = "ctx-test-recall-002";

    // First, attest.
    let attest_resp = server
        .call_tool(
            Some(&sub),
            "mnemonic_attest_a2a",
            json!({
                "kind": "task",
                "payload": minimal_task(ctx),
                "context_id": ctx,
            }),
        )
        .await;
    assert_eq!(attest_resp.status, axum::http::StatusCode::OK);
    assert!(attest_resp.envelope["error"].is_null());

    let attest_text = attest_resp.envelope["result"]["content"][0]["text"]
        .as_str()
        .expect("content text");
    let attest_result: serde_json::Value =
        serde_json::from_str(attest_text).expect("content JSON");
    let expected_id = attest_result["attestation_id"]
        .as_str()
        .expect("attestation_id in attest result");

    // Now recall.  recall_a2a is free — pass an auth header to keep it
    // simple (the tool does not require one, but callers may include it).
    let recall_resp = server
        .call_tool(
            Some(&sub),
            "mnemonic_recall_a2a",
            json!({ "context_id": ctx }),
        )
        .await;
    assert_eq!(
        recall_resp.status,
        axum::http::StatusCode::OK,
        "recall_a2a must return 200: {:?}",
        recall_resp.envelope
    );
    assert!(
        recall_resp.envelope["error"].is_null(),
        "recall_a2a must not return error: {:?}",
        recall_resp.envelope
    );

    let recall_text = recall_resp.envelope["result"]["content"][0]["text"]
        .as_str()
        .expect("recall content text");
    let recall_result: serde_json::Value =
        serde_json::from_str(recall_text).expect("recall content JSON");

    let attestations = recall_result["attestations"]
        .as_array()
        .expect("attestations array in recall result");
    assert!(
        !attestations.is_empty(),
        "recall_a2a must return at least one row for context_id={ctx}"
    );

    let ids: Vec<&str> = attestations
        .iter()
        .filter_map(|a| a["attestation_id"].as_str())
        .collect();
    assert!(
        ids.contains(&expected_id),
        "recall_a2a must include the attested id {expected_id}; got {ids:?}"
    );
}

// ── 3. attest_a2a is paid on x402 deploy — 402 without bearer ────────────────

#[tokio::test]
async fn attest_a2a_payment_required_without_bearer() {
    // Build a server with x402 payment mode.
    let server = TestServer::builder().payment_mode("x402").build();
    let ctx = "ctx-test-payment-003";

    // Call WITHOUT auth header (sub = None) — the middleware allowlist does
    // NOT include mnemonic_attest_a2a, so this hits the payment gate on the
    // non-authenticated path.
    let resp = server
        .call_tool(
            None, // no auth header
            "mnemonic_attest_a2a",
            json!({
                "kind": "task",
                "payload": minimal_task(ctx),
                "context_id": ctx,
            }),
        )
        .await;

    // x402 gate fires before or at JWT auth: expect 402 or 401.
    // Both are acceptable — the key requirement is that the tool does NOT
    // succeed (no 200 with a valid attestation_id).
    assert!(
        resp.status == axum::http::StatusCode::PAYMENT_REQUIRED
            || resp.status == axum::http::StatusCode::UNAUTHORIZED,
        "attest_a2a without auth on x402 deploy must return 402 or 401; got {}",
        resp.status
    );
}

// ── 4. recall_a2a is free — 200 without bearer ───────────────────────────────

#[tokio::test]
async fn recall_a2a_free_without_bearer() {
    let server = TestServer::builder().payment_mode("x402").build();

    // Call WITHOUT auth. recall_a2a is read-only and free; it must return 200
    // even on an x402 deploy and even when the context_id has no rows (empty
    // attestations array is the correct shape, not an error).
    let resp = server
        .call_tool(
            None,
            "mnemonic_recall_a2a",
            json!({ "context_id": "ctx-empty-no-rows" }),
        )
        .await;

    assert_eq!(
        resp.status,
        axum::http::StatusCode::OK,
        "recall_a2a must be free (200) even on x402 deploy: {:?}",
        resp.envelope
    );
    assert!(
        resp.envelope["error"].is_null(),
        "recall_a2a must not return JSON-RPC error: {:?}",
        resp.envelope
    );
}

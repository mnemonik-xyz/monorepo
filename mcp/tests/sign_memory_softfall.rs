//! Task 5 — agent-native-distribution soft-fall routing tests.
//!
//! Decision 4: when `allow_fallback_to_participate=true` AND the local path
//! returns one of the soft-fallable typed errors (`-32098 EmbedderInvalid`,
//! `-32099 LocalStorageBusy`, `-32094 IdentityBootstrapFailed`),
//! `sign_memory` re-dispatches the same arguments through the participate
//! HTTPS proxy at `state.hosted_endpoint`. On success the response carries
//! `escalated: { from, to, reason }`. On hosted unavailability the typed
//! error becomes `-32011 HostedUnavailable` — NOT the original local code.
//!
//! Decision 4 + 5b interaction: post-escalation visibility re-resolves on
//! the hosted side, so `visibility=public` without `public_write_confirmation`
//! is rejected with `-32095 PublicWriteRequiresConfirmation` and the
//! escalation is undone (no chain write).
//!
//! The tests inject a `FailingEmbedder` via the test-only
//! `mock_state_with_embedder_and_endpoint` constructor and use `httpmock`
//! for the hosted side. The mock's `received_requests().len()` assertion
//! proves the escalation is actually proxied (not stubbed inside
//! `sign_memory`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mnemonic_core::embed::Embedder;
use mnemonic_core::storage::Visibility;
use mnemonic_mcp::test_support::mock_state_with_embedder_and_endpoint;
use mnemonic_mcp::tools::{resolve_write_mode, sign_memory, ToolError};

// ── FailingEmbedder ────────────────────────────────────────────────────────

/// Same shape as the `error_catalogue.rs::FailingEmbedder` fixture — `embed()`
/// returns `Vec::new()` on the configured call, which `sign_memory_inline`
/// detects and surfaces as `-32098 EmbedderInvalid`. Kept inline in this
/// file (instead of imported) to avoid a cross-test private import; the
/// shape is small and the duplication keeps each integration test
/// self-contained.
struct FailingEmbedder {
    fail_on_call: AtomicUsize,
    counter: AtomicUsize,
}

impl FailingEmbedder {
    fn always_fail() -> Self {
        Self {
            fail_on_call: AtomicUsize::new(1),
            counter: AtomicUsize::new(0),
        }
    }
}

impl Embedder for FailingEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        if n + 1 >= self.fail_on_call.load(Ordering::SeqCst) {
            Vec::new()
        } else {
            vec![0.0; 8]
        }
    }
    fn dim(&self) -> usize {
        8
    }
    fn provider_name(&self) -> &str {
        "failing"
    }
    fn model_id(&self) -> &str {
        "test/failing-embedder"
    }
}

// ── Shared fixture builders ────────────────────────────────────────────────

fn cost_hint() -> mnemonic_mcp::pricing::CostHint {
    mnemonic_mcp::pricing::CostHint {
        irys_lamports: 0,
        sol_tx_fee_lamports: 0,
        sol_price_usdc: 0.0,
        charge_micro_usdc: 0,
    }
}

fn hosted_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // 2s is well under the test runner's per-test budget; the
        // no-network HostedUnavailable test relies on this.
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}

// ── Test 1: default — no escalation when opt-in is absent ─────────────────

#[tokio::test]
async fn default_no_silent_escalation() {
    // `allow_fallback_to_participate=false` (the default). Local path
    // fails with `-32098 EmbedderInvalid`; the response MUST be the
    // typed local error, never a silent escalation. The hosted_endpoint
    // is intentionally pointed at an unreachable URL so any accidental
    // proxy call would crash the test with a connection error rather
    // than a silent success.
    let state = mock_state_with_embedder_and_endpoint(
        Box::new(FailingEmbedder::always_fail()),
        "http://unreachable.invalid".to_string(),
    );
    let kp = solana_sdk::signature::Keypair::new();
    let owner = mnemonic_core::identity::pubkey_base58(&kp);
    let resolved = resolve_write_mode(None, "local").unwrap();
    let args = serde_json::json!({"content": "no-silent-escalation"});

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "no-silent-escalation",
        &[],
        &cost_hint(),
        "local",
        &owner,
        None,
        resolved,
        Visibility::Private,
        &state.envelope,
        Duration::from_secs(2),
        // allow_fallback = false → router MUST NOT escalate, even with a
        // hosted endpoint configured.
        false,
        &state.hosted_endpoint,
        &state.hosted_client,
        &args,
    )
    .await;

    let err = match result {
        Err(ToolError::TypedRpc(e)) => e,
        other => panic!("expected -32098 EmbedderInvalid (typed), got {other:?}"),
    };
    assert_eq!(err.code, -32098, "expected EmbedderInvalid, got {err:?}");
    let data = err.data.expect("data carrier");
    assert_eq!(data["kind"], "EmbedderInvalid");
}

// ── Test 2: opt-in escalation success ─────────────────────────────────────

#[tokio::test]
async fn opt_in_escalation_returns_escalated_field() {
    // The httpmock server simulates `mcp.mnemonik.xyz/mcp` returning a
    // successful sign_memory result. Decision 4: the local-side response
    // unwraps the hosted `result.content[0].text` JSON and injects
    // `escalated: {from, to, reason}` on top.
    let mock = httpmock::MockServer::start();
    let hosted_result = serde_json::json!({
        "attestation_id": "hosted-uuid",
        "content_hash": "abc123",
        "hash_algorithm": "blake3",
        "encoding": "cbor+cose",
        "solana_tx": "hosted-sol",
        "arweave_tx": "hosted-ar",
        "signer": "hosted-signer",
        "did_sol": "did:sol:hosted",
        "timestamp": "2026-06-04T00:00:00Z",
        "storage_mode": "full",
        "write_mode": "participate",
        "visibility": "private",
    });
    let hosted_text = hosted_result.to_string();
    let m = mock.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/mcp");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [{"type": "text", "text": hosted_text}],
                },
            }));
    });

    let endpoint = format!("{}/mcp", mock.base_url());
    let state = mock_state_with_embedder_and_endpoint(
        Box::new(FailingEmbedder::always_fail()),
        endpoint.clone(),
    );
    let kp = solana_sdk::signature::Keypair::new();
    let owner = mnemonic_core::identity::pubkey_base58(&kp);
    let resolved = resolve_write_mode(None, "local").unwrap();
    // visibility=public would require a public_write_confirmation;
    // that interaction is covered in the next test. Here we use the
    // default Private path so the escalation success surface is the
    // clean target.
    let args = serde_json::json!({"content": "trigger-escalation"});

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "trigger-escalation",
        &[],
        &cost_hint(),
        "local",
        &owner,
        None,
        resolved,
        Visibility::Private,
        &state.envelope,
        Duration::from_secs(2),
        true,
        &state.hosted_endpoint,
        &hosted_client(),
        &args,
    )
    .await
    .expect("escalation should succeed");

    // The escalated marker MUST be present per Decision 4.
    let escalated = result
        .get("escalated")
        .expect("escalated field present on success");
    assert_eq!(escalated["from"], "local");
    assert_eq!(escalated["to"], "participate");
    assert_eq!(
        escalated["reason"], "embedder_unavailable",
        "FailingEmbedder triggers EmbedderInvalid → embedder_unavailable"
    );
    // Hosted-result fields flow through verbatim.
    assert_eq!(result["attestation_id"], "hosted-uuid");
    assert_eq!(result["write_mode"], "participate");

    // The mock saw exactly one POST — proves the escalation actually
    // proxied through the network, not stubbed inside `sign_memory`.
    assert_eq!(m.calls(), 1, "exactly one outbound POST to the hosted mock");
}

// ── Test 3: opt-in escalation but visibility=public without confirmation ──

#[tokio::test]
async fn opt_in_escalation_no_confirmation_token() {
    // Decision 4 + 5b: when escalating to participate with
    // `visibility=public`, the hosted side's public-write gate fires. We
    // simulate the hosted's `-32095 PublicWriteRequiresConfirmation`
    // response and assert the local-side dispatcher propagates it
    // verbatim (the escalation is undone — no chain write).
    let mock = httpmock::MockServer::start();
    let m = mock.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/mcp");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32095,
                    "message": "Public-write confirmation required",
                    "data": {
                        "kind": "PublicWriteRequiresConfirmation",
                        "content_hash": "deadbeef",
                        "suggested_action": "call request_public_write_confirmation",
                    },
                },
            }));
    });

    let endpoint = format!("{}/mcp", mock.base_url());
    let state = mock_state_with_embedder_and_endpoint(
        Box::new(FailingEmbedder::always_fail()),
        endpoint.clone(),
    );
    let kp = solana_sdk::signature::Keypair::new();
    let owner = mnemonic_core::identity::pubkey_base58(&kp);
    // Resolve as a participate request from a participate-supporting
    // envelope so visibility=public is permitted on the local request
    // boundary. The dispatcher rejects local + public via
    // `resolve_visibility` at the boundary; for the soft-fall test we
    // bypass the dispatcher and call `sign_memory` directly with a
    // pre-resolved local mode AND a `Visibility::Public` value the
    // dispatcher would have rejected. That intentionally mirrors what
    // a malicious / buggy client could attempt — the gate STILL fires
    // at the hosted side post-escalation. Decision 4 + 5b.
    //
    // In production the dispatcher would have routed this differently;
    // the contract under test is specifically the hosted-side re-resolve.
    let resolved = resolve_write_mode(None, "local").unwrap();
    let args = serde_json::json!({"content": "pub-without-token", "visibility": "public"});

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "pub-without-token",
        &[],
        &cost_hint(),
        "local",
        &owner,
        None,
        resolved,
        Visibility::Public,
        &state.envelope,
        Duration::from_secs(2),
        true,
        &state.hosted_endpoint,
        &hosted_client(),
        &args,
    )
    .await;

    let err = match result {
        Err(ToolError::TypedRpc(e)) => e,
        other => panic!("expected -32095 PublicWriteRequiresConfirmation, got {other:?}"),
    };
    assert_eq!(
        err.code, -32095,
        "post-escalation visibility re-resolution must fire"
    );
    let data = err.data.expect("data carrier");
    assert_eq!(data["kind"], "PublicWriteRequiresConfirmation");
    assert_eq!(m.calls(), 1, "hosted-side gate is the source of truth");
}

// ── Test 4: opt-in escalation with no network → HostedUnavailable ─────────

#[tokio::test]
async fn opt_in_escalation_no_network() {
    // Decision 4: when the hosted endpoint is unreachable during
    // escalation, the typed error is `-32011 HostedUnavailable`, NOT the
    // original `-32098 EmbedderInvalid`. The agent sees the actual
    // failure point.
    //
    // We use a port-0 binding trick: bind a TcpListener to port 0,
    // capture the port, drop the listener. The OS will not immediately
    // re-issue that port, so a `POST` to it returns ECONNREFUSED almost
    // instantly — keeps the test fast (sub-second) and deterministic.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port-0 bind");
        let p = listener.local_addr().expect("local_addr").port();
        drop(listener);
        p
    };
    let endpoint = format!("http://127.0.0.1:{port}/mcp");

    let state = mock_state_with_embedder_and_endpoint(
        Box::new(FailingEmbedder::always_fail()),
        endpoint.clone(),
    );
    let kp = solana_sdk::signature::Keypair::new();
    let owner = mnemonic_core::identity::pubkey_base58(&kp);
    let resolved = resolve_write_mode(None, "local").unwrap();
    let args = serde_json::json!({"content": "no-network-escalation"});

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "no-network-escalation",
        &[],
        &cost_hint(),
        "local",
        &owner,
        None,
        resolved,
        Visibility::Private,
        &state.envelope,
        Duration::from_secs(2),
        true,
        &state.hosted_endpoint,
        &hosted_client(),
        &args,
    )
    .await;

    let err = match result {
        Err(ToolError::TypedRpc(e)) => e,
        other => panic!("expected -32011 HostedUnavailable, got {other:?}"),
    };
    assert_eq!(
        err.code, -32011,
        "no-network escalation MUST surface HostedUnavailable, not the original local code"
    );
    let data = err.data.expect("data carrier");
    assert_eq!(data["kind"], "HostedUnavailable");
    assert!(data["last_error"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(data["retry_after_ms"], 500);
}

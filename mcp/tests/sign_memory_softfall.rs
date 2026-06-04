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

use std::time::Duration;

use mnemonic_core::storage::Visibility;
use mnemonic_mcp::test_support::mock_state_with_embedder_and_endpoint;
use mnemonic_mcp::tools::{resolve_write_mode, sign_memory, ToolError};

mod support;
use support::FailingEmbedder;

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
    // Decision 4 + 5b — post-escalation visibility re-resolution fires
    // LOCALLY before the hosted proxy call, so a buggy/compromised hosted
    // operator cannot bypass the user-approval ceremony by returning
    // success on a missing-token request. Test-reviewer F-3 (round 1):
    // the mock is configured to return SUCCESS; if the local gate is the
    // source of truth, the agent still sees `-32095` AND the hosted mock
    // observes ZERO calls.
    let mock = httpmock::MockServer::start();
    let hosted_success = serde_json::json!({
        "attestation_id": "hosted-success-that-should-not-be-trusted",
        "write_mode": "participate",
        "visibility": "public",
    });
    let m = mock.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/mcp");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [{"type": "text", "text": hosted_success.to_string()}],
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
    // Direct sign_memory call with Visibility::Public bypasses the
    // dispatcher's local-mode rejection — the test deliberately mirrors
    // an internal caller that has pre-resolved visibility (or a future
    // refactor that admits public on the local path). The contract under
    // test is the LOCAL post-escalation gate: it fires before the proxy
    // network call, so the hosted mock must observe zero hits.
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
        "post-escalation visibility re-resolution must fire LOCALLY before any proxy call"
    );
    let data = err.data.expect("data carrier");
    assert_eq!(data["kind"], "PublicWriteRequiresConfirmation");
    // F-3 contract: the LOCAL gate is the source of truth. Even though the
    // mock is configured to return SUCCESS, the agent sees -32095 because
    // the local code aborts the proxy before reaching the network — a
    // buggy / compromised hosted operator MUST NOT be able to bypass the
    // user-approval ceremony.
    assert_eq!(
        m.calls(),
        0,
        "LOCAL gate must short-circuit before the proxy fires"
    );
}

// ── Test 3b: opt-in escalation with valid token reaches the proxy ─────────
//
// Negative-of-the-negative: when the caller DOES supply a non-empty
// `public_write_confirmation` + `jti` pair, the local pre-flight gate
// passes and the proxy fires. Pins the other side of the F-3 contract:
// the local gate does NOT over-fire on well-formed public-write requests.

#[tokio::test]
async fn opt_in_escalation_with_valid_confirmation_token_reaches_hosted() {
    let mock = httpmock::MockServer::start();
    let hosted_success = serde_json::json!({
        "attestation_id": "hosted-pub-uuid",
        "write_mode": "participate",
        "visibility": "public",
    });
    let m = mock.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/mcp");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [{"type": "text", "text": hosted_success.to_string()}],
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
    let args = serde_json::json!({
        "content": "pub-with-token",
        "visibility": "public",
        // The LOCAL pre-flight gate only checks presence of the token
        // and jti; cryptographic validation is the hosted side's ledger's
        // job (separate test surface). A non-empty pair is sufficient.
        "public_write_confirmation": "fake-but-nonempty-token",
        "jti": "00000000-0000-0000-0000-000000000001",
    });

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "pub-with-token",
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
    .await
    .expect("local gate passes when token+jti are present; proxy returns hosted success");

    let escalated = result
        .get("escalated")
        .expect("escalated field present on success");
    assert_eq!(escalated["reason"], "embedder_unavailable");
    assert_eq!(result["attestation_id"], "hosted-pub-uuid");
    assert_eq!(
        m.calls(),
        1,
        "well-formed public-write request must reach the hosted proxy"
    );
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

// ── Test 5: empty hosted_endpoint sentinel — no soft-fall available ───────
//
// Test-reviewer round-2 F-5: lock the documented contract that
// `hosted_endpoint == ""` disables soft-fall entirely, so `allow_fallback
// == true` with no configured endpoint propagates the original local error
// (`-32098 EmbedderInvalid`) instead of `-32011 HostedUnavailable`. The
// `mock_state` family uses this sentinel — without explicit coverage, a
// future refactor that started treating empty-endpoint as "try the
// default" would silently change which typed error the agent receives.
#[tokio::test]
async fn empty_endpoint_sentinel_propagates_local_error() {
    let state = mock_state_with_embedder_and_endpoint(
        Box::new(FailingEmbedder::always_fail()),
        String::new(),
    );
    let kp = solana_sdk::signature::Keypair::new();
    let owner = mnemonic_core::identity::pubkey_base58(&kp);
    let resolved = resolve_write_mode(None, "local").unwrap();
    let args = serde_json::json!({"content": "empty-endpoint-sentinel"});

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "empty-endpoint-sentinel",
        &[],
        &cost_hint(),
        "local",
        &owner,
        None,
        resolved,
        Visibility::Private,
        &state.envelope,
        Duration::from_secs(2),
        // allow_fallback=true would normally trigger soft-fall, BUT the
        // empty hosted_endpoint sentinel disables the router entirely.
        true,
        &state.hosted_endpoint,
        &hosted_client(),
        &args,
    )
    .await;

    let err = match result {
        Err(ToolError::TypedRpc(e)) => e,
        other => panic!("expected -32098 EmbedderInvalid (original local error), got {other:?}"),
    };
    assert_eq!(
        err.code, -32098,
        "empty hosted_endpoint sentinel must propagate the original local error, NOT HostedUnavailable"
    );
    let data = err.data.expect("data carrier");
    assert_eq!(data["kind"], "EmbedderInvalid");
}

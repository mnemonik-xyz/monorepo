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

// `softfall_env_guard` returns a `std::sync::MutexGuard` that intentionally
// outlives the entire async test body — including the `.await` calls on the
// soft-fall router — so the `MNEMONIC_CONFIG_DIR` env-var mutation in
// `expired_cached_token_surfaces_token_expired_typed_error` cannot leak into
// parallel tests reading the same env var via `read_token()`. Clippy flags
// std-Mutex-across-await as a potential deadlock risk, but here:
//   1. there is only ONE Mutex (the static GUARD inside the helper), and
//   2. each test acquires it exactly once at the top, so no two tests can
//      contend simultaneously to deadlock.
// An async-aware mutex (`tokio::sync::Mutex`) would force every helper
// caller to make the test async-yield-able just for the lock; the value of
// keeping the simple synchronous fixture outweighs the cosmetic warning.
#![allow(clippy::await_holding_lock)]

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
    // SAR5-INFO3: every soft-fall test acquires `softfall_env_guard()`
    // before touching `read_token()` so the expired-token fixture's
    // MNEMONIC_CONFIG_DIR override does not leak across tests.
    let _env_guard = softfall_env_guard();
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
    let _env_guard = softfall_env_guard();
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
    let _env_guard = softfall_env_guard();
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
    let _env_guard = softfall_env_guard();
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
    let _env_guard = softfall_env_guard();
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
    let last_error = data["last_error"].as_str().expect("last_error string");
    assert!(!last_error.is_empty(), "last_error must be non-empty");
    // SAR5-L1 (round-1 security audit): the last_error message MUST NOT
    // include the full URL. The scrubbed form is "<kind> error to host
    // <hostname>"; verify the path segment is absent and the loopback
    // host is the only address present.
    assert!(
        !last_error.contains("/mcp"),
        "scrubbed last_error must not contain the URL path: {last_error:?}"
    );
    assert!(
        !last_error.contains("http://"),
        "scrubbed last_error must not contain the URL scheme: {last_error:?}"
    );
    assert_eq!(data["retry_after_ms"], 500);
}

// ── Test 4a: hosted returns malformed response → HostedUnavailable ────────
//
// R1-004 (code-reviewer round 1): if the hosted side returns success-shaped
// JSON-RPC but the inner `result.content[0].text` is missing or not a JSON
// object, the local proxy can no longer inject `escalated{}` cleanly. The
// guard maps this to `-32011 HostedUnavailable` so the agent sees a typed
// error rather than `Ok(Null)`.
#[tokio::test]
async fn opt_in_escalation_hosted_malformed_response_surfaces_hosted_unavailable() {
    let _env_guard = softfall_env_guard();
    let mock = httpmock::MockServer::start();
    let m = mock.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/mcp");
        // No `content` array, no `text` field — just a bare `result` value.
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": "totally-not-an-object",
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
    let args = serde_json::json!({"content": "hosted-malformed"});

    let result = sign_memory(
        &kp,
        &state.solana,
        &state.arweave,
        &state.store,
        state.embedder.as_ref(),
        &state.compressor,
        &state.pending,
        "hosted-malformed",
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
        other => {
            panic!("expected -32011 HostedUnavailable on malformed hosted response, got {other:?}")
        }
    };
    assert_eq!(err.code, -32011);
    let data = err.data.expect("data carrier");
    assert_eq!(data["kind"], "HostedUnavailable");
    let last_error = data["last_error"].as_str().expect("last_error string");
    assert!(
        last_error.contains("malformed hosted response"),
        "last_error must name the malformed-response failure mode: {last_error:?}"
    );
    assert_eq!(m.calls(), 1);
}

// ── Test 4b: expired cached token surfaces -32099 TokenExpired ────────────
//
// SAR5-INFO3 (round-1 security audit, closure of Task 6 forward flag):
// the soft-fall proxy reads the cached JWT via `read_token()` BEFORE
// sending the HTTP request. If the cached token is expired, the agent
// must see the canonical `-32099 TokenExpired` typed error from the
// catalogue rather than the hosted side's `-32001 unauthorized` —
// otherwise an agent programmed against AC16 mistakes the condition for
// a generic auth failure. Mirrors the existing
// `oauth_loopback.rs::with_config_dir_override` ENV-mutation pattern.

/// Process-wide mutex that EVERY test in this file acquires before
/// touching the soft-fall proxy. The proxy reads `MNEMONIC_CONFIG_DIR`
/// via global `std::env`, so even tests that don't mutate the env var
/// must serialise with the one that does — otherwise the expired-token
/// test pollutes the env state for parallel tests running at the same
/// time. Cargo runs integration tests in parallel by default; this
/// mutex is the workaround that lets us keep that default without
/// flaky cross-test interference.
fn softfall_env_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GUARD.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_config_dir_override<F: FnOnce(&std::path::Path) -> R, R>(f: F) -> R {
    let _g = softfall_env_guard();
    let dir = tempfile::TempDir::new().unwrap();
    let previous = std::env::var_os("MNEMONIC_CONFIG_DIR");
    // SAFETY: ENV_GUARD serialises mutations; previous value is restored
    // below regardless of whether `f` returns or panics. The lock guard
    // outlives the inner call, so no other test can race on the env var.
    unsafe {
        std::env::set_var("MNEMONIC_CONFIG_DIR", dir.path());
    }
    let result = f(dir.path());
    unsafe {
        match previous {
            Some(v) => std::env::set_var("MNEMONIC_CONFIG_DIR", v),
            None => std::env::remove_var("MNEMONIC_CONFIG_DIR"),
        }
    }
    result
}

#[test]
fn expired_cached_token_surfaces_token_expired_typed_error() {
    with_config_dir_override(|cfg_dir| {
        // Seed an expired token (expires_at in the past) at the path the
        // production `token_path()` resolves to under the env override.
        let token_path = cfg_dir.join("token.json");
        let expired = serde_json::json!({
            "jwt": "expired.jwt.value",
            "expires_at": "2000-01-01T00:00:00Z",
            "sub": "expired-test-sub",
        });
        std::fs::write(&token_path, expired.to_string()).expect("seed expired token");

        // Build a fresh tokio runtime inside the env-guard scope — see the
        // `oauth_loopback.rs::fresh_install_path` rationale: a `#[tokio::test]`
        // attribute would build the runtime BEFORE the guard takes effect,
        // racing the env mutation.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let state = mock_state_with_embedder_and_endpoint(
                Box::new(FailingEmbedder::always_fail()),
                // Endpoint set to a syntactically-safe loopback so the URL
                // validation gate (SAR5-M1) accepts it — the test SHOULD
                // short-circuit at the expired-token branch BEFORE any
                // HTTP call is attempted, so the endpoint's reachability
                // is irrelevant.
                "http://127.0.0.1:1/mcp".to_string(),
            );
            let kp = solana_sdk::signature::Keypair::new();
            let owner = mnemonic_core::identity::pubkey_base58(&kp);
            let resolved = resolve_write_mode(None, "local").unwrap();
            let args = serde_json::json!({"content": "expired-token-test"});

            let result = sign_memory(
                &kp,
                &state.solana,
                &state.arweave,
                &state.store,
                state.embedder.as_ref(),
                &state.compressor,
                &state.pending,
                "expired-token-test",
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
                other => panic!(
                    "expected -32099 TokenExpired (typed) from soft-fall proxy, got {other:?}"
                ),
            };
            assert_eq!(
                err.code, -32099,
                "expired cached token must surface canonical -32099 TokenExpired, NOT hosted -32001"
            );
            let data = err.data.expect("data carrier");
            assert_eq!(data["kind"], "TokenExpired");
            assert_eq!(data["expires_at"], "2000-01-01T00:00:00Z");
            assert_eq!(data["pubkey"], "expired-test-sub");
        });
    });
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
    let _env_guard = softfall_env_guard();
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

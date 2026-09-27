//! Integration tests for the free daily anchor quota.
//!
//! Each agent key gets N free `participate` (on-chain anchored) writes per
//! UTC day on a `PAYMENT_MODE=x402` deploy, under a global daily cap. The
//! pre-parking x402 gate in `mcp_handler` only peeks; the sign-callback
//! consumes one free anchor at anchor time and refunds it when delivery is
//! not confirmed.
//!
//! Successful anchors use `MNEMONIC_DEFERRED_SYNTHETIC_ANCHOR=1` (synthetic
//! Arweave/Solana ids, no live chain). The variable is process-wide, so every
//! test takes `env_lock()` and sets or clears it for its own run.
//!
//! Scenarios:
//! 1. Ten free anchors for one key; the 11th gets 402 with `free_anchors`.
//!    A second key still has its own ten. `mnemonic_whoami` shows the counts.
//! 2. Global cap: with global = 3, the 4th anchor across keys gets 402.
//! 3. Single consumption at anchor time: two bundles parked on one free
//!    anchor; the second callback gets 402 and consumes nothing.
//! 4. `PAYMENT_MODE=none` consumes nothing and shows no `free_anchors`.
//! 5. A caller that sends `X-Payment` keeps the paid path.
//! 6. Delivery demotion refunds the free anchor.
//! 7. Universal Paywall: a free anchor skips the wallet link and the charge;
//!    with no free anchor left, the charge path starts (428 wallet link).
//!
//! Counter atomicity under concurrency and the UTC day rollover are unit
//! tests in `mcp/src/payment.rs`.

mod _helpers;
#[path = "_helpers/delivery_harness.rs"]
mod delivery_harness;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use _helpers::{TestServer, TEST_JWT_SECRET};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::post,
    Router,
};
use delivery_harness::{MockArweave, MockSolana};
use http_body_util::BodyExt;
use mnemonic_core::codec::sign::sign_cose;
use mnemonic_mcp::{
    api::sign_callback_handler,
    mcp::{mcp_handler, McpState},
    oauth::{self, OAuthState},
    payment,
};
use serde_json::{json, Value};
use solana_sdk::signature::{Keypair, Signer};
use tower::ServiceExt;

const COST: i64 = 1_000;

/// Serialises the tests in this binary around the process-wide
/// `MNEMONIC_DEFERRED_SYNTHETIC_ANCHOR` variable.
async fn env_lock(synthetic_anchor: bool) -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let guard = LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    if synthetic_anchor {
        std::env::set_var("MNEMONIC_DEFERRED_SYNTHETIC_ANCHOR", "1");
    } else {
        std::env::remove_var("MNEMONIC_DEFERRED_SYNTHETIC_ANCHOR");
    }
    guard
}

fn up_config() -> mnemonic_mcp::universal_paywall::UniversalPaywallConfig {
    mnemonic_mcp::universal_paywall::UniversalPaywallConfig {
        url: "http://localhost:0".into(),
        api_key: "test-key".into(),
        network: "eip155:84532".into(),
        asset: "0x0000000000000000000000000000000000000001".into(),
        pay_to: "0x0000000000000000000000000000000000000002".into(),
        payer_wallet: String::new(),
        approval_url_base: String::new(),
    }
}

fn paid_server(per_key: u32, global: u32) -> TestServer {
    TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(COST)
        .free_anchors(per_key, global)
        .build()
}

/// POST a JSON body to `path` on `app`; return status + parsed JSON.
async fn post_json(
    app: &Router,
    path: &str,
    headers: &[(&str, String)],
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, value);
    }
    let req = builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, json)
}

/// `tools/call mnemonic_sign_memory` (participate) as `kp`. Returns the HTTP
/// status and the raw response envelope.
async fn sign_memory(
    app: &Router,
    kp: &Keypair,
    content: &str,
    extra: &[(&str, String)],
) -> (StatusCode, Value) {
    let jwt = mnemonic_mcp::test_support::mint_jwt(&kp.pubkey().to_string(), TEST_JWT_SECRET);
    let mut headers = vec![("authorization", format!("Bearer {jwt}"))];
    headers.extend(extra.iter().cloned());
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "mnemonic_sign_memory",
            "arguments": {"content": content, "mode": "participate"},
        },
    });
    post_json(app, "/mcp", &headers, body).await
}

/// The tool result JSON inside a successful JSON-RPC envelope.
fn tool_result(envelope: &Value) -> Value {
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no tool result text: {envelope}"));
    serde_json::from_str(text).expect("tool result is JSON")
}

/// Sign the parked bundle in `parked` (an `awaiting_signature` result) with
/// `kp` and POST it to `/api/sign-callback`.
async fn submit(app: &Router, kp: &Keypair, parked: &Value) -> (StatusCode, Value) {
    assert_eq!(parked["status"], "awaiting_signature", "{parked}");
    let cbor = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        parked["canonical_cbor_b64"]
            .as_str()
            .expect("canonical_cbor_b64"),
    )
    .expect("base64");
    let cose = sign_cose(&cbor, kp).expect("sign");
    let body = json!({
        "correlation_id": parked["correlation_id"],
        "cose_signed_bytes": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, cose),
        "signer_pubkey": kp.pubkey().to_string(),
    });
    post_json(app, "/api/sign-callback", &[], body).await
}

/// Park, sign and anchor one participate write; assert it anchored.
async fn anchor_ok(app: &Router, kp: &Keypair, content: &str) {
    let (status, envelope) = sign_memory(app, kp, content, &[]).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    let (status, body) = submit(app, kp, &tool_result(&envelope)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
}

/// `mnemonic_whoami` as `kp`; returns the tool result JSON.
async fn whoami(app: &Router, kp: &Keypair) -> Value {
    let jwt = mnemonic_mcp::test_support::mint_jwt(&kp.pubkey().to_string(), TEST_JWT_SECRET);
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "mnemonic_whoami", "arguments": {}},
    });
    let (status, envelope) = post_json(
        app,
        "/mcp",
        &[("authorization", format!("Bearer {jwt}"))],
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    tool_result(&envelope)
}

fn remaining_today(state: &McpState, kp: &Keypair) -> (u32, u32) {
    let store = state.store.lock().unwrap();
    payment::free_anchors_remaining(
        store.conn(),
        &kp.pubkey().to_string(),
        &payment::utc_day(chrono::Utc::now()),
        state.free_anchors,
    )
    .unwrap()
}

// ── 1. Ten free anchors per key, then payment ───────────────────────────────

#[tokio::test]
async fn ten_free_anchors_per_key_then_the_eleventh_requires_payment() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let alice = Keypair::new();

    for i in 0..10 {
        anchor_ok(&server.app, &alice, &format!("alice free anchor {i}")).await;
    }
    assert_eq!(remaining_today(&server.state, &alice), (0, 990));

    // The 11th write: no free anchor left and no X-Payment → 402. The body
    // carries the x402 menu plus the free quota state.
    let (status, body) = sign_memory(&server.app, &alice, "alice 11th", &[]).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert!(body["accepts"].is_array(), "{body}");
    let free = &body["free_anchors"];
    assert_eq!(free["per_day"], 10, "{body}");
    assert_eq!(free["remaining"], 0, "{body}");
    assert_eq!(free["global_remaining"], 990, "{body}");
    assert!(free["resets_at"].as_str().unwrap().ends_with("T00:00:00Z"));

    // A second key still has its own ten.
    let bob = Keypair::new();
    anchor_ok(&server.app, &bob, "bob free anchor").await;
    assert_eq!(remaining_today(&server.state, &bob), (9, 989));

    // `mnemonic_whoami` shows each caller its own quota.
    let who = whoami(&server.app, &bob).await;
    assert_eq!(who["free_anchors"]["per_day"], 10, "{who}");
    assert_eq!(who["free_anchors"]["remaining"], 9, "{who}");
    assert_eq!(who["free_anchors"]["global_remaining"], 989, "{who}");
    let who = whoami(&server.app, &alice).await;
    assert_eq!(who["free_anchors"]["remaining"], 0, "{who}");
}

// ── 2. Global cap across keys ───────────────────────────────────────────────

#[tokio::test]
async fn global_cap_makes_the_fourth_anchor_across_keys_paid() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 3);
    for i in 0..3 {
        anchor_ok(&server.app, &Keypair::new(), &format!("key {i}")).await;
    }
    let fourth = Keypair::new();
    let (status, body) = sign_memory(&server.app, &fourth, "fourth key", &[]).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(body["free_anchors"]["global_remaining"], 0, "{body}");
    // The key itself never anchored, but no free anchor is usable.
    assert_eq!(body["free_anchors"]["remaining"], 0, "{body}");
    assert_eq!(remaining_today(&server.state, &fourth), (10, 0));
}

// ── 3. Consumption happens once, at anchor time ─────────────────────────────

#[tokio::test]
async fn parked_bundles_consume_at_anchor_time_and_the_loser_gets_402() {
    let _env = env_lock(true).await;
    let server = paid_server(1, 100);
    let kp = Keypair::new();

    // Both calls peek one free anchor, so both bundles are parked.
    let (s1, first) = sign_memory(&server.app, &kp, "first", &[]).await;
    let (s2, second) = sign_memory(&server.app, &kp, "second", &[]).await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK));
    // Parking consumed nothing.
    assert_eq!(remaining_today(&server.state, &kp), (1, 100));

    let (status, body) = submit(&server.app, &kp, &tool_result(&first)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(remaining_today(&server.state, &kp), (0, 99));

    // The second bundle finds no free anchor left: 402, nothing anchored,
    // nothing consumed.
    let (status, body) = submit(&server.app, &kp, &tool_result(&second)).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(body["status"], "payment_required", "{body}");
    assert_eq!(body["free_anchors"]["remaining"], 0, "{body}");
    assert_eq!(remaining_today(&server.state, &kp), (0, 99));
    assert_eq!(server.attestation_count(&kp.pubkey().to_string()), 1);
}

// ── 4. PAYMENT_MODE=none consumes nothing ───────────────────────────────────

#[tokio::test]
async fn payment_mode_none_consumes_no_free_anchors() {
    let _env = env_lock(true).await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .free_anchors(2, 2)
        .build();
    let kp = Keypair::new();
    for i in 0..4 {
        anchor_ok(&server.app, &kp, &format!("free deploy {i}")).await;
    }
    let rows: i64 = {
        let store = server.state.store.lock().unwrap();
        store
            .conn()
            .query_row("SELECT COUNT(*) FROM free_anchor_usage", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(rows, 0, "a free deploy must not touch the quota counters");
    let who = whoami(&server.app, &kp).await;
    assert!(who.get("free_anchors").is_none(), "{who}");
}

// ── 5. X-Payment keeps the paid path ────────────────────────────────────────

/// A caller that sends `X-Payment` asked to pay: the gate verifies the proof
/// (here it cannot reach the Solana RPC, so 401) and never falls back to the
/// free quota. Nothing is parked and no counter moves.
#[tokio::test]
async fn x_payment_header_keeps_the_paid_path() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let kp = Keypair::new();
    let proof = json!({"tx_sig": "unverifiable-sig", "network": "solana-mainnet"}).to_string();
    let (status, body) = sign_memory(&server.app, &kp, "paid", &[("x-payment", proof)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(remaining_today(&server.state, &kp), (10, 1000));
}

// ── 6. Delivery demotion refunds the free anchor ────────────────────────────

#[tokio::test]
async fn delivery_demotion_refunds_the_free_anchor() {
    // Real (mocked) Arweave + Solana path, so no synthetic anchor ids.
    let _env = env_lock(false).await;
    let arweave = MockArweave::read_fails("AR_TX_FREE_QUOTA");
    let post_tx = arweave.install();
    let solana = MockSolana::happy();
    let mut state = mnemonic_mcp::test_support::mock_state_for_delivery(
        "full",
        "x402",
        COST,
        &arweave.base_url(),
        &solana.base_url(),
        5,
        Duration::from_secs(60),
        Duration::from_millis(300),
        "",
        "",
    );
    Arc::get_mut(&mut state)
        .expect("fresh McpState has one owner")
        .free_anchors = payment::FreeAnchorLimits {
        per_key: 10,
        global: 1000,
    };
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_JWT_SECRET));
    let app = Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/api/sign-callback", post(sign_callback_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state.clone());

    let kp = Keypair::new();
    let (status, envelope) = sign_memory(&app, &kp, "demoted free anchor", &[]).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    let (status, body) = submit(&app, &kp, &tool_result(&envelope)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "delivery_not_confirmed", "{body}");
    assert_eq!(body["stage"], "refetch", "{body}");
    assert_eq!(post_tx.calls(), 1, "the write reached Arweave once");

    // The anchor was not confirmed, so the free anchor came back.
    assert_eq!(remaining_today(&state, &kp), (10, 1000));
    let write_mode: String = {
        let store = state.store.lock().unwrap();
        store
            .conn()
            .query_row(
                "SELECT write_mode FROM attestations WHERE attestation_id = ?",
                rusqlite::params![body["attestation_id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(write_mode, "local");
}

// ── 7. Universal Paywall rail ───────────────────────────────────────────────

#[tokio::test]
async fn universal_paywall_free_anchor_skips_the_charge_until_the_quota_is_spent() {
    let _env = env_lock(true).await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(COST)
        .free_anchors(1, 100)
        .universal_paywall(up_config())
        .build();
    let kp = Keypair::new();

    // The free anchor: no wallet link, no quote, no charge.
    anchor_ok(&server.app, &kp, "free under universal paywall").await;
    assert_eq!(remaining_today(&server.state, &kp), (0, 99));

    // Quota spent: the charge path starts with the wallet-link step, and the
    // body tells the agent why.
    let (status, envelope) = sign_memory(&server.app, &kp, "paid now", &[]).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    let (status, body) = submit(&server.app, &kp, &tool_result(&envelope)).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["status"], "awaiting_wallet_link", "{body}");
    assert_eq!(body["free_anchors"]["remaining"], 0, "{body}");
    assert_eq!(remaining_today(&server.state, &kp), (0, 99));
}

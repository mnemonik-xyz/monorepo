//! Integration tests for the free daily anchor quota.
//!
//! Each Google-linked account gets N free `anchored` (on-chain anchored)
//! writes per UTC day on a `PAYMENT_MODE=x402` deploy, under a per-IP share
//! and a global daily cap, for COSE bytes up to a size limit. The
//! pre-parking x402 gate in `mcp_handler` only peeks; the sign-callback
//! consumes one free anchor at anchor time and refunds it when delivery is
//! not confirmed (the global counter stays spent once a chain write
//! happened).
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
//! 6. Delivery demotion refunds the per-account anchor but not the global
//!    one (the chain write happened).
//! 7. Universal Paywall: a free anchor skips the wallet link and the charge;
//!    with no free anchor left, the charge path starts (428 wallet link).
//! 8. A key with no Google link gets no free anchor and is told why.
//! 9. Per-IP share: the real client IP counts, a spoofed X-Forwarded-For
//!    from an untrusted peer does not.
//! 10. A write over the free size limit takes the paid path.
//! 11. Content over `MNEMONIC_MAX_CONTENT_BYTES` is refused on every mode.
//! 12. Replays: the same callback twice, or concurrently, anchors once and
//!     consumes one free anchor; an already-anchored content hash is not
//!     anchored again.
//!
//! Counter atomicity under concurrency and the UTC day rollover are unit
//! tests in `mcp/src/payment.rs`.

mod _helpers;
#[path = "_helpers/delivery_harness.rs"]
mod delivery_harness;

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use _helpers::{TestServer, TEST_JWT_SECRET};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    middleware,
    routing::post,
    Router,
};
use delivery_harness::{MockArweave, MockSolana};
use http_body_util::BodyExt;
use mnemonic_core::codec::sign::sign_cose;
use mnemonic_core::storage::AttestationStore;
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
    post_json_from(app, path, headers, body, None).await
}

/// `post_json` from TCP peer `peer` (axum `ConnectInfo`), as the real
/// server sees every request.
async fn post_json_from(
    app: &Router,
    path: &str,
    headers: &[(&str, String)],
    body: Value,
    peer: Option<SocketAddr>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, value);
    }
    let mut req = builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    if let Some(peer) = peer {
        req.extensions_mut().insert(ConnectInfo(peer));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, json)
}

/// `tools/call mnemonic_sign_memory` (anchored) as `kp`. Returns the HTTP
/// status and the raw response envelope.
async fn sign_memory(
    app: &Router,
    kp: &Keypair,
    content: &str,
    extra: &[(&str, String)],
) -> (StatusCode, Value) {
    sign_memory_from(app, kp, content, extra, None).await
}

/// `sign_memory` from TCP peer `peer`.
async fn sign_memory_from(
    app: &Router,
    kp: &Keypair,
    content: &str,
    extra: &[(&str, String)],
    peer: Option<SocketAddr>,
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
            "arguments": {"content": content, "mode": "anchored"},
        },
    });
    post_json_from(app, "/mcp", &headers, body, peer).await
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

/// Park, sign and anchor one anchored write; assert it anchored.
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

/// The Google account the tests link to `kp`.
fn google_sub(kp: &Keypair) -> String {
    format!("google-{}", kp.pubkey())
}

/// A new agent key linked to its own Google account on `state`.
fn linked_key(state: &McpState) -> Keypair {
    let kp = Keypair::new();
    link(state, &kp);
    kp
}

fn link(state: &McpState, kp: &Keypair) {
    let store = state.store.lock().unwrap();
    mnemonic_mcp::test_support::link_google_account(
        &store,
        &google_sub(kp),
        &kp.pubkey().to_string(),
    );
}

/// `(account_remaining, global_remaining)` today for `kp`'s Google account.
fn remaining_today(state: &McpState, kp: &Keypair) -> (u32, u32) {
    let store = state.store.lock().unwrap();
    let keys = payment::FreeAnchorKeys::from_parts(
        store.conn(),
        &google_sub(kp),
        mnemonic_mcp::client_ip::UNKNOWN_CLIENT,
    )
    .unwrap();
    let left = payment::free_anchors_remaining(
        store.conn(),
        &keys,
        &payment::utc_day(chrono::Utc::now()),
        state.free_anchors,
    )
    .unwrap();
    (left.account, left.global)
}

/// Free anchors left today for client `ip`.
fn ip_remaining_today(state: &McpState, ip: &str) -> u32 {
    let store = state.store.lock().unwrap();
    let keys =
        payment::FreeAnchorKeys::from_parts(store.conn(), "any-account", ip.parse().unwrap())
            .unwrap();
    payment::free_anchors_remaining(
        store.conn(),
        &keys,
        &payment::utc_day(chrono::Utc::now()),
        state.free_anchors,
    )
    .unwrap()
    .ip
}

// ── 1. Ten free anchors per key, then payment ───────────────────────────────

#[tokio::test]
async fn ten_free_anchors_per_key_then_the_eleventh_requires_payment() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let alice = linked_key(&server.state);

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
    let bob = linked_key(&server.state);
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
        anchor_ok(&server.app, &linked_key(&server.state), &format!("key {i}")).await;
    }
    let fourth = linked_key(&server.state);
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
    let kp = linked_key(&server.state);

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
    let kp = linked_key(&server.state);
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
    let kp = linked_key(&server.state);
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
        per_account: 10,
        per_ip: 100,
        global: 1000,
        max_bytes: payment::DEFAULT_FREE_ANCHOR_MAX_BYTES,
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

    let kp = linked_key(&state);
    let (status, envelope) = sign_memory(&app, &kp, "demoted free anchor", &[]).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    let (status, body) = submit(&app, &kp, &tool_result(&envelope)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "delivery_not_confirmed", "{body}");
    assert_eq!(body["stage"], "refetch", "{body}");
    assert_eq!(post_tx.calls(), 1, "the write reached Arweave once");

    // The anchor was not confirmed, so the account's free anchor came back.
    // The chain write did happen (the operator paid its fees), so the global
    // budget stays spent.
    assert_eq!(remaining_today(&state, &kp), (10, 999));
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
    let kp = linked_key(&server.state);

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

// ── 8. Google account required ──────────────────────────────────────────────

#[tokio::test]
async fn a_key_without_a_google_link_gets_no_free_anchor_and_is_told_why() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let unlinked = Keypair::new();

    let (status, body) = sign_memory(&server.app, &unlinked, "no google", &[]).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert!(
        body["accepts"].is_array(),
        "the paid menu is still offered: {body}"
    );
    let free = &body["free_anchors"];
    assert_eq!(free["eligible"], false, "{body}");
    assert_eq!(free["reason"], "google_account_required", "{body}");
    assert!(free["link_hint"]
        .as_str()
        .unwrap()
        .contains("/oauth/google/link"));
    assert!(free.get("remaining").is_none(), "{body}");

    let who = whoami(&server.app, &unlinked).await;
    assert_eq!(
        who["free_anchors"]["reason"], "google_account_required",
        "{who}"
    );

    // Paid anchoring needs no Google account: the X-Payment path is taken
    // (the proof cannot be verified here, hence 401, not 402).
    let proof = json!({"tx_sig": "unverifiable-sig", "network": "solana-mainnet"}).to_string();
    let (status, body) = sign_memory(&server.app, &unlinked, "paid", &[("x-payment", proof)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // After linking, the same key gets free anchors.
    link(&server.state, &unlinked);
    anchor_ok(&server.app, &unlinked, "now linked").await;
    let who = whoami(&server.app, &unlinked).await;
    assert_eq!(who["free_anchors"]["eligible"], true, "{who}");
    assert_eq!(who["free_anchors"]["remaining"], 9, "{who}");
}

// ── 9. Per-IP share, real client IP ─────────────────────────────────────────

fn peer(addr: &str) -> Option<SocketAddr> {
    Some(addr.parse().unwrap())
}

#[tokio::test]
async fn per_ip_share_counts_the_real_client_ip_and_ignores_spoofed_headers() {
    let _env = env_lock(true).await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(COST)
        .free_anchor_limits(payment::FreeAnchorLimits {
            per_account: 10,
            per_ip: 2,
            global: 1000,
            max_bytes: payment::DEFAULT_FREE_ANCHOR_MAX_BYTES,
        })
        .build();

    // Behind the proxy (trusted loopback peer): the rightmost untrusted
    // X-Forwarded-For hop is the client. A forged leftmost hop changes nothing.
    let proxy = peer("127.0.0.1:40000");
    for i in 0..2 {
        let kp = linked_key(&server.state);
        let xff = vec![("x-forwarded-for", format!("10{i}.1.1.1, 198.51.100.7"))];
        let (status, envelope) =
            sign_memory_from(&server.app, &kp, &format!("proxied {i}"), &xff, proxy).await;
        assert_eq!(status, StatusCode::OK, "{envelope}");
        // The browser that posts the signature has no peer IP: the counter
        // still charges the agent's IP recorded on the parked bundle.
        let (status, body) = submit(&server.app, &kp, &tool_result(&envelope)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(ip_remaining_today(&server.state, "198.51.100.7"), 0);
    let third = linked_key(&server.state);
    let xff = vec![("x-forwarded-for", "8.8.8.8, 198.51.100.7".to_string())];
    let (status, body) = sign_memory_from(&server.app, &third, "third", &xff, proxy).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(body["free_anchors"]["reason"], "ip_quota_used", "{body}");
    assert_eq!(body["free_anchors"]["ip_remaining"], 0, "{body}");
    assert_eq!(remaining_today(&server.state, &third), (10, 998));

    // A different client behind the same proxy has its own share.
    let other = vec![("x-forwarded-for", "203.0.113.20".to_string())];
    let (status, envelope) = sign_memory_from(&server.app, &third, "other ip", &other, proxy).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");

    // An untrusted public peer cannot pick its bucket with the headers: every
    // request counts against the peer IP.
    let direct = peer("192.0.2.50:5555");
    for i in 0..2 {
        let kp = linked_key(&server.state);
        let spoof = vec![
            ("x-forwarded-for", format!("198.18.0.{i}")),
            ("x-real-ip", format!("198.18.1.{i}")),
        ];
        let (status, envelope) =
            sign_memory_from(&server.app, &kp, &format!("spoof {i}"), &spoof, direct).await;
        assert_eq!(status, StatusCode::OK, "{envelope}");
        let (status, body) = submit(&server.app, &kp, &tool_result(&envelope)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(ip_remaining_today(&server.state, "192.0.2.50"), 0);
    assert_eq!(ip_remaining_today(&server.state, "198.18.0.0"), 2);
    let kp = linked_key(&server.state);
    let spoof = vec![("x-forwarded-for", "198.18.0.99".to_string())];
    let (status, body) = sign_memory_from(&server.app, &kp, "spoof 3", &spoof, direct).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(body["free_anchors"]["reason"], "ip_quota_used", "{body}");
}

// ── 10. Size limit for the free tier ────────────────────────────────────────

#[tokio::test]
async fn a_write_over_the_free_size_limit_takes_the_paid_path() {
    let _env = env_lock(true).await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(COST)
        .free_anchor_limits(payment::FreeAnchorLimits {
            per_account: 10,
            per_ip: 100,
            global: 1000,
            max_bytes: 2_000,
        })
        .build();
    let kp = linked_key(&server.state);

    let (status, body) = sign_memory(&server.app, &kp, &"x".repeat(3_000), &[]).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert!(body["accepts"].is_array(), "{body}");
    assert_eq!(body["free_anchors"]["reason"], "too_large", "{body}");
    assert_eq!(body["free_anchors"]["max_bytes"], 2_000, "{body}");
    // Nothing stays parked and nothing is counted.
    assert_eq!(
        server
            .state
            .pending
            .user_count(&kp.pubkey().to_string())
            .await,
        0
    );
    assert_eq!(remaining_today(&server.state, &kp), (10, 1000));

    // A small write from the same key is still free.
    anchor_ok(&server.app, &kp, "small").await;
    assert_eq!(remaining_today(&server.state, &kp), (9, 999));
}

// ── 11. Server-wide content cap ─────────────────────────────────────────────

#[tokio::test]
async fn content_over_the_server_cap_is_refused_on_every_mode() {
    let _env = env_lock(true).await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let kp = Keypair::new();
    let jwt = mnemonic_mcp::test_support::mint_jwt(&kp.pubkey().to_string(), TEST_JWT_SECRET);
    let too_big = "y".repeat(mnemonic_mcp::pending::MAX_CONTENT_BYTES + 1);
    for mode in ["local", "anchored"] {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "mnemonic_sign_memory",
                "arguments": {"content": too_big, "mode": mode},
            },
        });
        let (status, envelope) = post_json(
            &server.app,
            "/mcp",
            &[("authorization", format!("Bearer {jwt}"))],
            body,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{envelope}");
        assert_eq!(envelope["error"]["code"], -32602, "{mode}: {envelope}");
        assert!(envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("MNEMONIC_MAX_CONTENT_BYTES"));
    }
    assert_eq!(server.attestation_count(&kp.pubkey().to_string()), 0);
}

// ── 12. Replays ─────────────────────────────────────────────────────────────

/// The signed callback body for a parked bundle, so a test can post the
/// exact same bytes more than once.
fn callback_body(kp: &Keypair, parked: &Value) -> Value {
    let cbor = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        parked["canonical_cbor_b64"].as_str().unwrap(),
    )
    .unwrap();
    let cose = sign_cose(&cbor, kp).unwrap();
    json!({
        "correlation_id": parked["correlation_id"],
        "cose_signed_bytes": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, cose),
        "signer_pubkey": kp.pubkey().to_string(),
    })
}

#[tokio::test]
async fn the_same_callback_posted_twice_anchors_once_and_consumes_one_free_anchor() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let kp = linked_key(&server.state);
    let (_, envelope) = sign_memory(&server.app, &kp, "replayed", &[]).await;
    let body = callback_body(&kp, &tool_result(&envelope));

    let (status, first) = post_json(&server.app, "/api/sign-callback", &[], body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (status, second) = post_json(&server.app, "/api/sign-callback", &[], body).await;
    assert_eq!(
        status,
        StatusCode::GONE,
        "a completed bundle cannot be reused: {second}"
    );
    assert_eq!(remaining_today(&server.state, &kp), (9, 999));
    assert_eq!(server.attestation_count(&kp.pubkey().to_string()), 1);
}

#[tokio::test]
async fn concurrent_duplicate_callbacks_anchor_once_and_consume_one_free_anchor() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let kp = linked_key(&server.state);
    let (_, envelope) = sign_memory(&server.app, &kp, "raced", &[]).await;
    let body = callback_body(&kp, &tool_result(&envelope));

    let calls = (0..8).map(|_| post_json(&server.app, "/api/sign-callback", &[], body.clone()));
    let results = futures::future::join_all(calls).await;
    let ok = results.iter().filter(|(s, _)| *s == StatusCode::OK).count();
    assert_eq!(ok, 1, "exactly one callback anchors: {results:?}");
    for (status, body) in &results {
        assert!(
            *status == StatusCode::OK || *status == StatusCode::GONE,
            "unexpected {status}: {body}"
        );
    }
    // Losers refunded their temporary claim: one anchor consumed in total.
    assert_eq!(remaining_today(&server.state, &kp), (9, 999));
    assert_eq!(server.attestation_count(&kp.pubkey().to_string()), 1);
}

#[tokio::test]
async fn an_already_anchored_content_hash_is_not_anchored_again() {
    let _env = env_lock(true).await;
    let server = paid_server(10, 1000);
    let kp = linked_key(&server.state);
    let (_, envelope) = sign_memory(&server.app, &kp, "anchored before", &[]).await;
    let parked = tool_result(&envelope);
    let content_hash = parked["content_hash"]
        .as_str()
        .expect("content_hash")
        .to_string();
    // An anchored row for the same artifact already exists.
    {
        let store = server.state.store.lock().unwrap();
        store
            .save_attestation(
                "existing-row",
                "anchored before",
                &content_hash,
                &[],
                "SOL_EXISTING",
                "AR_EXISTING",
                &kp.pubkey().to_string(),
                &kp.pubkey().to_string(),
                &chrono::Utc::now().to_rfc3339(),
                mnemonic_core::storage::WriteMode::Anchored,
                mnemonic_core::storage::Visibility::Private,
                &[0.1; 8],
            )
            .unwrap();
    }
    let (status, body) = submit(&server.app, &kp, &parked).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["already_anchored"], true, "{body}");
    assert_eq!(body["attestation_id"], "existing-row", "{body}");
    assert_eq!(body["solana_tx"], "SOL_EXISTING", "{body}");
    // No second row, no free anchor spent.
    assert_eq!(server.attestation_count(&kp.pubkey().to_string()), 1);
    assert_eq!(remaining_today(&server.state, &kp), (10, 1000));
}

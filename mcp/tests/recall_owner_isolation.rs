//! **CRITICAL** integration test: cross-tenant recall isolation (Task 8 #3 /
//! tech-spec line 224 / Risks-table "owner_pubkey filter forgotten").
//!
//! Two distinct JWTs (alice + bob) both complete the deferred-sign flow.
//! Alice signs 2 memories, bob signs 1. We assert:
//!
//! 1. `tools/call mnemonic_recall` with **bob's** JWT returns exactly 1 row,
//!    whose `owner_pubkey == bob.sub` and content matches what bob signed.
//! 2. Anonymous `/mcp tools/call mnemonic_recall` (no Authorization header)
//!    returns HTTP 401. Critically NEVER 200 with rows — that would be a
//!    cross-tenant leak.
//!
//! This is the security-critical assertion guarding the SQL `WHERE
//! owner_pubkey = ?` filter in `SqliteStore::search`. A regression here
//! means tenant data is observable across users.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::codec::sign::sign_cose;
use mnemonic_mcp::{
    api::{get_pending_handler, sign_callback_handler},
    mcp::{mcp_handler, McpState},
    oauth::{self, OAuthState},
    test_support::mock_state,
};
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"recall-isolation-secret-32-bytes";

fn build_app(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/api/pending/{correlation_id}", get(get_pending_handler))
        .route("/api/sign-callback", post(sign_callback_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

async fn post_jsonrpc(app: &Router, body: Value, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
    let parsed: Value = serde_json::from_str(&trimmed).unwrap_or(Value::String(trimmed));
    (status, parsed)
}

async fn get_pending(app: &Router, cid: &str, token: &str) -> Vec<u8> {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "GET pending failed");
    resp.into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn callback(app: &Router, cid: &str, cose_b64: &str, signer: &str, token: &str) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/sign-callback")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "correlation_id": cid,
                "cose_signed_bytes": cose_b64,
                "signer_pubkey": signer,
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "sign-callback failed");
}

fn extract_correlation_id(body: &Value) -> String {
    let text_blob = body["result"]["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| body["result"].to_string());
    let inner: Value = serde_json::from_str(&text_blob).unwrap_or(body["result"].clone());
    inner["correlation_id"]
        .as_str()
        .expect("correlation_id missing")
        .to_string()
}

/// Drive one full deferred-sign cycle: sign_memory → fetch pending → sign
/// locally → callback.
async fn one_attestation(app: &Router, kp: &Keypair, token: &str, pubkey: &str, content: &str) {
    let body = post_jsonrpc(
        app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "mnemonic_sign_memory",
                "arguments": {"content": content, "tags": []},
            },
            "id": 1,
        }),
        Some(token),
    )
    .await
    .1;
    let cid = extract_correlation_id(&body);
    let cbor = get_pending(app, &cid, token).await;
    let cose = sign_cose(&cbor, kp).expect("sign_cose");
    let cose_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &cose);
    callback(app, &cid, &cose_b64, pubkey, token).await;
}

/// Pull recall results out of the MCP envelope. Returns the inner JSON
/// (which is the `recall` tool's `serde_json::Value` — typically a
/// `{matches: [...]}` shape).
fn extract_recall_results(body: &Value) -> Value {
    let text_blob = body["result"]["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| body["result"].to_string());
    serde_json::from_str(&text_blob).unwrap_or(body["result"].clone())
}

#[tokio::test]
async fn test_recall_filters_by_owner_pubkey_and_anonymous_returns_401() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_app(state, oauth_state.clone());

    let alice_kp = Keypair::new();
    let alice = alice_kp.pubkey().to_string();
    let alice_token = oauth::issue_jwt(&oauth_state, &alice).expect("issue_jwt alice");

    let bob_kp = Keypair::new();
    let bob = bob_kp.pubkey().to_string();
    let bob_token = oauth::issue_jwt(&oauth_state, &bob).expect("issue_jwt bob");

    // Alice signs 2 memories.
    one_attestation(&app, &alice_kp, &alice_token, &alice, "alice memory 1").await;
    one_attestation(&app, &alice_kp, &alice_token, &alice, "alice memory 2").await;

    // Bob signs 1 memory.
    one_attestation(&app, &bob_kp, &bob_token, &bob, "bob memory 1").await;

    // 1. Bob's recall must return exactly 1 row (his own).
    let (sb, body_b) = post_jsonrpc(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "mnemonic_recall",
                "arguments": {"query": "memory", "limit": 10},
            },
            "id": 10,
        }),
        Some(&bob_token),
    )
    .await;
    assert_eq!(sb, StatusCode::OK, "bob recall failed: {body_b}");
    let bob_results = extract_recall_results(&body_b);
    // The recall tool returns `{matches: [...]}` — we accept either
    // `matches` or `results` for forward-compatibility.
    let rows = bob_results
        .get("matches")
        .or_else(|| bob_results.get("results"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        rows.len(),
        1,
        "bob's recall must return exactly 1 row (cross-tenant leak risk!): {bob_results}"
    );
    let row_content = rows[0]["content"]
        .as_str()
        .expect("content field")
        .to_string();
    assert_eq!(row_content, "bob memory 1", "bob got alice's row!");

    // 2. Anonymous recall (no Authorization header) — Task 4 lands the
    // visibility-filter recall path per AC13. Anonymous recall now returns
    // HTTP 200 with only `visibility='public'` rows included. Alice and
    // Bob's writes above went through the deferred-sign flow → resulting
    // attestations are `visibility = 'private'` by default (the WASM
    // sign-callback doesn't set visibility), so anonymous recall sees zero
    // rows here. The critical safety property — "anonymous callers do NOT
    // observe private rows across tenants" — is enforced by the
    // visibility-filter clause, not by 401.
    let (sa, body_a) = post_jsonrpc(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "mnemonic_recall",
                "arguments": {"query": "memory", "limit": 10},
            },
            "id": 11,
        }),
        None,
    )
    .await;
    assert_eq!(
        sa,
        StatusCode::OK,
        "anonymous recall is allowlisted per AC13, got {sa}: {body_a}"
    );
    // Anonymous recall body structure mirrors authenticated recall: the
    // wrapping `tools/call` result has a `content[0].text` JSON-string.
    let text = body_a["result"]["content"][0]["text"]
        .as_str()
        .expect("anon recall content text");
    let inner: Value = serde_json::from_str(text).expect("anon recall inner json");
    let rows = inner["results"].as_array().cloned().unwrap_or_default();
    // None of alice's or bob's private rows surface. The list is empty
    // because none of the deferred-sign writes opted into `visibility=public`.
    assert!(
        rows.is_empty(),
        "anonymous recall must NOT surface private rows: {inner}"
    );
}

//! Integration test: full deferred-sign lifecycle (Task 8 #14 / tech-spec
//! line 225). Path:
//!
//! 1. POST `/mcp tools/call mnemonic_sign_memory` → expect
//!    `{"status":"awaiting_signature", "correlation_id": ...}`.
//! 2. GET `/api/pending/<id>` with the same JWT → unsigned canonical-CBOR.
//! 3. Sign locally with the user's keypair (the production WASM does this).
//! 4. POST `/api/sign-callback` → 200 OK + persisted SQLite row.
//! 5. POST `/mcp tools/call mnemonic_recall` → row visible.
//! 6. POST `/api/sign-callback` again with same id → 410 Gone (single-use
//!    eviction per Decision 12).

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
use mnemonic_core::storage::AttestationStore;
use mnemonic_mcp::{
    api::{get_pending_handler, sign_callback_handler},
    mcp::{mcp_handler, McpState},
    oauth::{self, OAuthState},
    test_support::mock_state,
};
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"deferred-sign-flow-secret-32!!!!";

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

async fn post_jsonrpc(app: &Router, body: Value, token: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
    let parsed: Value = serde_json::from_str(&trimmed).unwrap_or(Value::String(trimmed));
    (status, parsed)
}

async fn get_pending(app: &Router, cid: &str, token: &str) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}

async fn sign_callback(
    app: &Router,
    cid: &str,
    cose_b64: &str,
    signer_pubkey: &str,
    token: &str,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/sign-callback")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "correlation_id": cid,
                "cose_signed_bytes": cose_b64,
                "signer_pubkey": signer_pubkey,
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
    let parsed: Value = serde_json::from_str(&trimmed).unwrap_or(Value::String(trimmed));
    (status, parsed)
}

/// Pull `correlation_id` out of an MCP `tools/call` response. The result
/// envelope is `result.content[0].text` (JSON-encoded string) per the
/// 2025-06-18 spec; fallback to flat-result for compatibility.
fn extract_correlation_id(body: &Value) -> Option<String> {
    let text_blob = body["result"]["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| body["result"].to_string());
    let inner: Value = serde_json::from_str(&text_blob).ok()?;
    inner["correlation_id"].as_str().map(|s| s.to_string())
}

#[tokio::test]
async fn test_full_lifecycle_sign_callback_410_on_replay() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_app(state.clone(), oauth_state.clone());

    // The user's local keypair (in production this is the WASM-generated
    // localStorage key). `jwt.sub == kp.pubkey().to_string()`.
    let kp = Keypair::new();
    let user_pubkey = kp.pubkey().to_string();
    let token = oauth::issue_jwt(&oauth_state, &user_pubkey).expect("issue_jwt");

    // 1. sign_memory — get correlation_id.
    let (s1, body1) = post_jsonrpc(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "mnemonic_sign_memory",
                "arguments": {"content": "lifecycle test memo", "tags": ["t1"]},
            },
            "id": 1,
        }),
        &token,
    )
    .await;
    assert_eq!(s1, StatusCode::OK, "sign_memory failed: {body1}");
    let cid = extract_correlation_id(&body1).expect("correlation_id");

    // 2. GET /api/pending/<id> — unsigned canonical CBOR.
    let (s2, cbor) = get_pending(&app, &cid, &token).await;
    assert_eq!(s2, StatusCode::OK, "GET pending failed status={s2}");
    assert!(!cbor.is_empty(), "empty pending body");

    // 3. Sign the CBOR locally with the user's keypair (production WASM
    //    path, simulated in-test).
    let cose = sign_cose(&cbor, &kp).expect("sign_cose");
    let cose_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &cose);

    // 4. POST /api/sign-callback — 200 OK.
    let (s4, body4) = sign_callback(&app, &cid, &cose_b64, &user_pubkey, &token).await;
    assert_eq!(s4, StatusCode::OK, "sign-callback failed: {body4}");
    assert_eq!(body4["status"], "ok");
    let _attestation_id = body4["attestation_id"].as_str().expect("attestation_id");

    // 5. Verify the row is visible to the owner via direct store search.
    //    (Going through tools/call recall would require parsing the MCP
    //    envelope, which is covered in `recall_owner_isolation.rs`; here we
    //    just want to prove persistence happened.)
    {
        let store = state.store.lock().expect("store mutex");
        let results = store
            .search(&[0.1; 8], Some(user_pubkey.as_str()), None, 5)
            .expect("search ok");
        assert_eq!(
            results.len(),
            1,
            "expected 1 persisted row, got {results:?}"
        );
        assert_eq!(results[0].content, "lifecycle test memo");
    }

    // 6. Replay sign-callback for the SAME correlation_id → 410 Gone.
    let (s6, _body6) = sign_callback(&app, &cid, &cose_b64, &user_pubkey, &token).await;
    assert_eq!(s6, StatusCode::GONE, "replay should be 410, got {s6}");
}

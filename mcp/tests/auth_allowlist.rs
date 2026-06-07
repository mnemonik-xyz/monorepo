//! Integration test: bearer-auth allowlist coverage (Decision 9).
//!
//! Covers Task 8 list item #5 / tech-spec lines 233–234. The middleware
//! permits the JSON-RPC discovery handshake (`initialize`, `tools/list`)
//! through without a Bearer JWT — Cursor and Claude.ai post these BEFORE
//! completing OAuth. Anything else (`tools/call sign_memory`, etc.) demands a
//! valid JWT.
//!
//! Catches the regression class where someone tightens the allowlist (e.g.
//! removes `tools/list` to "harden" it) and silently breaks MCP discovery
//! everywhere it's installed.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use mnemonic_mcp::{
    mcp::mcp_handler,
    oauth::{self, OAuthState},
    test_support::{mint_jwt, mock_state},
};
use serde_json::Value;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"auth-allowlist-secret-32-bytes-!";

fn build_router(state: Arc<mnemonic_mcp::mcp::McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

async fn post_json(app: &Router, body: Value, auth: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    // Streamable-HTTP NDJSON — body may be a single JSON line. Trim trailing
    // newline before parsing.
    let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
    let parsed: Value = serde_json::from_str(&trimmed).unwrap_or(Value::String(trimmed));
    (status, parsed)
}

#[tokio::test]
async fn test_tools_list_initialize_no_auth_200_sign_memory_no_auth_401() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state.clone());

    // 1. tools/list with NO Authorization header → 200 (allowlisted).
    let (s1, body1) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "tools/list", "id": 1}),
        None,
    )
    .await;
    assert_eq!(s1, StatusCode::OK, "tools/list anon must succeed: {body1}");
    // Sanity — body has the tools array.
    assert!(
        body1["result"]["tools"].is_array(),
        "tools/list response missing tools[]: {body1}"
    );

    // 2. initialize with NO Authorization header → 200 (allowlisted).
    let (s2, body2) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "initialize", "id": 2}),
        None,
    )
    .await;
    assert_eq!(s2, StatusCode::OK, "initialize anon must succeed: {body2}");
    assert_eq!(body2["result"]["protocolVersion"], "2025-06-18");

    // 3. tools/call sign_memory with NO Authorization header → 401.
    let (s3, body3) = post_json(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "mnemonic_sign_memory", "arguments": {"content": "hi"}},
            "id": 3,
        }),
        None,
    )
    .await;
    assert_eq!(
        s3,
        StatusCode::UNAUTHORIZED,
        "tools/call sign_memory anon must be 401: {body3}"
    );
    // Body should carry a JSON-RPC -32001 envelope (per `oauth::jsonrpc_unauthorized`).
    assert_eq!(
        body3["error"]["code"], -32001,
        "expected -32001 unauthorized envelope: {body3}"
    );

    // 4. tools/call sign_memory WITH valid JWT → does NOT 401 (it may return
    //    a different status if downstream fails, but the auth gate must pass).
    let token = mint_jwt("test-pubkey-base58", TEST_SECRET);
    let (s4, _) = post_json(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "mnemonic_sign_memory", "arguments": {"content": "hi"}},
            "id": 4,
        }),
        Some(&token),
    )
    .await;
    assert_ne!(
        s4,
        StatusCode::UNAUTHORIZED,
        "valid JWT must clear the auth gate"
    );
}

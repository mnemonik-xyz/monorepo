//! Integration test: OAuth-authenticated MCP tool call (Task 8 #2 /
//! tech-spec line 223).
//!
//! Mints a JWT directly via `oauth::issue_jwt` (skipping the full
//! authorize/token round-trip — that's covered by `oauth_flow.rs`), drives
//! `tools/list` and `tools/call mnemonic_sign_memory` through the full
//! stack with the bearer-auth middleware live, and asserts:
//!
//! 1. `tools/list` returns exactly 7 tools with the canonical names
//!    (Decision 12 added `mcp_auth` + `mnemonic_check_pending` on top of
//!    the original 5).
//! 2. `tools/call mnemonic_sign_memory` (with valid JWT) returns the
//!    deferred-signing envelope (`status: "awaiting_signature"`,
//!    `correlation_id`, `approve_url`).
//!
//! This is the "happy path" for the HTTP transport — anything regressing
//! tool discovery or the deferred-sign return shape fails here first.

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
    mcp::{mcp_handler, McpState},
    oauth::{self, OAuthState},
    test_support::mock_state,
};
use serde_json::Value;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"oauth-tool-call-secret-32-bytes!";

const EXPECTED_TOOLS: &[&str] = &[
    "mnemonic_whoami",
    "mnemonic_sign_memory",
    "mnemonic_verify",
    "mnemonic_prove_identity",
    "mnemonic_recall",
    "mcp_auth",
    "mnemonic_check_pending",
];

fn build_router(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_handler))
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
    if let Some(tok) = token {
        req = req.header("authorization", format!("Bearer {tok}"));
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

#[tokio::test]
async fn test_tools_list_5_tools_and_sign_memory_returns_awaiting_signature() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let app = build_router(state, oauth_state.clone());

    let user_pubkey = "test-user-pubkey-base58";
    let token = oauth::issue_jwt(&oauth_state, user_pubkey).expect("issue_jwt");

    // 1. tools/list — assert exactly 7 expected tools by name (Decision 12
    // added `mcp_auth` + `mnemonic_check_pending` on top of the original 5).
    let (s1, body1) = post_jsonrpc(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "tools/list", "id": 1}),
        Some(&token),
    )
    .await;
    assert_eq!(s1, StatusCode::OK, "tools/list failed: {body1}");
    let tools = body1["result"]["tools"].as_array().expect("tools is array");
    assert_eq!(tools.len(), 7, "want 7 tools, got {}: {body1}", tools.len());
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in EXPECTED_TOOLS {
        assert!(
            names.contains(expected),
            "missing tool {expected} in {names:?}"
        );
    }

    // 2. tools/call mnemonic_sign_memory — assert the deferred envelope.
    let (s2, body2) = post_jsonrpc(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "mnemonic_sign_memory",
                "arguments": {"content": "remember this", "tags": ["t1"]},
            },
            "id": 2,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(s2, StatusCode::OK, "tools/call failed: {body2}");

    // The MCP `tools/call` envelope wraps the tool result under
    // `result.content[0].text` (JSON-encoded) per the MCP 2025 spec; we
    // tolerate either that shape OR a flat `result.<field>` shape because
    // `tools.rs::sign_memory_deferred` returns a Value directly.
    let text_blob = body2["result"]["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| body2["result"].to_string());
    let inner: Value = serde_json::from_str(&text_blob).unwrap_or(body2["result"].clone());

    assert_eq!(
        inner["status"], "awaiting_signature",
        "want awaiting_signature, got: {inner}"
    );
    let correlation_id = inner["correlation_id"]
        .as_str()
        .expect("correlation_id is string");
    assert!(!correlation_id.is_empty(), "correlation_id is empty");
    // UUID v4 has 36 chars (8-4-4-4-12 hex with dashes).
    assert_eq!(
        correlation_id.len(),
        36,
        "correlation_id is not UUIDv4-shape: {correlation_id}"
    );

    let approve_url = inner["approve_url"]
        .as_str()
        .expect("approve_url is string");
    assert!(
        approve_url.contains(&format!("/sign/{correlation_id}")),
        "approve_url should embed correlation_id: {approve_url}"
    );
    assert_eq!(
        inner["expires_in"], 300,
        "expires_in should be 300s (Decision 12 TTL)"
    );
}

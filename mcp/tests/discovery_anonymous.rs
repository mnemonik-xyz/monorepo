//! Integration tests for the anonymous discovery surfaces wired in Task 2
//! of agent-native-distribution.
//!
//! Six TDD anchors:
//!   1. `prompts_list_returns_seven_without_bearer` — `prompts/list` is
//!      accessible without an `Authorization` header and returns the
//!      seven skill manifests.
//!   2. `resources_list_returns_seven_without_bearer` — same shape for
//!      `resources/list`.
//!   3. `resources_read_returns_full_markdown_for_attest` — `resources/read`
//!      with `uri = mnemonik://skills/mnemonik-attest.md` returns the
//!      verbatim manifest body.
//!   4. `tools_list_enriched_with_purpose_trigger` — every base tool's
//!      `description` contains the literal `Purpose:` and `Trigger:`
//!      sub-strings from its matching skill manifest.
//!   5. `initialize_surfaces_embedder_metadata` — `initialize` carries
//!      `embedder.model_id`, `embedder.model_version`, `embedder.dim`.
//!   6. `prompts_list_with_invalid_bearer_passes_through` — allowlisted
//!      methods must NOT 401 on a malformed Bearer.
//!
//! The middleware wiring mirrors `tests/auth_allowlist.rs` so we share
//! the same `mock_state` + `OAuthState` plumbing.

#![cfg(feature = "test-support")]

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
    mcp::{mcp_handler, skills, McpState},
    oauth::{self, OAuthState},
    test_support::mock_state,
};
use serde_json::Value;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"discovery-anon-secret-32-bytes-!";

fn build_router(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

async fn post_json(app: &Router, body: Value, auth: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(token) = auth {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = builder
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
async fn prompts_list_returns_seven_without_bearer() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, body) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "prompts/list", "id": 1}),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "anon prompts/list must be 200: {body}"
    );
    let prompts = body["result"]["prompts"]
        .as_array()
        .unwrap_or_else(|| panic!("missing prompts array: {body}"));
    assert!(
        prompts.len() >= 7,
        "expected >=7 prompts, got {}: {body}",
        prompts.len()
    );
    // Each entry exposes name + description.
    for prompt in prompts {
        let name = prompt["name"]
            .as_str()
            .unwrap_or_else(|| panic!("prompt missing name: {prompt}"));
        assert!(
            name.starts_with("mnemonik-"),
            "prompt name must be a mnemonik skill: {name}"
        );
        let desc = prompt["description"]
            .as_str()
            .unwrap_or_else(|| panic!("prompt missing description: {prompt}"));
        assert!(
            !desc.trim().is_empty(),
            "prompt description must be non-empty: {prompt}"
        );
    }
}

#[tokio::test]
async fn resources_list_returns_seven_without_bearer() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, body) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "resources/list", "id": 2}),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "anon resources/list must be 200: {body}"
    );
    let resources = body["result"]["resources"]
        .as_array()
        .unwrap_or_else(|| panic!("missing resources array: {body}"));
    assert!(
        resources.len() >= 7,
        "expected >=7 resources, got {}: {body}",
        resources.len()
    );
    for resource in resources {
        let uri = resource["uri"]
            .as_str()
            .unwrap_or_else(|| panic!("resource missing uri: {resource}"));
        assert!(
            uri.starts_with("mnemonik://skills/"),
            "resource uri must use mnemonik scheme: {uri}"
        );
        assert!(
            uri.ends_with(".md"),
            "resource uri must end with .md: {uri}"
        );
        assert_eq!(
            resource["mimeType"], "text/markdown",
            "resource mimeType must be text/markdown: {resource}"
        );
    }
}

#[tokio::test]
async fn resources_read_returns_full_markdown_for_attest() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let uri = "mnemonik://skills/mnemonik-attest.md";
    let (status, body) = post_json(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "resources/read",
            "id": 3,
            "params": {"uri": uri},
        }),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "anon resources/read must be 200: {body}"
    );
    let contents = body["result"]["contents"]
        .as_array()
        .unwrap_or_else(|| panic!("missing contents array: {body}"));
    assert_eq!(
        contents.len(),
        1,
        "contents must hold exactly one entry: {body}"
    );
    let entry = &contents[0];
    assert_eq!(entry["uri"], uri);
    assert_eq!(entry["mimeType"], "text/markdown");
    let text = entry["text"]
        .as_str()
        .unwrap_or_else(|| panic!("entry missing text: {entry}"));

    // Expected: byte-identical to the source file. The build-time
    // projection stores the full markdown verbatim.
    let expected = skills::ALL_SKILLS
        .iter()
        .find(|s| s.name == "mnemonik-attest")
        .expect("mnemonik-attest manifest registered")
        .full_markdown;
    assert_eq!(
        text, expected,
        "resources/read body must match the compiled manifest byte-for-byte"
    );
}

#[tokio::test]
async fn tools_list_enriched_with_purpose_trigger() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, body) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "tools/list", "id": 4}),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "anon tools/list must be 200: {body}"
    );
    let tools = body["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("missing tools array: {body}"));

    // Each of the six manifest-backed tools must surface BOTH the
    // `Purpose:` and `Trigger:` labels. The 7th tool
    // (`request_public_write_confirmation`) is gate infrastructure with
    // no manifest, so it is exempt.
    let manifest_backed = [
        "mnemonic_whoami",
        "mnemonic_sign_memory",
        "mnemonic_verify",
        "mnemonic_prove_identity",
        "mnemonic_recall",
        "mnemonic_check_pending",
    ];
    for name in manifest_backed {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} missing from tools/list: {body}"));
        let desc = tool["description"]
            .as_str()
            .unwrap_or_else(|| panic!("tool {name} missing description: {tool}"));
        assert!(
            desc.contains("Purpose:"),
            "tool {name} description missing `Purpose:` enrichment: {desc}"
        );
        assert!(
            desc.contains("Trigger:"),
            "tool {name} description missing `Trigger:` enrichment: {desc}"
        );
    }
}

#[tokio::test]
async fn initialize_surfaces_embedder_metadata() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, body) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "initialize", "id": 5}),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "initialize must be 200: {body}");
    let embedder = &body["result"]["embedder"];
    let model_id = embedder["model_id"]
        .as_str()
        .unwrap_or_else(|| panic!("embedder.model_id missing/non-string: {body}"));
    assert!(!model_id.is_empty(), "embedder.model_id must be non-empty");
    let model_version = embedder["model_version"]
        .as_str()
        .unwrap_or_else(|| panic!("embedder.model_version missing/non-string: {body}"));
    assert!(
        !model_version.is_empty(),
        "embedder.model_version must be non-empty"
    );
    let dim = embedder["dim"]
        .as_u64()
        .unwrap_or_else(|| panic!("embedder.dim missing/non-integer: {body}"));
    assert!(dim > 0, "embedder.dim must be a positive integer");

    // capabilities.prompts + capabilities.resources advertised so clients
    // know they can call the four new discovery methods.
    let caps = &body["result"]["capabilities"];
    assert!(
        caps["prompts"].is_object(),
        "capabilities.prompts must be an object: {body}"
    );
    assert!(
        caps["resources"].is_object(),
        "capabilities.resources must be an object: {body}"
    );
}

#[tokio::test]
async fn prompts_list_with_invalid_bearer_passes_through() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    // Garbage JWT — must not turn an allowlisted call into a 401. Per
    // `oauth/mod.rs` lines 1378-1384, allowlisted methods treat a bad
    // Bearer as if it were absent (no Claims attached, request proceeds).
    let (status, body) = post_json(
        &app,
        serde_json::json!({"jsonrpc": "2.0", "method": "prompts/list", "id": 6}),
        Some("not-a-real-jwt"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "allowlisted prompts/list must ignore bad bearer (got {status}): {body}"
    );
    assert!(
        body["result"]["prompts"].is_array(),
        "response must still carry prompts array: {body}"
    );
}

/// Round-1 code-review CR2-02: cover the unknown-name branch of
/// `prompts_get_payload` so the typed `-32602 InvalidParams` envelope
/// — `data.field == "name"`, `data.received` echoing the raw input —
/// is locked down against a refactor that swaps the error code.
#[tokio::test]
async fn prompts_get_unknown_name_returns_invalid_params() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, body) = post_json(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "prompts/get",
            "id": 7,
            "params": {"name": "mnemonik-does-not-exist"},
        }),
        None,
    )
    .await;

    // JSON-RPC errors travel inside a 200 envelope on streamable-HTTP;
    // only transport-level failures bump the HTTP status.
    assert_eq!(
        status,
        StatusCode::OK,
        "JSON-RPC error must travel inside a 200 envelope: {body}"
    );
    let err = body["error"]
        .as_object()
        .unwrap_or_else(|| panic!("expected error envelope: {body}"));
    assert_eq!(err["code"], -32602, "must be -32602 InvalidParams: {body}");
    let data = err["data"]
        .as_object()
        .unwrap_or_else(|| panic!("typed error must carry data: {body}"));
    assert_eq!(data["field"], "name");
    assert_eq!(data["received"], "mnemonik-does-not-exist");
}

/// Round-1 code-review CR2-02: cover the bad-URI branch of
/// `resources_read_payload`. Wrong scheme — `https://` instead of
/// `mnemonik://skills/` — must surface `-32602 InvalidParams` with
/// `data.field == "uri"` so clients distinguish "I sent a bad URI"
/// from "the server doesn't know that resource".
#[tokio::test]
async fn resources_read_bad_uri_scheme_returns_invalid_params() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let bad_uri = "https://invalid-scheme/foo.md";
    let (status, body) = post_json(
        &app,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "resources/read",
            "id": 8,
            "params": {"uri": bad_uri},
        }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let err = body["error"]
        .as_object()
        .unwrap_or_else(|| panic!("expected error envelope: {body}"));
    assert_eq!(err["code"], -32602);
    let data = err["data"]
        .as_object()
        .unwrap_or_else(|| panic!("typed error must carry data: {body}"));
    assert_eq!(data["field"], "uri");
    assert_eq!(data["received"], bad_uri);
}

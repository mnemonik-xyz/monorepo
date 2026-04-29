//! Integration test: production `/api/cli-bootstrap/*` handlers behind the
//! real `bearer_auth_middleware` allowlist (Round-2 fix for Task 6).
//!
//! Why a separate file? The unit tests in `mcp/src/api.rs::tests` re-implement
//! the redeem handler via a closure routed through a bare `Router` with no
//! middleware. So a regression that drops `/api/cli-bootstrap/redeem/` from
//! `bearer_auth_middleware`'s URI allowlist would silently break the "no auth
//! required" contract without failing any existing test.
//!
//! This file boots the production handlers from `mnemonic_mcp::api` under a
//! Router layered with the production `bearer_auth_middleware` (model:
//! `mcp/tests/auth_allowlist.rs`) and asserts:
//!
//!   1. `POST /api/cli-bootstrap/issue` with valid Bearer JWT → 200 + ticket_id
//!   2. `GET /api/cli-bootstrap/redeem/:ticket` with NO Authorization → 200
//!   3. Second redeem of the same ticket → 404 (single-use)
//!   4. `GET /api/cli-bootstrap/redeem/<garbage-uuid>` with NO auth → 404
//!      (NOT 401 — proves the URI allowlist is honored; bearer middleware
//!      did not short-circuit before reaching the handler)
//!   5. `POST /api/cli-bootstrap/issue` with NO Bearer → 401 (middleware bites)
//!   6. POST four tickets for the same `jwt.sub` → the 4th returns 429
//!      (production HTTP mapping of `BootstrapInsertError::PerUserCapExceeded`)

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_mcp::{
    api::{bootstrap_issue_handler, bootstrap_redeem_handler, BootstrapTickets},
    mcp::McpState,
    oauth::{self, OAuthState},
    test_support::{mint_jwt, mock_state},
};
use serde_json::Value;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"cli-bootstrap-secret-32-bytes-!!";

fn build_router(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/api/cli-bootstrap/issue", post(bootstrap_issue_handler))
        .route(
            "/api/cli-bootstrap/redeem/{ticket}",
            get(bootstrap_redeem_handler),
        )
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

/// Build a minimal, valid Solana keypair JSON: 64 bytes where bytes[32..64]
/// are arbitrary but non-zero so `pubkey_base58.len() == 44` (a real base58
/// pubkey is 32–44 chars; the all-zero case collapses to 32 chars and would
/// not exercise the derive path strictly). The unit test in api.rs already
/// covers the all-zero edge; here we use a non-zero suffix.
fn fake_keypair_json() -> String {
    let mut bytes = vec![0u8; 64];
    // Distinct trailing bytes so the derived pubkey is non-trivial.
    for (i, b) in bytes[32..64].iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(1);
    }
    serde_json::to_string(&bytes).unwrap()
}

async fn post_issue(app: &Router, body: Value, auth: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/cli-bootstrap/issue")
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
    let parsed: Value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::new()));
    (status, parsed)
}

async fn get_redeem(app: &Router, ticket: &str, auth: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("GET")
        .uri(format!("/api/cli-bootstrap/redeem/{ticket}"));
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req.body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::new()));
    (status, parsed)
}

#[tokio::test]
async fn test_issue_and_redeem_through_real_middleware() {
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let app = build_router(state, oauth_state);

    // 1. Issue with a valid Bearer JWT — passes middleware, 200 + ticket_id.
    let token = mint_jwt("alice-pubkey-base58", TEST_SECRET);
    let kp_json = fake_keypair_json();
    let (s_issue, body_issue) = post_issue(
        &app,
        serde_json::json!({"keypair_json": kp_json}),
        Some(&token),
    )
    .await;
    assert_eq!(
        s_issue,
        StatusCode::OK,
        "valid-JWT issue must succeed: {body_issue}"
    );
    let ticket_id = body_issue["ticket_id"]
        .as_str()
        .expect("ticket_id missing")
        .to_string();
    // UUIDv4 string shape (8-4-4-4-12 = 36 chars).
    assert_eq!(ticket_id.len(), 36, "ticket_id must be UUID: {ticket_id}");

    // 2. Redeem with NO Authorization — middleware allowlist must let it
    //    through to the handler, which returns 200 with secret + pubkey.
    let (s_redeem, body_redeem) = get_redeem(&app, &ticket_id, None).await;
    assert_eq!(
        s_redeem,
        StatusCode::OK,
        "anon redeem must succeed (allowlist): {body_redeem}"
    );
    let secret = body_redeem["secret"].as_array().expect("secret missing");
    assert_eq!(secret.len(), 64, "secret must be 64 bytes");
    assert!(
        body_redeem["pubkey_base58"]
            .as_str()
            .map(|s| s.len() >= 32)
            .unwrap_or(false),
        "pubkey_base58 missing or too short: {body_redeem}"
    );

    // 3. Second redeem of the same ticket — single-use, 404.
    let (s_again, body_again) = get_redeem(&app, &ticket_id, None).await;
    assert_eq!(
        s_again,
        StatusCode::NOT_FOUND,
        "second redeem must be 404 (single-use): {body_again}"
    );
}

#[tokio::test]
async fn test_redeem_garbage_uuid_no_auth_is_404_not_401() {
    // Litmus check: a developer who tightens the allowlist by removing
    // `/api/cli-bootstrap/redeem/` would cause this test to flip from 404 to
    // 401, immediately surfacing the regression. The previous closure-based
    // test could not catch this because the closure was routed under a bare
    // Router with no middleware.
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, _body) = get_redeem(&app, "not-a-uuid-at-all", None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "garbage UUID must be 404 (allowlist passed → handler), NOT 401 (middleware bit)"
    );
}

#[tokio::test]
async fn test_issue_without_bearer_is_401() {
    // Production middleware must reject /issue with no Authorization header.
    // /issue is NOT on the allowlist; `extract_json_rpc_method` on a non-
    // JSON-RPC body returns None → not allowlisted → 401 from middleware.
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let (status, body) = post_issue(
        &app,
        serde_json::json!({"keypair_json": fake_keypair_json()}),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anon issue must be 401: {body}"
    );
    // Middleware uses the JSON-RPC -32001 envelope shape (see
    // `oauth::jsonrpc_unauthorized`). We don't pin the exact body further —
    // status code is the load-bearing assertion.
}

#[tokio::test]
async fn test_per_user_cap_returns_429_at_handler() {
    // Mint 4 tokens with the SAME `sub` and POST 4 issue calls. The first
    // three must be 200; the fourth must be 429 (per-user cap = 3 in
    // BootstrapTickets::with_defaults). This pins the production handler's
    // `BootstrapInsertError::PerUserCapExceeded → StatusCode::TOO_MANY_REQUESTS`
    // mapping that was previously only covered at the unit-store level.
    let state = mock_state();
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let app = build_router(state, oauth_state);

    let kp_json = fake_keypair_json();
    let body = serde_json::json!({"keypair_json": kp_json});

    // Three successful issues for the same sub (each call mints a fresh JWT
    // with a fresh `jti`, but `sub` is constant — the per-user counter keys
    // off `sub`).
    for i in 0..3 {
        let token = mint_jwt("rate-limited-user", TEST_SECRET);
        let (status, b) = post_issue(&app, body.clone(), Some(&token)).await;
        assert_eq!(status, StatusCode::OK, "issue #{i} must succeed: {b}");
        assert!(
            b["ticket_id"].is_string(),
            "issue #{i} must return ticket_id: {b}"
        );
    }

    // Fourth issue → 429 (per-user cap exceeded).
    let token = mint_jwt("rate-limited-user", TEST_SECRET);
    let (status, body4) = post_issue(&app, body.clone(), Some(&token)).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "4th issue must be 429: {body4}"
    );
    // Sanity — error body shape is JSON with an `error` field.
    assert!(
        body4["error"].is_string(),
        "429 body must have `error` field: {body4}"
    );

    // Different user is unaffected — proves the cap is per-`sub`, not global.
    let other_token = mint_jwt("different-user", TEST_SECRET);
    let (status_other, _) = post_issue(&app, body, Some(&other_token)).await;
    assert_eq!(
        status_other,
        StatusCode::OK,
        "different sub must NOT be rate-limited"
    );
}

/// Sanity guard: `BootstrapTickets::with_defaults` actually exists and is
/// reachable through the public surface tests rely on. Pure compile-check
/// (no runtime work). Would catch a refactor that hides the type.
#[tokio::test]
async fn test_bootstrap_tickets_default_constructible() {
    let _t: Arc<BootstrapTickets> = Arc::new(BootstrapTickets::with_defaults());
}

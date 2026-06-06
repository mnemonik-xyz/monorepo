//! Smoke tests for the `_helpers` test harness — Task 4 contract.
//!
//! These tests live in a dedicated `tests/` target rather than being inlined
//! into `_helpers/mod.rs` because `mod _helpers;` is included by every
//! integration test in this crate; sticking `#[tokio::test]` functions into
//! the shared module would compile (and run) them once per consumer binary.
//!
//! Coverage:
//!
//! 1. `_helpers_test_server_mounts_oauth_token` — `with_oauth_token(true)`
//!    mounts BOTH `/oauth/token` (POST) AND `/oauth/authorize` (GET) on the
//!    same tower-stack as `/mcp`. Asserts each returns a NON-404 status when
//!    probed; the actual 4xx body is irrelevant — what matters is that the
//!    route exists. Without the flag, both routes return 404 (regression
//!    guard for Task 5 AC9 `test_anonymous_recall_unchanged`, which depends
//!    on the unmounted default).
//!
//! 2. `_helpers_bootstrap_oauth_returns_usable_code` — `bootstrap_oauth`
//!    returns a triple `(code, verifier, redirect_uri)` such that POSTing
//!    it to `/oauth/token` as `authorization_code` yields a 200 with
//!    non-empty `access_token` and `refresh_token`. Production-flow
//!    coverage: the helper drives `/oauth/authorize` (GET + POST), no
//!    direct SQL.
//!
//! 3. `_helpers_rotate_returns_pair_both_strings_non_empty` — after
//!    bootstrap + initial token redemption, `rotate(&server, &refresh1)`
//!    returns a `(access2, refresh2)` pair where both are non-empty and
//!    `refresh2 != refresh1`. Closes the Task 4 round-2 test-reviewer
//!    finding that `rotate` previously returned only the refresh token.
//!
//! 4. `_helpers_insert_expired_refresh_returns_plaintext_and_family_id` —
//!    `insert_expired_refresh_for_test` returns `(plaintext, family_id)`
//!    where the plaintext is a 32-byte URL-safe-base64 (43-character)
//!    string, the family_id is a valid UUID, and the inserted row has the
//!    expected `expires_at < now`, `revoked = 0`, and `token_hash`
//!    matches `refresh::hash_refresh_token(salt, plaintext)`.
//!
//! 5. `_helpers_family_has_unrevoked_rows` — after
//!    `insert_expired_refresh_for_test`, the helper returns `true`; after
//!    flipping `revoked = 1` directly in SQL it returns `false`; after
//!    flipping back it returns `true` again. Confirms the boolean
//!    branch behaves on the round-trip.

#![allow(clippy::bool_assert_comparison)]

#[path = "_helpers/mod.rs"]
mod _helpers;

use _helpers::{
    bootstrap_oauth, family_has_unrevoked_rows, insert_expired_refresh_for_test, rotate,
    TestServer, TEST_REDIRECT_URI,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use mnemonic_mcp::oauth::refresh;
use serde_json::Value;
use tower::ServiceExt;

/// AC9 regression guard + Verify-smoke. Mounts BOTH /oauth/token and
/// /oauth/authorize when the flag is set; returns 404 for both without.
#[tokio::test]
async fn _helpers_test_server_mounts_oauth_token() {
    // Positive case — flag set, both routes mounted.
    let server = TestServer::builder().with_oauth_token(true).build();

    let token_req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(
            br#"{"this":"is intentionally invalid"}"#.to_vec(),
        ))
        .unwrap();
    let token_resp = server.app.clone().oneshot(token_req).await.unwrap();
    assert_ne!(
        token_resp.status(),
        StatusCode::NOT_FOUND,
        "POST /oauth/token must be mounted when with_oauth_token(true); got 404 — route missing"
    );

    let authz_req = Request::builder()
        .method("GET")
        .uri("/oauth/authorize?client_id=x&redirect_uri=y&code_challenge=z&code_challenge_method=S256&state=s")
        .body(Body::empty())
        .unwrap();
    let authz_resp = server.app.clone().oneshot(authz_req).await.unwrap();
    assert_ne!(
        authz_resp.status(),
        StatusCode::NOT_FOUND,
        "GET /oauth/authorize must be mounted when with_oauth_token(true); got 404 — route missing"
    );

    // Negative case — default builder, no flag, neither route mounted.
    let server_no_oauth = TestServer::builder().build();
    let token_req2 = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(br#"{"any":"body"}"#.to_vec()))
        .unwrap();
    let token_resp2 = server_no_oauth
        .app
        .clone()
        .oneshot(token_req2)
        .await
        .unwrap();
    assert_eq!(
        token_resp2.status(),
        StatusCode::NOT_FOUND,
        "POST /oauth/token MUST 404 without with_oauth_token — regression guard for Task 5 AC9"
    );

    let authz_req2 = Request::builder()
        .method("GET")
        .uri("/oauth/authorize?client_id=x&redirect_uri=y&code_challenge=z&code_challenge_method=S256&state=s")
        .body(Body::empty())
        .unwrap();
    let authz_resp2 = server_no_oauth
        .app
        .clone()
        .oneshot(authz_req2)
        .await
        .unwrap();
    assert_eq!(
        authz_resp2.status(),
        StatusCode::NOT_FOUND,
        "GET /oauth/authorize MUST 404 without with_oauth_token — regression guard"
    );
}

/// `bootstrap_oauth` returns a triple that successfully drives the first
/// `/oauth/token` exchange — production parity, no SQL shortcuts.
#[tokio::test]
async fn _helpers_bootstrap_oauth_returns_usable_code() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (code, verifier, redirect_uri) = bootstrap_oauth(&server).await;

    assert_eq!(redirect_uri, TEST_REDIRECT_URI);
    assert!(
        !code.is_empty(),
        "bootstrap_oauth must return non-empty code"
    );
    assert!(
        !verifier.is_empty(),
        "bootstrap_oauth must return non-empty verifier"
    );

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "code_verifier": verifier,
        "redirect_uri": redirect_uri,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = server.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "authorization_code grant should succeed; body={}",
        String::from_utf8_lossy(&bytes)
    );
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        v["access_token"].as_str().is_some_and(|s| !s.is_empty()),
        "TokenResponse must carry non-empty access_token"
    );
    assert!(
        v["refresh_token"].as_str().is_some_and(|s| !s.is_empty()),
        "TokenResponse must carry non-empty refresh_token (Task 3 wired emission)"
    );
}

/// `rotate` returns both `(access, refresh)` and the new refresh differs
/// from the old. Pins the round-2 test-reviewer contract for AC12.
#[tokio::test]
async fn _helpers_rotate_returns_pair_both_strings_non_empty() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (code, verifier, redirect_uri) = bootstrap_oauth(&server).await;

    // First exchange — authorization_code grant.
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "code_verifier": verifier,
        "redirect_uri": redirect_uri,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = server.app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let refresh1 = v["refresh_token"].as_str().unwrap().to_string();
    assert!(!refresh1.is_empty());

    // Now rotate.
    let (access2, refresh2) = rotate(&server, &refresh1).await;
    assert!(!access2.is_empty(), "rotate must return non-empty access");
    assert!(!refresh2.is_empty(), "rotate must return non-empty refresh");
    assert_ne!(
        refresh1, refresh2,
        "Branch A rotation must mint a fresh refresh plaintext distinct from the seed"
    );
}

/// `insert_expired_refresh_for_test` returns the right shape AND inserts a
/// matching row in `refresh_tokens` whose hash recomputes from `(salt,
/// plaintext)`.
#[tokio::test]
async fn _helpers_insert_expired_refresh_returns_plaintext_and_family_id() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (plaintext, family_id) = insert_expired_refresh_for_test(&server).await;

    // 32 bytes → URL-safe-base64 no-pad → 43 characters (ceil(32 * 8 / 6)).
    assert_eq!(
        plaintext.len(),
        43,
        "plaintext must be 32-byte URL-safe-base64 no-pad (43 chars)"
    );
    // Uuid v4 string form: 8-4-4-4-12 = 36 chars including dashes.
    assert_eq!(
        family_id.len(),
        36,
        "family_id must be UUID-form (36 chars)"
    );
    let uuid_parsed = uuid::Uuid::parse_str(&family_id);
    assert!(
        uuid_parsed.is_ok(),
        "family_id must parse as a valid UUID; got {family_id:?}"
    );

    // Hash recompute matches what we just stored.
    let expected_hash = refresh::hash_refresh_token(&server.oauth_state.refresh_salt, &plaintext);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let conn = server.oauth_state.refresh_store.lock().unwrap();
    let (token_hash, expires_at, revoked): (String, u64, i64) = conn
        .query_row(
            "SELECT token_hash, expires_at, revoked
               FROM refresh_tokens WHERE family_id = ?1",
            rusqlite::params![family_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("row with this family_id exists");
    assert_eq!(
        token_hash, expected_hash,
        "stored token_hash matches recompute"
    );
    assert!(
        expires_at < now,
        "expires_at must be in the past; got expires_at={expires_at} now={now}"
    );
    assert_eq!(revoked, 0, "row inserted with revoked=0");
}

/// `family_has_unrevoked_rows` flips on direct SQL updates to `revoked`.
#[tokio::test]
async fn _helpers_family_has_unrevoked_rows() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_plaintext, family_id) = insert_expired_refresh_for_test(&server).await;

    assert!(
        family_has_unrevoked_rows(&server, &family_id),
        "fresh insert must register as unrevoked"
    );

    // Flip revoked = 1 directly via SQL.
    {
        let conn = server.oauth_state.refresh_store.lock().unwrap();
        conn.execute(
            "UPDATE refresh_tokens SET revoked = 1 WHERE family_id = ?1",
            rusqlite::params![family_id],
        )
        .expect("UPDATE revoked=1");
    }
    assert!(
        !family_has_unrevoked_rows(&server, &family_id),
        "after revoke, helper must return false"
    );

    // Flip back.
    {
        let conn = server.oauth_state.refresh_store.lock().unwrap();
        conn.execute(
            "UPDATE refresh_tokens SET revoked = 0 WHERE family_id = ?1",
            rusqlite::params![family_id],
        )
        .expect("UPDATE revoked=0");
    }
    assert!(
        family_has_unrevoked_rows(&server, &family_id),
        "after un-revoke, helper must return true again"
    );

    // Unknown family → false.
    assert!(
        !family_has_unrevoked_rows(&server, "never-inserted-family"),
        "unknown family must return false (empty count)"
    );
}

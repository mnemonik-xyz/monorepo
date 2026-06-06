//! Integration test suite — OAuth 2.1 refresh-token rotation (Task 5).
//!
//! 14 `#[tokio::test]` functions: one per AC1–AC13 plus a single
//! `cross_table_mutex_no_starvation` cross-cutting test that proves the
//! per-table mutexes do not starve either side under interleaved
//! contention.
//!
//! Every test builds the server via
//! `TestServer::builder().with_oauth_token(true).with_reuse_interval(
//! Duration::from_millis(100)).build()` so the whole suite finishes
//! around 2 s wall-clock under default `cargo test` parallelism (under
//! 2 s when `--test-threads >= 8`); AC3 sleeps 150 ms (just above the
//! configured 100 ms reuse window) and AC4 sleeps 1100 ms (necessary to
//! cross the wall-clock second boundary because the DB-layer
//! inside-window check rounds `Duration::as_secs()` to 0 for a 100 ms
//! reuse_interval — see [`POST_FAMILY_REVOKE_SLEEP_MS`] and the in-crate
//! `refresh.rs` unit test that uses the same workaround). The 6 s draft
//! sketched in `code-research.md` §I.9 is NOT used.
//!
//! Helper signatures consumed from `_helpers` (sealed by Task 4):
//!
//! - `bootstrap_oauth(&server) -> (code, verifier, redirect_uri)` —
//!   drives the real production challenge-sign-redeem path and returns
//!   the redeemable triple. `client_id = "test-client"` is embedded by
//!   the helper; direct `/oauth/token` POSTs in this file echo the same
//!   value (Task 4 deviation 1 in `decisions.md`).
//! - `rotate(&server, refresh) -> (access_token, refresh_token)` —
//!   happy-path JSON-encoded `grant_type=refresh_token`. AC10's
//!   form-encoded branch is inlined in the test body because the helper
//!   is JSON-only (Task 4 deviation 2).
//! - `insert_expired_refresh_for_test(&server) -> (plaintext, family_id)`
//!   — synthesises a refresh-tokens row with `expires_at = now - 60` so
//!   AC5 exercises Branch D without waiting a year.
//! - `family_has_unrevoked_rows(&server, &family_id) -> bool` — direct
//!   SQL probe used by AC5 to assert Branch D did NOT revoke the family.
//!
//! Scope-down: R6 (DB-write-failure retry-safety) is intentionally NOT
//! exercised here — no `McpState::refresh_store_inject_failure_once()`
//! hook exists and this task is forbidden from modifying production
//! code to add one. R6 risk is delegated to Task 6 code-audit.

#![allow(clippy::bool_assert_comparison)]

mod _helpers;

use std::sync::Arc;
use std::time::Duration;

use _helpers::{
    bootstrap_oauth, family_has_unrevoked_rows, insert_expired_refresh_for_test, rotate, TestServer,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures::future::join_all;
use http_body_util::BodyExt;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use serde_json::Value;
use tower::ServiceExt;

/// 100 ms reuse-interval — keeps the in-process `ReuseCache` TTL
/// short so AC3's post-window cache-miss sleep is sub-second. Mirrors
/// the `with_reuse_interval` value the builder is configured with so
/// the cache TTL and the assertion sleeps stay internally consistent.
const REUSE_INTERVAL_MS: u64 = 100;

/// Sleep duration for AC3 — strictly above [`REUSE_INTERVAL_MS`] so the
/// cached `(access, refresh)` entry has expired by the time the second
/// POST lands. AC3 only asserts the response is 400 / `invalid_grant`,
/// which Branch B' (cache miss, same wall-clock second) already
/// produces — so a sub-second sleep is sufficient.
const POST_REUSE_SLEEP_MS: u64 = 150;

/// Sleep duration for AC4 — strictly above the **DB-layer**
/// inside-window check (which calls `reuse_interval.as_secs()`,
/// truncating sub-second values to 0). The task spec sketched 150 ms
/// here, but the Task 1 implementation enforces inside-window with
/// 1-second granularity via `as_secs()` (see
/// `mcp/src/oauth/refresh.rs:689` and the in-crate test at line 917,
/// which deliberately uses a 2 s reuse_interval for the same reason).
/// Crossing the wall-clock-second boundary is mandatory to land on
/// Branch C (family revoke) rather than Branch B' (fail-closed); both
/// surface as 400 / `invalid_grant`, but only Branch C revokes the
/// family — which AC4's second assertion (live descendant r2 rejects)
/// requires. 1100 ms guarantees a unix-second tick on cgroup-throttled
/// CI runners.
const POST_FAMILY_REVOKE_SLEEP_MS: u64 = 1100;

// ── shared utilities ───────────────────────────────────────────────────────

fn build_server() -> TestServer {
    TestServer::builder()
        .with_oauth_token(true)
        .with_reuse_interval(Duration::from_millis(REUSE_INTERVAL_MS))
        .build()
}

/// Drive a full `authorization_code` exchange and return the JSON body
/// of the `/oauth/token` response. Used as the seed for every test that
/// needs a fresh `(access, refresh)` pair without re-implementing the
/// PKCE handshake.
async fn fresh_authcode_pair(server: &TestServer) -> Value {
    let (code, verifier, redirect_uri) = bootstrap_oauth(server).await;
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "code_verifier": verifier,
        "redirect_uri": redirect_uri,
        "client_id": "test-client",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .expect("build POST /oauth/token authorization_code");
    let resp = server
        .app
        .clone()
        .oneshot(req)
        .await
        .expect("oneshot /oauth/token");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "authorization_code redemption must return 200"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("TokenResponse JSON")
}

/// Send a raw POST to `/oauth/token` with the given body bytes and
/// content-type. Returns `(status, parsed_body)`. Used by tests that
/// need to exercise an error path or a non-JSON content-type the
/// `rotate` helper does not cover.
async fn post_token_raw(
    server: &TestServer,
    content_type: &str,
    body_bytes: Vec<u8>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", content_type)
        .body(Body::from(body_bytes))
        .expect("build POST /oauth/token raw");
    let resp = server
        .app
        .clone()
        .oneshot(req)
        .await
        .expect("oneshot raw POST /oauth/token");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, parsed)
}

/// SELECT the `refresh_tokens.expires_at` for a given plaintext. Used by
/// AC6 to assert the rolling TTL extends across a rotation.
fn db_expires_at_for_plaintext(server: &TestServer, plaintext: &str) -> u64 {
    let token_hash = mnemonic_mcp::oauth::refresh::hash_refresh_token(
        &server.oauth_state.refresh_salt,
        plaintext,
    );
    let guard = server
        .oauth_state
        .refresh_store
        .lock()
        .expect("refresh_store mutex");
    guard
        .query_row(
            "SELECT expires_at FROM refresh_tokens WHERE token_hash = ?1",
            rusqlite::params![token_hash],
            |row| row.get::<_, u64>(0),
        )
        .expect("expires_at row")
}

/// Assert that two `TokenResponse`-shaped JSON bodies are byte-identical on
/// every field except `access_token` (which differs across independent
/// rotations because each JWT carries a fresh `jti` + `iat`). Used by
/// AC10 — content-type parity — to confirm the form-encoded and JSON
/// branches produce structurally equal envelopes.
fn assert_structural_eq(form_body: &Value, json_body: &Value) {
    let form_obj = form_body.as_object().expect("form body is an object");
    let json_obj = json_body.as_object().expect("json body is an object");
    let form_keys: std::collections::BTreeSet<&String> = form_obj.keys().collect();
    let json_keys: std::collections::BTreeSet<&String> = json_obj.keys().collect();
    assert_eq!(
        form_keys, json_keys,
        "structural equality: same field set across content-types; form={form_body} json={json_body}"
    );
    for (k, v) in form_obj.iter() {
        if k == "access_token" {
            continue;
        }
        if k == "refresh_token" {
            let form_rt = v.as_str().expect("form refresh_token is a string");
            let json_rt = json_obj["refresh_token"]
                .as_str()
                .expect("json refresh_token is a string");
            assert_eq!(
                form_rt.len(),
                43,
                "form-encoded refresh_token must be 43-char URL-safe-base64 (32 bytes); got {form_rt:?}"
            );
            assert_eq!(
                json_rt.len(),
                43,
                "json-encoded refresh_token must be 43-char URL-safe-base64 (32 bytes); got {json_rt:?}"
            );
            continue;
        }
        assert_eq!(
            v, &json_obj[k],
            "structural equality on field {k:?}: form={v} json={}",
            json_obj[k]
        );
    }
    let form_at = form_obj["access_token"]
        .as_str()
        .expect("form access_token is a string");
    let json_at = json_obj["access_token"]
        .as_str()
        .expect("json access_token is a string");
    assert_eq!(
        form_at.split('.').count(),
        3,
        "form access_token must be a three-segment JWT; got {form_at:?}"
    );
    assert_eq!(
        json_at.split('.').count(),
        3,
        "json access_token must be a three-segment JWT; got {json_at:?}"
    );
}

// ── AC1 — authorization_code grant emits a refresh_token ──────────────────

#[tokio::test]
async fn test_authcode_grant_returns_refresh() {
    let server = build_server();
    let tok = fresh_authcode_pair(&server).await;
    let refresh = tok["refresh_token"]
        .as_str()
        .expect("AC1: authorization_code response must carry a refresh_token field");
    assert!(
        !refresh.is_empty(),
        "AC1: refresh_token must be non-empty; got {tok}"
    );
    assert_eq!(
        refresh.len(),
        43,
        "AC1: refresh_token plaintext is 32-byte URL-safe-base64 no-pad (43 chars); got {refresh:?}"
    );
}

// ── AC2 — refresh-grant returns a new (access, refresh) pair ──────────────

#[tokio::test]
async fn test_refresh_grant_returns_new_pair() {
    let server = build_server();
    let seed = fresh_authcode_pair(&server).await;
    let access1 = seed["access_token"].as_str().unwrap().to_string();
    let refresh1 = seed["refresh_token"].as_str().unwrap().to_string();

    let (access2, refresh2) = rotate(&server, &refresh1).await;

    assert_ne!(
        access1, access2,
        "AC2: refresh-grant must mint a fresh access_token (different jti/iat)"
    );
    assert_ne!(
        refresh1, refresh2,
        "AC2: refresh-grant must mint a fresh refresh_token plaintext"
    );
    assert!(!access2.is_empty(), "AC2: new access_token is non-empty");
    assert!(!refresh2.is_empty(), "AC2: new refresh_token is non-empty");
}

// ── AC3 — old refresh outside reuse-window → invalid_grant ────────────────

#[tokio::test]
async fn test_old_refresh_outside_reuse_interval_rejected() {
    let server = build_server();
    let seed = fresh_authcode_pair(&server).await;
    let refresh1 = seed["refresh_token"].as_str().unwrap().to_string();

    // First rotation succeeds — refresh1 is now revoked, cache holds the pair
    // for 100 ms. Wait > 100 ms to drift past the reuse window so the
    // second attempt with refresh1 hits Branch C (replay outside window).
    let _new_pair = rotate(&server, &refresh1).await;
    tokio::time::sleep(Duration::from_millis(POST_REUSE_SLEEP_MS)).await;

    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh1,
    });
    let (status, parsed) = post_token_raw(
        &server,
        "application/json",
        serde_json::to_vec(&body).unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "AC3: outside-reuse replay must return 400; got body={parsed}"
    );
    assert_eq!(
        parsed["error"], "invalid_grant",
        "AC3: error code must be invalid_grant; got body={parsed}"
    );
}

// ── AC4 — outside-window replay revokes the whole family ──────────────────

#[tokio::test]
async fn test_replay_outside_reuse_revokes_family() {
    let server = build_server();
    let seed = fresh_authcode_pair(&server).await;
    let r1 = seed["refresh_token"].as_str().unwrap().to_string();

    // Branch A — legitimate rotation; r2 is now the live descendant.
    let (_a2, r2) = rotate(&server, &r1).await;

    // Drift past the DB-layer inside-window check (1-second granularity
    // via `reuse_interval.as_secs()`) so the next replay of r1 lands on
    // Branch C (family revoke) rather than Branch B' (fail-closed). See
    // [`POST_FAMILY_REVOKE_SLEEP_MS`] for the full rationale.
    tokio::time::sleep(Duration::from_millis(POST_FAMILY_REVOKE_SLEEP_MS)).await;

    // Replay r1 → 400 invalid_grant AND family revoke.
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": r1,
    });
    let (status, parsed) = post_token_raw(
        &server,
        "application/json",
        serde_json::to_vec(&body).unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "AC4: replay outside window must be 400; body={parsed}"
    );
    assert_eq!(parsed["error"], "invalid_grant");

    // Live descendant r2 must now also reject — family is revoked.
    let body2 = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": r2,
    });
    let (status2, parsed2) = post_token_raw(
        &server,
        "application/json",
        serde_json::to_vec(&body2).unwrap(),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::BAD_REQUEST,
        "AC4: live descendant r2 must reject after family revoke; body={parsed2}"
    );
    assert_eq!(parsed2["error"], "invalid_grant");
}

// ── AC5 — expired refresh → invalid_grant, family NOT revoked ─────────────

#[tokio::test]
async fn test_expired_refresh_rejected_no_family_revoke() {
    let server = build_server();
    let (plaintext, family_id) = insert_expired_refresh_for_test(&server).await;

    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": plaintext,
    });
    let (status, parsed) = post_token_raw(
        &server,
        "application/json",
        serde_json::to_vec(&body).unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "AC5: expired refresh must be 400; body={parsed}"
    );
    assert_eq!(parsed["error"], "invalid_grant");

    // Branch D: expired ≠ replay attack. The family must still carry
    // unrevoked rows — verified via the synchronous helper that probes
    // `refresh_tokens.family_id = ? AND revoked = 0`.
    assert!(
        family_has_unrevoked_rows(&server, &family_id),
        "AC5: Branch D must NOT revoke the family; family_id={family_id}"
    );
}

// ── AC6 — rolling expires_at extends on every rotation ────────────────────

#[tokio::test]
async fn test_rolling_expires_at_extended() {
    let server = build_server();
    let seed = fresh_authcode_pair(&server).await;
    let r1 = seed["refresh_token"].as_str().unwrap().to_string();

    let expires_before = db_expires_at_for_plaintext(&server, &r1);

    // The DB stores `expires_at` as unix-seconds. The mint-time
    // `expires_at = now + 1y` and the post-rotation `expires_at = new_now + 1y`
    // — both computed from `SystemTime::now()` at distinct instants. Sleep
    // at least 1 s so the integer-second comparison sees the extension.
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let (_, r2) = rotate(&server, &r1).await;
    let expires_after = db_expires_at_for_plaintext(&server, &r2);

    assert!(
        expires_after > expires_before,
        "AC6: rolling TTL must extend strictly; before={expires_before} after={expires_after}"
    );
}

// ── AC7 — discovery advertises both grant types ───────────────────────────

#[tokio::test]
async fn test_discovery_advertises_refresh_grant() {
    // The discovery handler is pure — does not need an HTTP route mounted
    // on the test router. Call it directly through axum's response
    // primitive and inspect the JSON body.
    use axum::response::IntoResponse;

    let response = mnemonic_mcp::oauth::oauth_authorization_server_metadata().await;
    let resp = response.into_response();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).expect("discovery JSON");

    let grant_types = v["grant_types_supported"]
        .as_array()
        .expect("AC7: discovery must carry grant_types_supported[]");
    let names: Vec<&str> = grant_types.iter().filter_map(|x| x.as_str()).collect();
    assert!(
        names.contains(&"authorization_code"),
        "AC7: discovery must advertise authorization_code; got {names:?}"
    );
    assert!(
        names.contains(&"refresh_token"),
        "AC7: discovery must advertise refresh_token; got {names:?}"
    );
}

// ── AC8 — access_token JWT shape is unchanged across both grants ──────────

#[tokio::test]
async fn test_access_token_format_unchanged() {
    let server = build_server();
    let seed = fresh_authcode_pair(&server).await;
    let access_authcode = seed["access_token"].as_str().unwrap().to_string();
    let refresh1 = seed["refresh_token"].as_str().unwrap().to_string();
    let (access_refresh, _r2) = rotate(&server, &refresh1).await;

    // Both JWTs must decode to a Claims schema with the documented set:
    // `sub`, `iat`, `exp`, `jti`, `iss`, `aud`. `google_sub` is optional
    // and `skip_serializing_if = Option::is_none` keeps it off the wire
    // for the Solana-wallet OAuth path used here. AC8 specifically
    // demands the schema is preserved across both grants.
    for (label, jwt) in [
        ("authorization_code", &access_authcode),
        ("refresh_token", &access_refresh),
    ] {
        let segments: Vec<&str> = jwt.split('.').collect();
        assert_eq!(
            segments.len(),
            3,
            "AC8: {label} access_token must be a three-segment JWT; got {jwt:?}"
        );
        let payload_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            segments[1],
        )
        .expect("AC8: JWT payload base64url decode");
        let payload: Value = serde_json::from_slice(&payload_bytes).expect("AC8: JWT payload JSON");
        for field in ["sub", "iat", "exp", "jti", "iss", "aud"] {
            assert!(
                payload.get(field).is_some(),
                "AC8: {label} JWT missing required claim {field:?}; payload={payload}"
            );
        }
        assert_eq!(payload["iss"], "mcp.mnemonik.xyz", "AC8: iss is fixed");
        assert_eq!(payload["aud"], "mcp", "AC8: aud is fixed");
    }

    // The `sub` claim must be byte-identical across the two grants since
    // they belong to the same authenticated session.
    let sub_of = |jwt: &str| -> String {
        let payload_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            jwt.split('.').nth(1).unwrap(),
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&payload_bytes).unwrap();
        payload["sub"].as_str().unwrap().to_string()
    };
    assert_eq!(
        sub_of(&access_authcode),
        sub_of(&access_refresh),
        "AC8: sub claim must survive the refresh-grant rotation"
    );
}

// ── AC9 — anonymous recall path stays untouched ───────────────────────────

#[tokio::test]
async fn test_anonymous_recall_unchanged() {
    // AC9 is a regression anchor against `anonymous_recall.rs`. Build the
    // server WITHOUT `with_oauth_token` so the OAuth routes 404 and the
    // bearer-auth middleware path matches the production "no OAuth"
    // configuration the original `anonymous_recall.rs` test exercises.
    let server = TestServer::builder().build();

    let owner = server.server_pubkey();
    {
        let store = server.state.store.lock().expect("store mutex");
        let embedding = vec![0.1f32; 8];
        store
            .save_attestation(
                "ac9-public",
                "AC9 anonymous recall regression keyword",
                "hash-pub",
                &["seed".to_string()],
                "local:ac9-pub",
                "local:ac9-pub-ar",
                &owner,
                &owner,
                "2026-06-04T00:00:00Z",
                WriteMode::Participate,
                Visibility::Public,
                &embedding,
            )
            .expect("seed AC9 public row");
    }

    let result = server
        .call_tool(
            None,
            "mnemonic_recall",
            serde_json::json!({ "query": "AC9 anonymous recall regression keyword", "limit": 5 }),
        )
        .await;
    assert_eq!(
        result.status,
        StatusCode::OK,
        "AC9: anonymous recall must remain 200; envelope={}",
        result.envelope
    );
    let inner = result.result_text();
    let rows = inner["results"].as_array().expect("AC9: results array");
    assert!(
        rows.iter().any(|r| r["attestation_id"] == "ac9-public"),
        "AC9: seeded public row must still surface anonymously; inner={inner}"
    );
}

// ── AC10 — content-type parity (form-encoded vs JSON) ─────────────────────

#[tokio::test]
async fn test_refresh_grant_content_type_parity() {
    let server = build_server();

    // TWO independent bootstraps + authorization_code redemptions so each
    // content-type branch consumes its OWN refresh token. Reusing the
    // same `rt_X` would hit Branch B (cache HIT, parity becomes
    // tautological) or, post-window, Branch C (family revoke).
    let seed_form = fresh_authcode_pair(&server).await;
    let rt_form = seed_form["refresh_token"].as_str().unwrap().to_string();

    let seed_json = fresh_authcode_pair(&server).await;
    let rt_json = seed_json["refresh_token"].as_str().unwrap().to_string();

    // Form-encoded branch — VS Code / Claude.ai wire shape.
    let form_body = format!(
        "grant_type=refresh_token&refresh_token={}",
        urlencode_value(&rt_form)
    );
    let (form_status, form_resp) = post_token_raw(
        &server,
        "application/x-www-form-urlencoded",
        form_body.into_bytes(),
    )
    .await;
    assert_eq!(
        form_status,
        StatusCode::OK,
        "AC10: form-encoded refresh-grant must return 200; body={form_resp}"
    );

    // JSON-encoded branch — Cursor wire shape.
    let json_body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": rt_json,
    });
    let (json_status, json_resp) = post_token_raw(
        &server,
        "application/json",
        serde_json::to_vec(&json_body).unwrap(),
    )
    .await;
    assert_eq!(
        json_status,
        StatusCode::OK,
        "AC10: JSON-encoded refresh-grant must return 200; body={json_resp}"
    );

    assert_structural_eq(&form_resp, &json_resp);
}

/// Minimal URL-encoder for application/x-www-form-urlencoded values.
/// Reserved characters are encoded; everything in the URL-safe-base64
/// charset (`A-Za-z0-9-_~.`) passes through. Sufficient for the
/// 43-character refresh_token plaintext AC10 sends.
fn urlencode_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// ── AC11 — legacy client never reads refresh_token, keeps working ─────────

#[tokio::test]
async fn test_back_compat_ignores_refresh_field() {
    let server = build_server();

    // The legacy client performs multiple `authorization_code` exchanges
    // and a Bearer-gated tool call between them, NEVER reading the
    // `refresh_token` field and NEVER sending `grant_type=refresh_token`.
    // The additive field must not break parsing.
    for cycle in 0..3 {
        let tok = fresh_authcode_pair(&server).await;
        assert!(
            tok["access_token"].as_str().is_some_and(|s| !s.is_empty()),
            "AC11 cycle {cycle}: access_token field is present and non-empty"
        );
        // The additive `refresh_token` field is allowed in the envelope —
        // the legacy client just ignores it. Parse it via `get` so an
        // absent field does NOT panic.
        let _ignored: Option<&str> = tok.get("refresh_token").and_then(|v| v.as_str());

        // Mid-cycle Bearer-gated tool call — pin that the OAuth pipeline
        // continues serving authenticated traffic even though we never
        // touched the refresh-grant.
        let owner = server.server_pubkey();
        let result = server
            .call_tool(
                Some(&owner),
                "mnemonic_recall",
                serde_json::json!({ "query": "AC11 probe", "limit": 1 }),
            )
            .await;
        assert_eq!(
            result.status,
            StatusCode::OK,
            "AC11 cycle {cycle}: authenticated tool call must return 200; envelope={}",
            result.envelope
        );
    }
}

// ── AC12 — 10 parallel rotations of the same rt_X are byte-identical ──────

#[tokio::test]
async fn test_concurrent_rotation_idempotent_within_reuse() {
    let server = build_server();
    let seed = fresh_authcode_pair(&server).await;
    let r1 = seed["refresh_token"].as_str().unwrap().to_string();

    // Fan out 10 concurrent rotations through `tokio::spawn` — `tokio::join!`
    // is fixed-arity and clunky at 10. Each future drives the `rotate`
    // helper, which posts `grant_type=refresh_token` with r1 as the
    // plaintext. Inside the 100 ms reuse window exactly one wins Branch A
    // (mints the new pair, publishes to ReuseCache before COMMIT); the
    // other nine hit Branch B (cache HIT, returns the cached pair). All
    // ten must observe byte-identical `(access, refresh)` pairs.
    let server = Arc::new(server);
    let mut handles = Vec::with_capacity(10);
    for _ in 0..10 {
        let server = Arc::clone(&server);
        let r1 = r1.clone();
        handles.push(tokio::spawn(async move { rotate(&server, &r1).await }));
    }
    let results = join_all(handles).await;
    let pairs: Vec<(String, String)> = results
        .into_iter()
        .map(|h| h.expect("AC12: rotate task did not panic"))
        .collect();

    let (first_access, first_refresh) = pairs[0].clone();
    for (i, (access, refresh)) in pairs.iter().enumerate() {
        assert_eq!(
            access, &first_access,
            "AC12: rotation {i} access_token must equal rotation 0 byte-for-byte"
        );
        assert_eq!(
            refresh, &first_refresh,
            "AC12: rotation {i} refresh_token must equal rotation 0 byte-for-byte"
        );
    }
    // Sanity — none of the pairs should be the seed.
    assert_ne!(first_refresh, r1, "AC12: rotated refresh differs from seed");
}

// ── AC13 — refresh-grant without refresh_token → invalid_request ──────────

#[tokio::test]
async fn test_malformed_refresh_grant_invalid_request() {
    let server = build_server();
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        // refresh_token field intentionally omitted
    });
    let (status, parsed) = post_token_raw(
        &server,
        "application/json",
        serde_json::to_vec(&body).unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "AC13: missing refresh_token must be 400; body={parsed}"
    );
    assert_eq!(
        parsed["error"], "invalid_request",
        "AC13: error code must be invalid_request; body={parsed}"
    );
}

// ── Cross-cutting — mutex non-starvation under interleaved load ───────────

#[tokio::test]
async fn cross_table_mutex_no_starvation() {
    let server = Arc::new(build_server());

    // Pre-seed an `authorization_code` so the rotation side has a starting
    // refresh token to chain off. Each rotation thread chains its OWN
    // sequence by remembering the latest refresh; otherwise the same r1
    // would race through the same cache slot and degenerate to all-Branch-B.
    let seed = fresh_authcode_pair(&server).await;
    let r1 = seed["refresh_token"].as_str().unwrap().to_string();

    let owner = server.server_pubkey();
    let embedding = vec![0.1f32; 8];

    // Spawn the rotation side — 50 sequential rotations from a single
    // refresh-token chain. Each rotation takes the writer lock on
    // `oauth_state.refresh_store` for the duration of the rusqlite
    // BEGIN IMMEDIATE / COMMIT (no `.await` while held).
    let rotation_completions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let rotations = {
        let server = Arc::clone(&server);
        let completions = Arc::clone(&rotation_completions);
        tokio::spawn(async move {
            let mut current = r1;
            for i in 0..50 {
                // The 100 ms reuse window allows in-window retries to hit
                // Branch B; insert a tiny sleep to force a fresh Branch A
                // on every loop iteration so the writer lock is exercised.
                let (_a, r_next) = rotate(&server, &current).await;
                current = r_next;
                completions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Yield so the attestation-side task can interleave.
                tokio::task::yield_now().await;
                let _ = i;
            }
        })
    };

    // Spawn the attestation-write side — 50 sequential saves through
    // `McpState.store` (a separate `Arc<Mutex<Connection>>` from the
    // OAuth refresh store, so this is a cross-table check on per-table
    // mutex fairness rather than a single-mutex contention test).
    let attestation_completions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attestations = {
        let server = Arc::clone(&server);
        let completions = Arc::clone(&attestation_completions);
        let owner = owner.clone();
        let embedding = embedding.clone();
        tokio::spawn(async move {
            for i in 0..50 {
                {
                    let store = server.state.store.lock().expect("store mutex");
                    store
                        .save_attestation(
                            &format!("cross-{i}"),
                            &format!("cross-table content {i}"),
                            &format!("hash-{i}"),
                            &["seed".to_string()],
                            &format!("local:cross-{i}"),
                            &format!("local:cross-{i}-ar"),
                            &owner,
                            &owner,
                            "2026-06-04T00:00:00Z",
                            WriteMode::Local,
                            Visibility::Public,
                            &embedding,
                        )
                        .expect("save_attestation");
                    // Guard dropped at end of scope — no `.await` held.
                }
                completions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        })
    };

    // Bounded total time budget. 50 rotations × 1 ms each ≈ 50 ms; 50
    // attestation writes × 0.1 ms each ≈ 5 ms; serial expectation is
    // well under 500 ms. The actual budget here is intentionally
    // generous (1 s) to absorb cgroup-throttled CI runners while still
    // catching a true deadlock (which would timeout indefinitely).
    let combined = futures::future::join(rotations, attestations);
    let outcome = tokio::time::timeout(Duration::from_secs(1), combined).await;
    let (r_res, a_res) = outcome.expect("cross-table mutex did not deadlock within 1s");
    r_res.expect("rotation task did not panic");
    a_res.expect("attestation task did not panic");

    let rotations_done = rotation_completions.load(std::sync::atomic::Ordering::SeqCst);
    let attestations_done = attestation_completions.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        rotations_done, 50,
        "cross-table: rotation side must complete all 50 (not starved); got {rotations_done}"
    );
    assert_eq!(
        attestations_done, 50,
        "cross-table: attestation side must complete all 50 (not starved); got {attestations_done}"
    );
}

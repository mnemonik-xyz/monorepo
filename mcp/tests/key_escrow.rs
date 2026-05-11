//! Integration tests for the chrome-extension key-escrow endpoints (T15
//! of `work/chrome-extension/`). Drives Decision 9 end-to-end:
//!
//! - **rate_limit_blocks_brute_force** — issue 5 GETs, 6th returns 429 with
//!   `Retry-After`. Bounds online brute-force on the encrypted blob (the
//!   Argon2id KDF bounds the offline brute-force; this test pins the online
//!   side).
//! - **pubkey_binding_enforced** — PUT with `pubkey_base58 !=
//!   linked_owner_pubkey` returns 403. Closes the stolen-Google-account →
//!   swap-key attack.
//! - **server_never_stores_plaintext** — the migration SQL contains only
//!   ciphertext + opaque KDF params + bound pubkey. No `plaintext_key` /
//!   `secret_key` / `seed` field. Asserts D9's zero-knowledge property at
//!   the schema level.
//! - PUT then GET round-trip returns identical bytes.
//! - Stale 24h window seeded into `last_fetch_at` → counter resets, GET
//!   succeeds.
//! - DELETE removes the row; subsequent GET → 404.
//! - PUT without prior `google_identity_links` row → 403.
//!
//! Run with: `cargo test -p mnemonic-mcp --features test-support
//!            --test key_escrow -- --nocapture`

#![cfg(feature = "test-support")]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Router,
};
use base64::Engine as _;
use http_body_util::BodyExt;
use mnemonic_mcp::{
    escrow::{
        self, extension_bootstrap_issue_handler, extension_bootstrap_redeem_handler,
        key_escrow_delete_handler, key_escrow_get_handler, key_escrow_put_handler,
        mint_extension_jwt, EscrowState, ExtensionBootstrapTickets, MIGRATION_SQL,
    },
    oauth::{self, OAuthState},
    test_support::mock_state,
};
use rusqlite::params;
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"key-escrow-test-secret-32-bytes!";
const TEST_RATE_LIMIT: u32 = 5;

// ── Harness ──────────────────────────────────────────────────────────────────

struct Harness {
    app: Router,
    escrow_state: Arc<EscrowState>,
    oauth_state: Arc<OAuthState>,
    mcp: Arc<mnemonic_mcp::mcp::McpState>,
}

fn build_app(escrow_state: Arc<EscrowState>) -> Router {
    // Same routing as production main.rs, minus the bearer-auth middleware on
    // /issue (we mint the issue-time `aud=mcp` JWT inline in each test and
    // simulate the Claims extension via a thin wrapper). This keeps the
    // integration tests independent of the global middleware setup.
    let issue_router = Router::new()
        .route(
            "/api/extension-bootstrap/issue",
            post(extension_bootstrap_issue_handler_with_claims),
        )
        .with_state(escrow_state.clone());
    let redeem_router = Router::new()
        .route(
            "/api/extension-bootstrap/redeem/{ticket}",
            get(extension_bootstrap_redeem_handler),
        )
        .with_state(escrow_state.clone());
    let key_escrow_router = Router::new()
        .route(
            "/api/key-escrow",
            axum::routing::put(key_escrow_put_handler)
                .get(key_escrow_get_handler)
                .delete(key_escrow_delete_handler),
        )
        .with_state(escrow_state);
    Router::new()
        .merge(issue_router)
        .merge(redeem_router)
        .merge(key_escrow_router)
}

/// Test-side wrapper around `extension_bootstrap_issue_handler` that manually
/// verifies the `aud=mcp` Bearer JWT (the production bearer-auth middleware
/// does this in main.rs; we recreate the contract inline so tests don't have
/// to thread the middleware).
async fn extension_bootstrap_issue_handler_with_claims(
    axum::extract::State(state): axum::extract::State<Arc<EscrowState>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let token = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim())
    {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": "missing Bearer JWT"})),
            )
                .into_response()
        }
    };
    let claims = match oauth::verify_jwt(&state.oauth, &token) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": format!("invalid JWT: {e}")})),
            )
                .into_response()
        }
    };
    // Re-build a request with Extension<Claims> set and dispatch.
    extension_bootstrap_issue_handler(axum::extract::State(state.clone()), axum::Extension(claims))
        .await
}

fn make_harness() -> Harness {
    make_harness_with_tickets(None)
}

/// Variant of `make_harness` that lets a test inject a custom
/// `ExtensionBootstrapTickets` (e.g. with `ttl_seconds=0` to drive the
/// expired-ticket path). `None` uses the production defaults.
fn make_harness_with_tickets(tickets: Option<Arc<ExtensionBootstrapTickets>>) -> Harness {
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let mcp = mock_state();
    // Migrate both tables: T14's google_identity_links first (so the FK in
    // key_escrow_blobs can resolve), then T15's key_escrow_blobs.
    {
        let store = mcp.store.lock().expect("store mutex");
        oauth::google::migrate_google_identity_links(store.conn())
            .expect("migrate google_identity_links");
        escrow::migrate_key_escrow_blobs(store.conn()).expect("migrate key_escrow_blobs");
    }
    let mut state = EscrowState::new(TEST_RATE_LIMIT, mcp.clone(), oauth_state.clone());
    if let Some(t) = tickets {
        state.bootstrap_tickets = t;
    }
    let escrow_state = Arc::new(state);
    let app = build_app(escrow_state.clone());
    Harness {
        app,
        escrow_state,
        oauth_state,
        mcp,
    }
}

/// Seed a `google_identity_links` row so PUT can pass the binding check.
///
/// NOTE: `linked_at` is **unix seconds (INTEGER)** in the actual T14 migration
/// (`oauth/google.rs::migrate_google_identity_links`), not RFC3339 TEXT as
/// the original T15 task spec described. T14's schema is the source of
/// truth; the T15 spec deviation is recorded in `decisions.md` (T15
/// round-2 entry "linked_at column type"). Tests insert an integer here
/// to match the real column type.
fn seed_link(h: &Harness, google_sub: &str, pubkey: &str) {
    let store = h.mcp.store.lock().expect("store mutex");
    let linked_at: i64 = chrono::Utc::now().timestamp();
    store
        .conn()
        .execute(
            "INSERT INTO google_identity_links (google_sub, pubkey_base58, linked_at) \
             VALUES (?1, ?2, ?3)",
            params![google_sub, pubkey, linked_at],
        )
        .expect("seed google_identity_links");
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────

async fn put_escrow(app: &Router, token: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("PUT")
        .uri("/api/key-escrow")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, parsed)
}

async fn get_escrow(app: &Router, token: &str) -> (StatusCode, Option<String>, Value) {
    let req = Request::builder()
        .method("GET")
        .uri("/api/key-escrow")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, retry_after, parsed)
}

async fn delete_escrow(app: &Router, token: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/key-escrow")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, parsed)
}

fn put_body(pubkey: &str) -> Value {
    let ct = b"\xfe\xed\xfa\xce\xde\xad\xbe\xef".to_vec();
    let nonce = (0..12u8).collect::<Vec<u8>>();
    serde_json::json!({
        "ciphertext_b64": base64::engine::general_purpose::STANDARD.encode(&ct),
        "nonce_b64": base64::engine::general_purpose::STANDARD.encode(&nonce),
        "kdf": "argon2id",
        "kdf_params": r#"{"m":65536,"t":3,"p":1,"s":"YWFhYWFhYWFhYWFhYWFhYQ=="}"#,
        "pubkey_base58": pubkey,
    })
}

// ── TDD anchor #1: rate-limit blocks brute force ─────────────────────────────

#[tokio::test]
async fn rate_limit_blocks_brute_force() {
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-rate-limit-1";
    seed_link(&h, google_sub, &pubkey);

    let token = mint_extension_jwt(&h.oauth_state, &pubkey, Some(google_sub.to_string())).unwrap();

    // PUT once to populate the row. The PUT resets the counter.
    let (st, _) = put_escrow(&h.app, &token, put_body(&pubkey)).await;
    assert_eq!(st, StatusCode::OK, "initial PUT must succeed");

    // TEST_RATE_LIMIT=5 GETs are allowed.
    for i in 0..TEST_RATE_LIMIT {
        let (st, _ra, _val) = get_escrow(&h.app, &token).await;
        assert_eq!(
            st,
            StatusCode::OK,
            "GET #{} (1-indexed) within the cap must succeed, got {st}",
            i + 1
        );
    }

    // The (cap+1)th GET must return 429 with a Retry-After header.
    let (st, retry_after, val) = get_escrow(&h.app, &token).await;
    assert_eq!(
        st,
        StatusCode::TOO_MANY_REQUESTS,
        "6th GET must be rate-limited, got {st} body={val:?}"
    );
    // Body must carry the fixed `rate_limited` token. A regression that
    // drops or renames the field (e.g. {"error": null}) would silently
    // satisfy "any 429" without this assertion. F3 of test-reviewer-round1.
    let body_err = val
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        body_err, "rate_limited",
        "429 body error must be the fixed token 'rate_limited', got {body_err:?}"
    );
    let ra = retry_after.expect("Retry-After header must be set on 429");
    let ra_secs: i64 = ra.parse().expect("Retry-After must be an integer");
    assert!(
        ra_secs > 0 && ra_secs <= 24 * 3600,
        "Retry-After must be in (0, 24h], got {ra_secs}"
    );
}

// ── TDD anchor #2: pubkey binding enforced ───────────────────────────────────

#[tokio::test]
async fn pubkey_binding_enforced() {
    let h = make_harness();
    let owner = Keypair::new();
    let attacker = Keypair::new();
    let owner_pubkey = owner.pubkey().to_string();
    let attacker_pubkey = attacker.pubkey().to_string();
    let google_sub = "user-binding-1";
    seed_link(&h, google_sub, &owner_pubkey);

    let token =
        mint_extension_jwt(&h.oauth_state, &owner_pubkey, Some(google_sub.to_string())).unwrap();

    // PUT with the wrong pubkey_base58 — must 403 even though the JWT carries
    // a matching google_sub (an attacker who phished the Google account can
    // mint the JWT but cannot rebind the on-chain identity).
    let (st, val) = put_escrow(&h.app, &token, put_body(&attacker_pubkey)).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "PUT with mismatched pubkey must 403, got {st} body={val:?}"
    );
    let msg = val.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        msg.contains("pubkey_base58") || msg.contains("linked"),
        "403 body should mention the binding mismatch, got {msg}"
    );

    // Sanity: the correct pubkey works.
    let (st, _) = put_escrow(&h.app, &token, put_body(&owner_pubkey)).await;
    assert_eq!(st, StatusCode::OK, "PUT with matching pubkey must 200");
}

// ── TDD anchor #3: server never stores plaintext (schema audit) ──────────────

#[tokio::test]
async fn server_never_stores_plaintext() {
    // Direct schema-level audit: the migration SQL must not contain any
    // column name that suggests plaintext key storage. This is a static
    // check on the source-of-truth migration string, so it catches a future
    // refactor that adds an unintended plaintext-leaking column.
    let lower = MIGRATION_SQL.to_lowercase();
    for forbidden in [
        "plaintext",
        "plaintext_key",
        "secret_key",
        "secret_seed",
        " seed ",
        "raw_key",
        "private_key",
        "unencrypted",
    ] {
        assert!(
            !lower.contains(forbidden),
            "migration SQL must not mention {forbidden:?} (zero-knowledge property — D9); migration: {MIGRATION_SQL}"
        );
    }
    // Sanity: the migration DOES carry the opaque storage columns.
    for required in ["ciphertext", "nonce", "kdf", "kdf_params", "pubkey_base58"] {
        assert!(
            lower.contains(required),
            "migration SQL must include {required:?} column"
        );
    }
}

// ── Round-trip + lifecycle ────────────────────────────────────────────────────

#[tokio::test]
async fn put_then_get_round_trip_identical_bytes() {
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-roundtrip-1";
    seed_link(&h, google_sub, &pubkey);

    let token = mint_extension_jwt(&h.oauth_state, &pubkey, Some(google_sub.to_string())).unwrap();
    let body = put_body(&pubkey);
    let (st, _) = put_escrow(&h.app, &token, body.clone()).await;
    assert_eq!(st, StatusCode::OK);

    let (st, _ra, val) = get_escrow(&h.app, &token).await;
    assert_eq!(st, StatusCode::OK, "GET must succeed; body={val:?}");
    assert_eq!(val["ciphertext_b64"], body["ciphertext_b64"]);
    assert_eq!(val["nonce_b64"], body["nonce_b64"]);
    assert_eq!(val["kdf"], body["kdf"]);
    assert_eq!(val["kdf_params"], body["kdf_params"]);
    assert_eq!(val["pubkey_base58"], body["pubkey_base58"]);
}

#[tokio::test]
async fn stale_window_resets_counter() {
    // Seed last_fetch_at to >24h ago so the rolling-window reset path fires
    // (T15 spec calls this out). Counter should reset to 1 on the next GET
    // and the user gets a fresh budget of TEST_RATE_LIMIT - 1 more fetches.
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-stale-window-1";
    seed_link(&h, google_sub, &pubkey);
    let token = mint_extension_jwt(&h.oauth_state, &pubkey, Some(google_sub.to_string())).unwrap();

    let (st, _) = put_escrow(&h.app, &token, put_body(&pubkey)).await;
    assert_eq!(st, StatusCode::OK);

    // Burn the entire rate-limit budget.
    for _ in 0..TEST_RATE_LIMIT {
        let (st, _, _) = get_escrow(&h.app, &token).await;
        assert_eq!(st, StatusCode::OK);
    }
    // Next GET must 429.
    let (st, _, _) = get_escrow(&h.app, &token).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);

    // Seed last_fetch_at to 25h ago so the next GET sees an elapsed window.
    {
        let store = h.mcp.store.lock().unwrap();
        let twenty_five_h_ago = (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
        store
            .conn()
            .execute(
                "UPDATE key_escrow_blobs SET last_fetch_at = ?1 WHERE google_sub = ?2",
                params![twenty_five_h_ago, google_sub],
            )
            .unwrap();
    }

    // GET must succeed and reset the counter to 1.
    let (st, _, _) = get_escrow(&h.app, &token).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "GET after 24h window elapse must succeed"
    );

    // Verify the counter is back to 1 (defense: read directly from SQLite).
    let counter: i64 = {
        let store = h.mcp.store.lock().unwrap();
        store
            .conn()
            .query_row(
                "SELECT fetch_count_24h FROM key_escrow_blobs WHERE google_sub = ?1",
                params![google_sub],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(
        counter, 1,
        "counter must reset to 1 after stale-window GET, got {counter}"
    );
}

#[tokio::test]
async fn delete_removes_row_subsequent_get_404() {
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-delete-1";
    seed_link(&h, google_sub, &pubkey);
    let token = mint_extension_jwt(&h.oauth_state, &pubkey, Some(google_sub.to_string())).unwrap();

    let (st, _) = put_escrow(&h.app, &token, put_body(&pubkey)).await;
    assert_eq!(st, StatusCode::OK);

    let (st, val) = delete_escrow(&h.app, &token).await;
    assert_eq!(st, StatusCode::OK, "DELETE must 200, got {st} body={val:?}");
    assert_eq!(val["removed"].as_i64().unwrap_or(-1), 1);

    let (st, _, _) = get_escrow(&h.app, &token).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "GET after DELETE must 404");

    // Idempotent: a second DELETE returns 200 with removed=0.
    let (st, val) = delete_escrow(&h.app, &token).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(val["removed"].as_i64().unwrap_or(-1), 0);
}

#[tokio::test]
async fn put_without_prior_google_identity_link_403() {
    // No seed_link — the FK target row is missing. Even with a valid JWT
    // carrying a matching `google_sub`, PUT must 403 (app-level check fires
    // before the FK violation hits the DB).
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-no-link-1";
    let token = mint_extension_jwt(&h.oauth_state, &pubkey, Some(google_sub.to_string())).unwrap();
    let (st, val) = put_escrow(&h.app, &token, put_body(&pubkey)).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "PUT without link must 403, got {st} body={val:?}"
    );
}

// ── JWT audience + auth negative tests ───────────────────────────────────────

#[tokio::test]
async fn key_escrow_rejects_mcp_aud_jwt() {
    // A standard `aud=mcp` JWT (issued by the production OAuth flow) must
    // NOT be accepted at the escrow endpoints — they require `aud=extension`.
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-aud-1";
    seed_link(&h, google_sub, &pubkey);

    let mcp_token =
        oauth::issue_jwt_with_google_sub(&h.oauth_state, &pubkey, Some(google_sub.to_string()))
            .expect("issue mcp JWT");
    let (st, _, _) = get_escrow(&h.app, &mcp_token).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "aud=mcp JWT must NOT be accepted on /api/key-escrow"
    );
    let _ = &h.escrow_state; // suppress unused warning under cfg
}

#[tokio::test]
async fn key_escrow_requires_google_sub_claim() {
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    // Mint an aud=extension JWT with NO google_sub claim — must 401.
    let token = mint_extension_jwt(&h.oauth_state, &pubkey, None).expect("issue extension JWT");
    let (st, _, val) = get_escrow(&h.app, &token).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "JWT without google_sub must 401, got {st} body={val:?}"
    );
}

// ── Bootstrap-ticket round-trip ──────────────────────────────────────────────

#[tokio::test]
async fn bootstrap_ticket_round_trip_issues_extension_jwt() {
    // Issue a ticket (with an aud=mcp JWT) → redeem → assert the response
    // body carries an aud=extension JWT that verifies under the same secret.
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-bootstrap-1";
    seed_link(&h, google_sub, &pubkey);

    let mcp_token =
        oauth::issue_jwt_with_google_sub(&h.oauth_state, &pubkey, Some(google_sub.to_string()))
            .expect("issue mcp JWT");
    let issue_req = Request::builder()
        .method("POST")
        .uri("/api/extension-bootstrap/issue")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {mcp_token}"))
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp = h.app.clone().oneshot(issue_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let issue_resp: Value = serde_json::from_slice(&body_bytes).unwrap();
    let ticket = issue_resp["ticket_id"].as_str().unwrap().to_string();

    // Redeem: no auth header.
    let redeem_req = Request::builder()
        .method("GET")
        .uri(format!("/api/extension-bootstrap/redeem/{ticket}"))
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(redeem_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let redeem_resp: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(redeem_resp["aud"].as_str(), Some("extension"));
    let ext_token = redeem_resp["access_token"].as_str().unwrap().to_string();

    // The extension JWT must work against /api/key-escrow.
    let (st, _) = put_escrow(&h.app, &ext_token, put_body(&pubkey)).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "PUT with redeemed extension JWT must 200"
    );

    // Second redeem of the SAME ticket → 404 (single-use).
    let redeem_req2 = Request::builder()
        .method("GET")
        .uri(format!("/api/extension-bootstrap/redeem/{ticket}"))
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(redeem_req2).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "second redeem of the same ticket must 404"
    );
}

// ── Round-2 review fixes ─────────────────────────────────────────────────────

/// F1 (test-reviewer round-1, blocker): cross-user DELETE isolation.
///
/// User A's `aud=extension` JWT must NOT touch user B's escrow row. The
/// production code scopes DELETE by `WHERE google_sub = ?` from the
/// validated JWT claim, so a wrong-identity DELETE simply matches nothing
/// and returns `removed=0`. This test pins that invariant by reading user
/// B's row directly from SQLite after the cross-user DELETE attempt — a
/// regression that drops the `WHERE` clause from the DELETE SQL would
/// erase user B's row and fail this test.
#[tokio::test]
async fn delete_cross_user_isolation() {
    let h = make_harness();
    let user_a = Keypair::new();
    let user_b = Keypair::new();
    let pk_a = user_a.pubkey().to_string();
    let pk_b = user_b.pubkey().to_string();
    let sub_a = "user-cross-delete-a";
    let sub_b = "user-cross-delete-b";
    seed_link(&h, sub_a, &pk_a);
    seed_link(&h, sub_b, &pk_b);

    // User B puts an escrow row.
    let token_b = mint_extension_jwt(&h.oauth_state, &pk_b, Some(sub_b.to_string())).unwrap();
    let (st, _) = put_escrow(&h.app, &token_b, put_body(&pk_b)).await;
    assert_eq!(st, StatusCode::OK, "user B PUT must succeed");

    // User A's token: aud=extension, google_sub=sub_a.
    let token_a = mint_extension_jwt(&h.oauth_state, &pk_a, Some(sub_a.to_string())).unwrap();
    let (st, val) = delete_escrow(&h.app, &token_a).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "DELETE with wrong identity must 200 with removed=0 (not 403), got {st} body={val:?}"
    );
    assert_eq!(
        val["removed"].as_i64().unwrap_or(-1),
        0,
        "removed must be 0 — user A's DELETE must not touch user B's row"
    );

    // Defense: read user B's row directly from SQLite. A regression that
    // dropped the WHERE clause from delete_escrow would have erased it.
    let row_exists: bool = {
        let store = h.mcp.store.lock().expect("store");
        store
            .conn()
            .query_row(
                "SELECT 1 FROM key_escrow_blobs WHERE google_sub = ?1",
                params![sub_b],
                |_r| Ok(true),
            )
            .unwrap_or(false)
    };
    assert!(
        row_exists,
        "user B's escrow row must still exist after user A's cross-user DELETE"
    );
}

/// F2 (test-reviewer round-1, blocker): expired bootstrap ticket → 404.
///
/// Builds a harness with `ttl_seconds=0` so any ticket issued is already
/// expired by the time consume() compares `now_unix() >= entry.expires_at`.
/// Redeem must return 404 with the same body as garbage UUIDs (no oracle).
#[tokio::test]
async fn expired_bootstrap_ticket_returns_404() {
    // ttl=0 → expires_at = now + 0 = now → consume's `now >= expires_at`
    // is true immediately, so redeem sees the ticket as expired.
    let tickets = Arc::new(ExtensionBootstrapTickets::new(
        16, // small LRU cap — irrelevant for this test
        3,  // per-user cap
        0,  // ttl_seconds = 0 forces immediate expiry
    ));
    let h = make_harness_with_tickets(Some(tickets));
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-expired-ticket";
    seed_link(&h, google_sub, &pubkey);

    // Issue: needs a valid aud=mcp JWT.
    let mcp_token =
        oauth::issue_jwt_with_google_sub(&h.oauth_state, &pubkey, Some(google_sub.to_string()))
            .expect("issue mcp JWT");
    let issue_req = Request::builder()
        .method("POST")
        .uri("/api/extension-bootstrap/issue")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {mcp_token}"))
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp = h.app.clone().oneshot(issue_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "issue must succeed");
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let issue_resp: Value = serde_json::from_slice(&body_bytes).unwrap();
    let ticket = issue_resp["ticket_id"].as_str().unwrap().to_string();

    // Redeem: ticket is already expired (ttl=0).
    let redeem_req = Request::builder()
        .method("GET")
        .uri(format!("/api/extension-bootstrap/redeem/{ticket}"))
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(redeem_req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "expired ticket must 404 — same body as garbage UUID (no oracle)"
    );
}

/// F5 (test-reviewer round-1, minor): PUT and DELETE must also 401 when the
/// JWT carries no `google_sub` claim. The existing
/// `key_escrow_requires_google_sub_claim` covers GET only; this pins the
/// same invariant for the write verbs.
#[tokio::test]
async fn key_escrow_put_and_delete_require_google_sub_claim() {
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    // aud=extension JWT with NO google_sub claim.
    let token = mint_extension_jwt(&h.oauth_state, &pubkey, None).expect("issue extension JWT");

    let (st, _) = put_escrow(&h.app, &token, put_body(&pubkey)).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "PUT without google_sub must 401, got {st}"
    );

    let (st, _) = delete_escrow(&h.app, &token).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "DELETE without google_sub must 401, got {st}"
    );
}

/// F7 (test-reviewer round-1, minor): per-user bootstrap-ticket cap is
/// reached at the 4th issue.
///
/// Issues 3 tickets for the same `aud=mcp` sub, asserts all 200; the 4th
/// must return 429 with the fixed `too_many_pending_tickets` body token.
/// Defense in depth against the T15-M-02 attack where expired tickets
/// otherwise pinned the counter (fix: insert-time expired-ticket sweep,
/// also exercised below).
#[tokio::test]
async fn bootstrap_per_user_cap_blocks_fourth_ticket() {
    let h = make_harness();
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-percap-1";
    let mcp_token =
        oauth::issue_jwt_with_google_sub(&h.oauth_state, &pubkey, Some(google_sub.to_string()))
            .expect("issue mcp JWT");

    async fn one_issue(app: &Router, token: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/extension-bootstrap/issue")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(b"{}".to_vec()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
        (status, parsed)
    }

    for i in 0..3 {
        let (st, val) = one_issue(&h.app, &mcp_token).await;
        assert_eq!(
            st,
            StatusCode::OK,
            "issue #{} of 3 must 200, got {st} body={val:?}",
            i + 1
        );
    }
    let (st, val) = one_issue(&h.app, &mcp_token).await;
    assert_eq!(
        st,
        StatusCode::TOO_MANY_REQUESTS,
        "4th issue must 429, got {st} body={val:?}"
    );
    let err = val.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(
        err, "too_many_pending_tickets",
        "429 body must be the fixed opaque token (no constant leak), got {err:?}"
    );
}

/// Round-1 finding T15-M-02 (security-auditor): after 3 expired tickets
/// the user must be able to issue again. The insert-time sweep removes
/// expired entries belonging to the same `jwt_sub` BEFORE counting toward
/// the cap, so a self-DoS via three-then-wait is prevented.
#[tokio::test]
async fn expired_tickets_do_not_pin_per_user_counter() {
    // ttl=0 → tickets are expired as soon as they're inserted.
    let tickets = Arc::new(ExtensionBootstrapTickets::new(16, 3, 0));
    let h = make_harness_with_tickets(Some(tickets));
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let google_sub = "user-expsweep-1";
    let mcp_token =
        oauth::issue_jwt_with_google_sub(&h.oauth_state, &pubkey, Some(google_sub.to_string()))
            .expect("issue mcp JWT");

    let do_issue = |token: String| {
        let app = h.app.clone();
        async move {
            let req = Request::builder()
                .method("POST")
                .uri("/api/extension-bootstrap/issue")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(b"{}".to_vec()))
                .unwrap();
            app.oneshot(req).await.unwrap().status()
        }
    };

    // Burn 3 tickets — they all expire immediately (ttl=0). Without the
    // round-2 sweep, the per_user counter would stick at 3 and the 4th
    // issue would 429. With the sweep, the 4th issue must 200.
    for _ in 0..3 {
        assert_eq!(do_issue(mcp_token.clone()).await, StatusCode::OK);
    }
    let st = do_issue(mcp_token.clone()).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "4th issue must 200 — expired tickets must NOT pin the per-user counter \
         (T15-M-02 round-1 finding)"
    );
}

/// Round-1 finding T15-L-02 (security-auditor): explicit cross-user
/// isolation on GET. User A's `aud=extension` JWT must not return user B's
/// escrow row even if both users have rows. The production code reads
/// `google_sub` only from the validated JWT claim, never from the request
/// body — this test pins the invariant for future refactors.
#[tokio::test]
async fn get_cross_user_isolation() {
    let h = make_harness();
    let user_a = Keypair::new();
    let user_b = Keypair::new();
    let pk_a = user_a.pubkey().to_string();
    let pk_b = user_b.pubkey().to_string();
    let sub_a = "user-cross-get-a";
    let sub_b = "user-cross-get-b";
    seed_link(&h, sub_a, &pk_a);
    seed_link(&h, sub_b, &pk_b);

    // Only user B has an escrow row.
    let token_b = mint_extension_jwt(&h.oauth_state, &pk_b, Some(sub_b.to_string())).unwrap();
    let (st, _) = put_escrow(&h.app, &token_b, put_body(&pk_b)).await;
    assert_eq!(st, StatusCode::OK, "user B PUT must succeed");

    // User A's GET — must 404 (no row for sub_a). MUST NOT return user B's
    // ciphertext under any circumstance.
    let token_a = mint_extension_jwt(&h.oauth_state, &pk_a, Some(sub_a.to_string())).unwrap();
    let (st, _ra, val) = get_escrow(&h.app, &token_a).await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "user A GET must 404 when only user B has a row, got {st} body={val:?}"
    );
    // Defense: even on 200 (future refactor regression), the body MUST NOT
    // carry user B's pubkey.
    if let Some(pk) = val.get("pubkey_base58").and_then(|v| v.as_str()) {
        assert_ne!(
            pk, pk_b,
            "user A's GET response must NEVER contain user B's pubkey"
        );
    }
}

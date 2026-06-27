//! Cross-cutting backend integration tests for the webapp-rethink data +
//! publishing paths (Task 11). These are the END-TO-END scenarios that span
//! multiple surfaces at once — the per-route unit paths live in
//! `tests/public_read_routes.rs` (T8) and `tests/blog_publish.rs` (T9); this
//! file deliberately does NOT re-test those.
//!
//! The load-bearing guarantees exercised here:
//!   - Privacy (Decision 6): a PRIVATE owner memory never leaks through ANY
//!     public surface — `GET /artifacts` (plain AND `?q=`) nor `GET /blog` /
//!     `GET /blog/:slug` — even while public rows + a published post coexist in
//!     the same store.
//!   - Cross-surface publish upsert (Decision 5/7): re-publishing the same
//!     title via a DIFFERENT surface REPLACES the row (slug is the PK).
//!   - Cross-surface auth (Decision 5): anonymous publish is rejected on both
//!     surfaces; a single valid bearer authorises a publish on both.
//!   - Per-identity publish rate limit trips after the quota.
//!   - The attestation a publish signs is VERIFIABLE end-to-end through the
//!     live `publish::publish_post` path (POST_V1 COSE round-trip).
//!
//! Built against `test_support::mock_state()` (in-memory SQLite + StubEmbedder)
//! and the live `oauth::bearer_auth_middleware`, exercised via
//! `tower::ServiceExt::oneshot` so no socket is bound. The router mirrors the
//! production merge: public GET routes on the base router, authed POST /blog +
//! /mcp on the bearer-layered subrouter.

#![cfg(feature = "test-support")]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use mnemonic_mcp::{
    api,
    mcp::{self, McpState},
    oauth::{self, OAuthState},
    publish::{self, PublishInput},
    test_support::mock_state,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"privacy-publish-e2e-secret-32by!";

/// Mirror the production wiring: public GET read routes on the base router,
/// authed `POST /blog` + `/mcp` on a bearer-layered subrouter merged in.
fn build_router(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    let authed = Router::new()
        .route("/blog", post(api::blog_publish_handler))
        .route("/mcp", post(mcp::mcp_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state.clone());

    Router::new()
        .route("/artifacts", get(api::artifacts_handler))
        .route("/blog", get(api::blog_list_handler))
        .route("/blog/{slug}", get(api::blog_post_handler))
        .with_state(state)
        .merge(authed)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
    let parsed: Value = serde_json::from_str(&trimmed).unwrap_or(Value::String(trimmed));
    (status, headers, parsed)
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let (status, _h, v) = send(app, req).await;
    (status, v)
}

fn post_blog_json(body: Value, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/blog")
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn tools_call_publish(app: &Router, args: Value, token: Option<&str>) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "mnemonic_publish_post", "arguments": args}
    });
    let req = b
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _h, v) = send(app, req).await;
    (status, v)
}

/// Seed one PUBLIC and one PRIVATE attestation under `owner`, both with the
/// constant StubEmbedder embedding so the `?q=` cosine path ranks them equally
/// — the only thing that can drop the private row is the visibility predicate.
/// The private content carries a sentinel string that must never reach any wire.
fn seed_public_and_private(state: &Arc<McpState>, owner: &str) {
    let store = state.store.lock().expect("store");
    let embedding = vec![0.1f32; 8];
    store
        .save_attestation(
            "pub-id",
            PUBLIC_SENTINEL,
            "hash-pub",
            &["release".to_string()],
            "5NfSolanaPubSig",
            "ArweavePubTx",
            owner,
            owner,
            "2026-06-10T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
            &embedding,
        )
        .expect("seed public");
    store
        .save_attestation(
            "priv-id",
            PRIVATE_SENTINEL,
            "hash-priv",
            &["secret".to_string()],
            "local:priv",
            "local:priv-ar",
            owner,
            owner,
            "2026-06-11T00:00:00Z",
            WriteMode::Local,
            Visibility::Private,
            &embedding,
        )
        .expect("seed private");
}

const PUBLIC_SENTINEL: &str = "PUBLIC-LEDGER-ROW-VISIBLE-payload";
const PRIVATE_SENTINEL: &str = "TOPSECRET-PRIVATE-PAYLOAD-do-not-leak";

/// The single highest-value assertion of the feature: a private owner memory
/// is invisible on EVERY public surface, even while public rows + a published
/// blog post coexist in the same store. We assert on the absence of the
/// sentinel string in the FULL serialized body (not just array length), so a
/// leak through any field — content, tags, hash — is caught.
#[tokio::test]
async fn private_memory_never_leaks_across_any_public_surface() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let state = mock_state();
    seed_public_and_private(&state, "owner-pubkey-1");
    let app = build_router(state, oauth_state.clone());

    // A public post coexists with the seeded rows (a post IS a public
    // attestation), so the privacy guarantee is exercised with a non-trivial
    // public surface present.
    let token = oauth::issue_jwt(&oauth_state, "PublisherAgent").expect("issue_jwt");
    let (status, _h, _b) = send(
        &app,
        post_blog_json(
            json!({"title": "Public Post", "body_markdown": "openly published body"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Every public surface, including both /artifacts read paths.
    for uri in [
        "/artifacts",
        "/artifacts?q=payload&limit=50",
        "/blog",
        "/blog/public-post",
    ] {
        let (status, body) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} should 200");
        let serialized = body.to_string();
        assert!(
            !serialized.contains(PRIVATE_SENTINEL),
            "PRIVATE content leaked on {uri}: {serialized}"
        );
    }

    // Positive controls — the public row IS visible (so the test is not
    // vacuously passing on an empty/erroring store), and exactly the public
    // attestation surfaces on /artifacts (the post's own attestation + the
    // seeded public row), never the private one.
    let (_s, artifacts) = get_json(&app, "/artifacts").await;
    let body = artifacts.to_string();
    assert!(
        body.contains(PUBLIC_SENTINEL),
        "public row must be visible on /artifacts: {body}"
    );
    let ids: Vec<&str> = artifacts["artifacts"]
        .as_array()
        .expect("artifacts array")
        .iter()
        .filter_map(|a| a["attestation_id"].as_str())
        .collect();
    assert!(
        ids.contains(&"pub-id"),
        "public attestation present: {ids:?}"
    );
    assert!(
        !ids.contains(&"priv-id"),
        "private attestation absent: {ids:?}"
    );
}

/// Slug is the blog_posts PK, so re-publishing the same title — even from a
/// DIFFERENT surface — REPLACES the row rather than creating a duplicate.
/// T9 only publishes one post per slug; this is the cross-surface upsert path.
#[tokio::test]
async fn republish_same_title_replaces_row_across_surfaces() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(mock_state(), oauth_state.clone());
    let token = oauth::issue_jwt(&oauth_state, "EditorAgent").expect("issue_jwt");

    // v1 via the MCP tool.
    let (status, _v) = tools_call_publish(
        &app,
        json!({"title": "Release Notes", "body_markdown": "v1 body"}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_s, detail) = get_json(&app, "/blog/release-notes").await;
    assert_eq!(detail["post"]["body_markdown"], "v1 body");

    // v2 via POST /blog under the SAME slug — must overwrite, not duplicate.
    let (status, _h, _b) = send(
        &app,
        post_blog_json(
            json!({"title": "Release Notes", "body_markdown": "v2 body REPLACED"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (_s, list) = get_json(&app, "/blog").await;
    let matching: Vec<&Value> = list["posts"]
        .as_array()
        .expect("posts array")
        .iter()
        .filter(|p| p["slug"] == "release-notes")
        .collect();
    assert_eq!(matching.len(), 1, "slug PK must dedupe to one row: {list}");

    let (_s, detail) = get_json(&app, "/blog/release-notes").await;
    assert_eq!(
        detail["post"]["body_markdown"], "v2 body REPLACED",
        "latest publish must win"
    );
}

/// Anonymous publish is rejected on BOTH surfaces; a single valid bearer then
/// authorises a publish on BOTH, and both posts coexist. The cross-surface
/// single-token angle is what T9's per-surface auth tests do not cover.
#[tokio::test]
async fn anonymous_rejected_then_single_bearer_authorises_both_surfaces() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(mock_state(), oauth_state.clone());

    // Anonymous → 401 on the HTTP surface and the MCP tool surface.
    let (status, _h, _b) = send(
        &app,
        post_blog_json(json!({"title": "Nope", "body_markdown": "x"}), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "anon POST /blog must 401");
    let (status, _v) =
        tools_call_publish(&app, json!({"title": "Nope", "body_markdown": "x"}), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anon tool publish must 401"
    );

    // One bearer authorises a publish on each surface.
    let token = oauth::issue_jwt(&oauth_state, "DualSurfaceAgent").expect("issue_jwt");
    let (status, _h, _b) = send(
        &app,
        post_blog_json(
            json!({"title": "Via Http", "body_markdown": "http body"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "authed POST /blog must 201");
    let (status, _v) = tools_call_publish(
        &app,
        json!({"title": "Via Tool", "body_markdown": "tool body"}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "authed tool publish must 200");

    let (_s, list) = get_json(&app, "/blog").await;
    let slugs: Vec<&str> = list["posts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["slug"].as_str())
        .collect();
    assert!(slugs.contains(&"via-http"), "http post listed: {slugs:?}");
    assert!(slugs.contains(&"via-tool"), "tool post listed: {slugs:?}");
}

/// The per-identity publish limiter (keyed on the authenticated pubkey) trips
/// once the burst quota is exhausted. `mock_state` seeds a 10/min quota, so a
/// tight burst of 10 succeeds and the 11th is throttled with 429 — proving the
/// abuse control is wired into the live POST /blog path, not just unit-tested.
#[tokio::test]
async fn publish_rate_limit_trips_after_quota() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(mock_state(), oauth_state.clone());
    let token = oauth::issue_jwt(&oauth_state, "FloodAgent").expect("issue_jwt");

    for i in 0..10 {
        let (status, _h, _b) = send(
            &app,
            post_blog_json(
                json!({"title": format!("Post {i}"), "body_markdown": "body"}),
                Some(&token),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "burst publish {i} must 201");
    }

    let (status, _h, _b) = send(
        &app,
        post_blog_json(
            json!({"title": "One Too Many", "body_markdown": "body"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "11th publish past the 10/min quota must 429"
    );
}

/// The attestation a publish signs is VERIFIABLE end-to-end: drive the LIVE
/// `publish::publish_post` pipeline, then reconstruct the POST_V1 artifact from
/// only the returned `BlogPost` fields + the server signer, re-sign, and prove
/// (a) the content_hash is reproducible from the published projection and
/// (b) the COSE_Sign1 envelope verifies against that hash. Distinct from the
/// `publish.rs` unit test, which hand-builds an artifact instead of using the
/// live path's output.
#[tokio::test]
async fn published_attestation_is_verifiable_via_live_path() {
    use mnemonic_core::codec::schema::{validate_artifact, POST_V1};
    use mnemonic_core::codec::sign::{sign_artifact, verify_artifact};
    use mnemonic_core::identity;

    let state = mock_state();
    let post = publish::publish_post(
        &state,
        "AuthorPubkey",
        PublishInput {
            title: "Verifiable Post".into(),
            body_markdown: "# Heading\n\nverifiable body".into(),
            tags: vec!["proof".into()],
            author: Some("Author Name".into()),
        },
    )
    .expect("live publish");

    // content_hash must be a 64-char blake3 hex.
    assert_eq!(
        post.content_hash.len(),
        64,
        "blake3 hex: {}",
        post.content_hash
    );

    // Reconstruct the exact POST_V1 artifact the live path signed, using only
    // fields exposed on the returned BlogPost plus the server's signer identity.
    let artifact = json!({
        "artifact_id": post.attestation_id,
        "type": "post",
        "schema_version": 1,
        "title": post.title,
        "slug": post.slug,
        "content": post.body_markdown,
        "author": post.author,
        "published_at": post.published_at,
        "tags": post.tags,
        "created_at": post.published_at,
        "producer": identity::did_sol(&state.keypair),
    });
    validate_artifact(&artifact, &POST_V1).expect("POST_V1 validates");
    let signed = sign_artifact(&artifact, &POST_V1, &state.keypair).expect("sign");

    assert_eq!(
        signed.content_hash, post.content_hash,
        "live publish content_hash must be reproducible from the projection"
    );
    let result = verify_artifact(&signed.cose_bytes, Some(&post.content_hash)).expect("verify");
    assert!(result.valid, "published POST_V1 COSE must verify");
    assert_eq!(result.signer, identity::pubkey_base58(&state.keypair));
}

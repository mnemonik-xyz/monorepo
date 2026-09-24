//! Cross-cutting backend integration tests for the webapp-rethink data +
//! publishing paths (Task 11). These are the END-TO-END scenarios that span
//! multiple surfaces at once — the per-route unit paths live in
//! `tests/public_read_routes.rs` (T8) and `tests/blog_publish.rs` (T9); this
//! file deliberately does NOT re-test those.
//!
//! The load-bearing guarantees exercised here:
//!   - Surface separation: every stored memory is public, so `GET /artifacts`
//!     (plain AND `?q=`) lists ALL attestation rows; but `GET /blog` /
//!     `GET /blog/:slug` read the separate `blog_posts` table, so a plain
//!     memory (not a published post) never appears on the blog surfaces.
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

/// Surface separation: every memory is public so BOTH attestation rows surface
/// on `/artifacts`, but the blog surfaces (`/blog`, `/blog/:slug`) read the
/// separate `blog_posts` table and therefore never carry a plain memory's
/// content — only published posts.
#[tokio::test]
async fn artifacts_lists_all_memories_blog_lists_only_posts() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let state = mock_state();
    seed_public_and_private(&state, "owner-pubkey-1");
    // Operator identity: the only caller allowed to publish plain fields
    // (its own key signs). User publishes must be client-signed.
    let operator = state.keypair.pubkey_base58();
    let app = build_router(state, oauth_state.clone());

    // A published post coexists with the seeded rows.
    let token = oauth::issue_jwt(&oauth_state, &operator).expect("issue_jwt");
    let (status, _h, _b) = send(
        &app,
        post_blog_json(
            json!({"title": "Public Post", "body_markdown": "openly published body"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Blog surfaces read `blog_posts`, so a plain seeded memory (not a post)
    // never appears there — neither the private nor the public seeded memory.
    for uri in ["/blog", "/blog/public-post"] {
        let (status, body) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} should 200");
        let serialized = body.to_string();
        assert!(
            !serialized.contains(PRIVATE_SENTINEL),
            "seeded memory content must not appear on blog surface {uri}: {serialized}"
        );
        assert!(
            !serialized.contains(PUBLIC_SENTINEL),
            "seeded memory content must not appear on blog surface {uri}: {serialized}"
        );
    }

    // /artifacts lists ALL attestation rows — both seeded memories surface.
    for uri in ["/artifacts", "/artifacts?q=payload&limit=50"] {
        let (status, artifacts) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} should 200");
        let ids: Vec<&str> = artifacts["artifacts"]
            .as_array()
            .expect("artifacts array")
            .iter()
            .filter_map(|a| a["attestation_id"].as_str())
            .collect();
        assert!(
            ids.contains(&"pub-id"),
            "{uri}: public row present: {ids:?}"
        );
        assert!(
            ids.contains(&"priv-id"),
            "{uri}: every memory is public, private-marked row present too: {ids:?}"
        );
    }
}

/// Slug is the blog_posts PK, so re-publishing the same title — even from a
/// DIFFERENT surface — REPLACES the row rather than creating a duplicate.
/// T9 only publishes one post per slug; this is the cross-surface upsert path.
#[tokio::test]
async fn republish_same_title_replaces_row_across_surfaces() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let state = mock_state();
    // Operator identity: the only caller allowed to publish plain fields.
    let operator = state.keypair.pubkey_base58();
    let app = build_router(state, oauth_state.clone());
    let token = oauth::issue_jwt(&oauth_state, &operator).expect("issue_jwt");

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
    let state = mock_state();
    // Operator identity: the only caller allowed to publish plain fields.
    let operator = state.keypair.pubkey_base58();
    let app = build_router(state, oauth_state.clone());

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
    let token = oauth::issue_jwt(&oauth_state, &operator).expect("issue_jwt");
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
    let state = mock_state();
    // Operator identity: the only caller allowed to publish plain fields.
    let operator = state.keypair.pubkey_base58();
    let app = build_router(state, oauth_state.clone());
    let token = oauth::issue_jwt(&oauth_state, &operator).expect("issue_jwt");

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

/// Publish is non-custodial: the AUTHOR signs the POST_V1, the live pipeline
/// only verifies it. The stored content_hash is the author's signed hash, and
/// the signer of record is the author, never the server.
#[tokio::test]
async fn published_attestation_is_author_signed_via_live_path() {
    use mnemonic_core::codec::sign::verify_artifact;
    use mnemonic_mcp::test_support::sign_post;
    use solana_sdk::signature::{Keypair, Signer};

    let state = mock_state();
    let author = Keypair::new();
    let author_pk = author.pubkey().to_string();
    let cose = sign_post(
        &author,
        "Verifiable Post",
        "# Heading\n\nverifiable body",
        &["proof"],
        "Author Name",
    );
    let expected = verify_artifact(&cose, None).expect("verify");

    let post = publish::publish_post(
        &state,
        &author_pk,
        PublishInput {
            title: String::new(),
            body_markdown: String::new(),
            tags: vec![],
            author: None,
            signed_post: Some(cose),
        },
    )
    .expect("live publish");

    assert_eq!(post.content_hash, expected.content_hash);
    assert_eq!(post.title, "Verifiable Post");
    assert_eq!(post.author, "Author Name");
    assert_ne!(author_pk, state.keypair.pubkey_base58());
}

/// A non-operator caller sending plain fields is refused: the server must
/// never sign a post on a user's behalf.
#[tokio::test]
async fn unsigned_publish_from_user_is_rejected() {
    let state = mock_state();
    let err = publish::publish_post(
        &state,
        "SomeUserPubkey",
        PublishInput {
            title: "Forged".into(),
            body_markdown: "server must not sign this".into(),
            tags: vec![],
            author: None,
            signed_post: None,
        },
    )
    .expect_err("must reject");
    assert!(err.message().contains("signed_post"), "{}", err.message());
}

/// A post signed by one key cannot be published under another identity.
#[tokio::test]
async fn signed_post_from_other_identity_is_rejected() {
    use mnemonic_mcp::test_support::sign_post;
    use solana_sdk::signature::Keypair;

    let state = mock_state();
    let cose = sign_post(&Keypair::new(), "Stolen", "body", &[], "x");
    let err = publish::publish_post(
        &state,
        "VictimPubkey",
        PublishInput {
            title: String::new(),
            body_markdown: String::new(),
            tags: vec![],
            author: None,
            signed_post: Some(cose),
        },
    )
    .expect_err("must reject");
    assert!(
        err.message().contains("does not match"),
        "{}",
        err.message()
    );
}

/// One author cannot overwrite another author's post by reusing its title.
#[tokio::test]
async fn other_author_cannot_replace_post_by_same_slug() {
    use mnemonic_mcp::test_support::sign_post;
    use solana_sdk::signature::{Keypair, Signer};

    let state = mock_state();
    let publish = |kp: &Keypair, body: &str| {
        publish::publish_post(
            &state,
            &kp.pubkey().to_string(),
            PublishInput {
                title: String::new(),
                body_markdown: String::new(),
                tags: vec![],
                author: None,
                signed_post: Some(sign_post(kp, "Shared Title", body, &[], "")),
            },
        )
    };
    let alice = Keypair::new();
    let mallory = Keypair::new();
    publish(&alice, "original").expect("alice publishes");
    let err = publish(&mallory, "hijack").expect_err("mallory must be refused");
    assert!(
        err.message().contains("another author"),
        "{}",
        err.message()
    );
    publish(&alice, "edited").expect("owner may replace own post");
}

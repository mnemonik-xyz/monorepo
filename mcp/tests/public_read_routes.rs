//! Integration tests for the webapp-rethink Task 8 public read routes:
//! `GET /artifacts`, `GET /analytics/attestations`, `GET /blog`, and
//! `GET /blog/:slug`.
//!
//! These endpoints are unauthenticated JSON surfaces consumed cross-origin by
//! the webapp. The critical guarantee under test is the privacy invariant
//! (Decision 6): `/artifacts` exposes `visibility = public` rows ONLY — never a
//! private or NULL-owner row, on either the plain-list or `?q=` search path.
//!
//! TDD anchors (tasks/8.md):
//!   - `/artifacts` returns only public rows and matches the wire shape.
//!   - `/analytics` buckets/totals correct per range.
//!   - `/blog` + `/blog/:slug` return expected JSON; unknown slug → 404.
//!
//! Built against `test_support::mock_state()` (in-memory SQLite + StubEmbedder)
//! and exercised via `tower::ServiceExt::oneshot` so no socket is bound.

#![cfg(feature = "test-support")]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use mnemonic_mcp::{api, mcp::McpState, test_support::mock_state};
use serde_json::Value;
use tower::ServiceExt;

/// Mirror the production wiring from `main.rs::run_http`: the four read routes
/// on a base router carrying shared state. (CORS is a separate `tower-http`
/// layer in production; it does not affect the JSON body these tests assert
/// on, so it is omitted here — the CORS predicate itself is covered by
/// `cors_policy`'s own unit tests and `tests/cors.rs`.)
fn build_router(state: Arc<McpState>) -> Router {
    Router::new()
        .route("/artifacts", get(api::artifacts_handler))
        .route(
            "/analytics/attestations",
            get(api::analytics_attestations_handler),
        )
        .route("/blog", get(api::blog_list_handler))
        .route("/blog/{slug}", get(api::blog_post_handler))
        .with_state(state)
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed)
}

/// Seed one public + one private row under `owner`, both with the constant
/// StubEmbedder embedding so the `?q=` cosine path ranks them equally — the
/// only thing that can drop the private row is the visibility predicate.
fn seed_one_public_one_private(state: &Arc<McpState>, owner: &str) {
    let store = state.store.lock().expect("store");
    let embedding = vec![0.1f32; 8];
    store
        .save_attestation(
            "public-id",
            "public ledger row",
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
            "private-id",
            "private ledger row",
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

#[tokio::test]
async fn artifacts_plain_list_returns_public_only_with_shape() {
    let state = mock_state();
    let owner = "seed-owner-pubkey";
    seed_one_public_one_private(&state, owner);
    let app = build_router(state);

    let (status, body) = get_json(&app, "/artifacts").await;
    assert_eq!(status, StatusCode::OK);

    let artifacts = body["artifacts"].as_array().expect("artifacts array");
    assert_eq!(artifacts.len(), 1, "private row must not surface: {body}");
    assert_eq!(body["total"], 1);

    // Wire shape (lib/ledger.ts Artifact, with `attestation_id` key per the
    // PublicArtifact contract).
    let row = &artifacts[0];
    assert_eq!(row["attestation_id"], "public-id");
    assert_eq!(row["content"], "public ledger row");
    assert_eq!(row["content_hash"], "hash-pub");
    assert_eq!(row["tags"], serde_json::json!(["release"]));
    assert_eq!(row["solana_tx"], "5NfSolanaPubSig");
    assert_eq!(row["arweave_tx"], "ArweavePubTx");
    assert_eq!(row["created_at"], "2026-06-10T00:00:00Z");
    assert_eq!(row["write_mode"], "participate");
    // Private-only fields are never projected onto the public wire shape.
    assert!(row.get("visibility").is_none(), "visibility must not leak");
    assert!(row.get("relevance_score").is_none());
}

#[tokio::test]
async fn artifacts_search_query_returns_public_only() {
    let state = mock_state();
    let owner = "seed-owner-pubkey";
    seed_one_public_one_private(&state, owner);
    let app = build_router(state);

    // `?q=` routes through cosine search over the cross-owner public pool.
    let (status, body) = get_json(&app, "/artifacts?q=ledger&limit=10").await;
    assert_eq!(status, StatusCode::OK);

    let artifacts = body["artifacts"].as_array().expect("artifacts array");
    assert_eq!(
        artifacts.len(),
        1,
        "search must surface public rows only: {body}"
    );
    assert_eq!(artifacts[0]["attestation_id"], "public-id");
    assert!(artifacts[0].get("relevance_score").is_none());
}

#[tokio::test]
async fn artifacts_empty_store_is_empty_list() {
    let state = mock_state();
    let app = build_router(state);
    let (status, body) = get_json(&app, "/artifacts").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts"].as_array().unwrap().len(), 0);
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn analytics_buckets_and_totals_by_write_mode() {
    let state = mock_state();
    let owner = "seed-owner-pubkey";
    {
        let store = state.store.lock().expect("store");
        let embedding = vec![0.1f32; 8];
        // Two on-node (local) + one on-chain (participate) on the same UTC day.
        for (i, (mode, vis)) in [
            (WriteMode::Local, Visibility::Private),
            (WriteMode::Local, Visibility::Public),
            (WriteMode::Participate, Visibility::Public),
        ]
        .into_iter()
        .enumerate()
        {
            store
                .save_attestation(
                    &format!("a{i}"),
                    "row",
                    &format!("h{i}"),
                    &[],
                    "local:x",
                    "local:y",
                    owner,
                    owner,
                    "2026-06-15T00:00:00Z",
                    mode,
                    vis,
                    &embedding,
                )
                .expect("seed");
        }
    }
    let app = build_router(state);

    let (status, body) = get_json(&app, "/analytics/attestations?range=90d").await;
    assert_eq!(status, StatusCode::OK);

    let buckets = body["buckets"].as_array().expect("buckets array");
    assert_eq!(buckets.len(), 1, "all rows on one UTC day: {body}");
    assert_eq!(buckets[0]["date"], "2026-06-15");
    assert_eq!(buckets[0]["on_node"], 2);
    assert_eq!(buckets[0]["on_chain"], 1);
    assert_eq!(body["total_on_node"], 2);
    assert_eq!(body["total_on_chain"], 1);
    // unique_users present (all-time distinct owner count).
    assert!(body["unique_users"].is_i64());
}

#[tokio::test]
async fn analytics_empty_store_zero_totals() {
    let state = mock_state();
    let app = build_router(state);
    let (status, body) = get_json(&app, "/analytics/attestations?range=30d").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["buckets"].as_array().unwrap().len(), 0);
    assert_eq!(body["total_on_node"], 0);
    assert_eq!(body["total_on_chain"], 0);
}

#[tokio::test]
async fn blog_list_and_detail_roundtrip() {
    let state = mock_state();
    {
        let store = state.store.lock().expect("store");
        store
            .upsert_blog_post(
                "hello-world",
                "Hello World",
                "## Body\n\nmarkdown here",
                &["changelog".to_string()],
                "research-agent-01",
                "attestation-xyz",
                "hash-blog",
                "2026-06-20T10:00:00Z",
            )
            .expect("seed blog post");
    }
    let app = build_router(state);

    // List.
    let (status, body) = get_json(&app, "/blog").await;
    assert_eq!(status, StatusCode::OK);
    let posts = body["posts"].as_array().expect("posts array");
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0]["slug"], "hello-world");
    assert_eq!(posts[0]["title"], "Hello World");
    assert_eq!(posts[0]["author"], "research-agent-01");
    assert_eq!(posts[0]["tags"], serde_json::json!(["changelog"]));

    // Detail.
    let (status, body) = get_json(&app, "/blog/hello-world").await;
    assert_eq!(status, StatusCode::OK);
    let post = &body["post"];
    assert_eq!(post["slug"], "hello-world");
    assert_eq!(post["body_markdown"], "## Body\n\nmarkdown here");
    assert_eq!(post["content_hash"], "hash-blog");
    assert_eq!(post["attestation_id"], "attestation-xyz");
}

#[tokio::test]
async fn blog_unknown_slug_is_404() {
    let state = mock_state();
    let app = build_router(state);
    let (status, body) = get_json(&app, "/blog/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["slug"], "does-not-exist");
    assert!(body["error"].is_string());
}

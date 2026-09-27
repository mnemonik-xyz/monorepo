//! Integration tests for the webapp-rethink Task 8 public read routes:
//! `GET /artifacts`, `GET /analytics/attestations`, `GET /blog`, and
//! `GET /blog/:slug`.
//!
//! These endpoints are unauthenticated JSON surfaces consumed by the webapp.
//! Private means private (owner decision 2026-09-27): `/artifacts` returns
//! only `visibility = 'public'` DB rows on the plain-list and `?q=` search
//! paths, for every `source` filter. A chain-snapshot item that matches a
//! private DB row is dropped; a chain-only item stays (public on Arweave).
//!
//! TDD anchors (tasks/8.md):
//!   - `/artifacts` returns public rows only and matches the wire shape.
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
use mnemonic_core::arweave::recovery::{RecoveredEmbedding, RecoveredItem};
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use mnemonic_mcp::chain_stats::ChainStatsCache;
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
            WriteMode::Anchored,
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
async fn artifacts_plain_list_returns_public_rows_only_with_shape() {
    let state = mock_state();
    let owner = "seed-owner-pubkey";
    seed_one_public_one_private(&state, owner);
    let app = build_router(state);

    // No source filter returns a private row.
    for uri in [
        "/artifacts?source=all",
        "/artifacts?source=on_node",
        "/artifacts?source=on_chain",
    ] {
        let (status, body) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.to_string().contains("private ledger row"),
            "{uri}: private content leaked: {body}"
        );
    }

    let (status, body) = get_json(&app, "/artifacts").await;
    assert_eq!(status, StatusCode::OK);

    let artifacts = body["artifacts"].as_array().expect("artifacts array");
    assert_eq!(artifacts.len(), 1, "public rows only: {body}");
    assert_eq!(body["total"], 1);

    // Wire shape (lib/ledger.ts Artifact, with `attestation_id` key per the
    // PublicArtifact contract). Locate the seeded public row regardless of order.
    let row = artifacts
        .iter()
        .find(|r| r["attestation_id"] == "public-id")
        .expect("public-id row present");
    assert_eq!(row["content"], "public ledger row");
    assert_eq!(row["content_hash"], "hash-pub");
    assert_eq!(row["tags"], serde_json::json!(["release"]));
    assert_eq!(row["solana_tx"], "5NfSolanaPubSig");
    assert_eq!(row["arweave_tx"], "ArweavePubTx");
    assert_eq!(row["created_at"], "2026-06-10T00:00:00Z");
    assert_eq!(row["write_mode"], "anchored");
    // Owner decision D-8: an anchored row says its content is plain text on
    // Arweave.
    assert_eq!(row["plaintext_on_arweave"], true);
    // The `visibility` and `relevance_score` fields are not part of the wire shape.
    assert!(
        row.get("visibility").is_none(),
        "visibility not in wire shape"
    );
    assert!(row.get("relevance_score").is_none());
}

#[tokio::test]
async fn artifacts_search_query_returns_public_rows_only() {
    let state = mock_state();
    let owner = "seed-owner-pubkey";
    seed_one_public_one_private(&state, owner);
    let app = build_router(state);

    // `?q=` routes through cosine search over the cross-owner public pool.
    let (status, body) = get_json(&app, "/artifacts?q=ledger&limit=10").await;
    assert_eq!(status, StatusCode::OK);

    let artifacts = body["artifacts"].as_array().expect("artifacts array");
    let ids: Vec<&str> = artifacts
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(ids, vec!["public-id"], "public rows only: {body}");
    assert!(artifacts[0].get("relevance_score").is_none());

    for source in ["all", "on_node", "on_chain"] {
        let uri = format!("/artifacts?q=ledger&limit=10&source={source}");
        let (_, body) = get_json(&app, &uri).await;
        assert!(
            !body.to_string().contains("private ledger row"),
            "{uri}: private content leaked: {body}"
        );
    }
}

// ── Recall over chain-recovered anchored memories (#201) ─────────────────

/// Attach a pre-loaded chain snapshot to a fresh `mock_state()`. The state
/// is not shared yet, so `Arc::get_mut` succeeds.
fn state_with_chain(items: Vec<RecoveredItem>) -> Arc<McpState> {
    let mut state = mock_state();
    Arc::get_mut(&mut state)
        .expect("state not shared yet")
        .chain_stats = Some(Arc::new(ChainStatsCache::from_items(items)));
    state
}

fn chain_item(
    tx: &str,
    hash: &str,
    content: &str,
    embedding: Option<RecoveredEmbedding>,
) -> RecoveredItem {
    RecoveredItem {
        arweave_tx: tx.to_string(),
        solana_tx: Some(format!("sol-{tx}")),
        content_hash: Some(hash.to_string()),
        content: Some(content.to_string()),
        tags: Vec::new(),
        day: Some("2026-07-01".to_string()),
        producer: None,
        embedding,
    }
}

/// TurboQuant bytes as `mock_state()`'s compressor (8 dims, 4 bits, seed
/// 42) would have written them into `metadata.embedding_compressed`.
fn compressed(v: &[f32]) -> Option<RecoveredEmbedding> {
    let c = mnemonic_core::compress::EmbeddingCompressor::new(8, 4, 42);
    Some(RecoveredEmbedding::Compressed(c.compress(v).to_bytes()))
}

fn rows_by_tx(body: &Value) -> Vec<(String, String)> {
    body["artifacts"]
        .as_array()
        .expect("artifacts array")
        .iter()
        .map(|r| {
            (
                r["arweave_tx"].as_str().unwrap_or("").to_string(),
                r["match"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// Production repro: `?q=memory&source=on_chain` returned `[]` while the
/// plain On-chain listing showed 16 recovered items. With an empty DB the
/// query must now search the recovered snapshot itself.
#[tokio::test]
async fn artifacts_recall_finds_chain_recovered_items_with_empty_db() {
    // StubEmbedder embeds every query as [0.1; 8].
    let near = [0.1f32; 8];
    let far = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
    let app = build_router(state_with_chain(vec![
        chain_item("tx-near", "h1", "close in meaning", compressed(&near)),
        chain_item("tx-far", "h2", "far in meaning", compressed(&far)),
        chain_item("tx-text", "h3", "a recovered Memory", None),
        chain_item("tx-miss", "h4", "nothing to see", None),
    ]));

    for source in ["on_chain", "all"] {
        let uri = format!("/artifacts?q=memory&limit=10&source={source}");
        let (status, body) = get_json(&app, &uri).await;
        assert_eq!(status, StatusCode::OK);
        let rows = rows_by_tx(&body);
        let pos = |tx: &str| rows.iter().position(|(t, _)| t == tx);
        assert_eq!(rows.len(), 3, "[{source}] {body}");
        assert_eq!(body["total"], 3);
        assert!(pos("tx-miss").is_none(), "no embedding + no text match");
        assert!(pos("tx-near") < pos("tx-far"), "ranked by cosine: {body}");
        let kind = |tx: &str| rows[pos(tx).unwrap()].1.clone();
        assert_eq!(kind("tx-near"), "semantic");
        assert_eq!(kind("tx-far"), "semantic");
        assert_eq!(kind("tx-text"), "text", "fallback is labelled");

        let text_row = &body["artifacts"][pos("tx-text").unwrap()];
        assert_eq!(text_row["write_mode"], "anchored");
        assert_eq!(text_row["content_hash"], "h3");
        assert_eq!(text_row["solana_tx"], "sol-tx-text");
    }

    // `limit` still caps the combined page.
    let (_, body) = get_json(&app, "/artifacts?q=memory&limit=1&source=on_chain").await;
    assert_eq!(rows_by_tx(&body).len(), 1);

    // Empty query keeps the complete recovered listing.
    let (_, body) = get_json(&app, "/artifacts?limit=10&source=on_chain").await;
    assert_eq!(body["artifacts"].as_array().unwrap().len(), 4, "{body}");
    assert!(body["artifacts"][0].get("match").is_none());

    // On-node recall never includes chain items.
    let (_, body) = get_json(&app, "/artifacts?q=memory&source=on_node").await;
    assert_eq!(body["total"], 0, "{body}");
}

/// `source=all` must union SQLite and chain matches without duplicates. A
/// chain item repeats a DB row when it has the same `arweave_tx` or the
/// same `content_hash`; the DB row wins.
#[tokio::test]
async fn artifacts_recall_source_all_dedupes_db_and_chain() {
    let state = state_with_chain(vec![
        chain_item("ArweavePubTx", "hash-other", "memory dup by tx", None),
        chain_item("tx-hash-dup", "hash-pub", "memory dup by hash", None),
        chain_item("tx-chain-only", "hash-chain", "memory chain only", None),
    ]);
    seed_one_public_one_private(&state, "seed-owner-pubkey");
    let app = build_router(state);

    let (status, body) = get_json(&app, "/artifacts?q=memory&limit=10&source=all").await;
    assert_eq!(status, StatusCode::OK);
    let rows = rows_by_tx(&body);
    let txs: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(rows.len(), 2, "{body}");
    assert_eq!(txs.iter().filter(|t| **t == "ArweavePubTx").count(), 1);
    assert!(!txs.contains(&"tx-hash-dup"), "same content_hash as DB row");
    assert!(txs.contains(&"tx-chain-only"));
    assert!(
        !txs.contains(&"local:priv-ar"),
        "private DB row is not served"
    );

    let db_row = body["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["arweave_tx"] == "ArweavePubTx")
        .unwrap();
    assert_eq!(db_row["attestation_id"], "public-id", "DB row wins");
    assert_eq!(db_row["match"], "semantic");

    // `on_chain` keeps the anchored DB row + chain-only item, drops the
    // local row.
    let (_, body) = get_json(&app, "/artifacts?q=memory&limit=10&source=on_chain").await;
    let mut txs: Vec<String> = rows_by_tx(&body).into_iter().map(|(t, _)| t).collect();
    txs.sort();
    assert_eq!(txs, vec!["ArweavePubTx", "tx-chain-only"], "{body}");
}

/// A chain-snapshot item that matches a private DB row (by Arweave tx id or
/// by content hash) is dropped from the plain listing and from `?q=`, for
/// every source. A chain-only item with no DB row stays.
#[tokio::test]
async fn artifacts_chain_items_matching_private_rows_are_dropped() {
    let state = state_with_chain(vec![
        chain_item("ArweavePrivTx", "hash-x", "memory secret by tx", None),
        chain_item(
            "tx-by-hash",
            "hash-priv-anchored",
            "memory secret by hash",
            None,
        ),
        chain_item("tx-chain-only", "hash-chain", "memory chain only", None),
    ]);
    {
        let store = state.store.lock().expect("store");
        store
            .save_attestation(
                "priv-anchored",
                "memory secret db row",
                "hash-priv-anchored",
                &[],
                "sol-priv",
                "ArweavePrivTx",
                "owner-a",
                "owner-a",
                "2026-06-12T00:00:00Z",
                // A demoted row: its bytes went to Arweave, but the delivery
                // check failed, so the row is `local` and stays private.
                WriteMode::Local,
                Visibility::Private,
                &[0.1f32; 8],
            )
            .expect("seed private anchored");
    }
    let app = build_router(state);

    for uri in [
        "/artifacts?limit=10",
        "/artifacts?limit=10&source=all",
        "/artifacts?limit=10&source=on_chain",
        "/artifacts?limit=10&source=on_node",
        "/artifacts?q=memory&limit=10",
        "/artifacts?q=memory&limit=10&source=all",
        "/artifacts?q=memory&limit=10&source=on_chain",
        "/artifacts?q=memory&limit=10&source=on_node",
    ] {
        let (status, body) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.to_string().contains("secret"),
            "{uri}: private content leaked: {body}"
        );
        let txs: Vec<String> = rows_by_tx(&body).into_iter().map(|(t, _)| t).collect();
        if uri.contains("on_node") {
            assert!(txs.is_empty(), "{uri}: {body}");
        } else {
            assert_eq!(txs, vec!["tx-chain-only"], "{uri}: {body}");
        }
    }
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
    // Seed yesterday (UTC), not a fixed date: a fixed date drifts out of the
    // 90-day window and the test starts to fail on its own.
    let day = (chrono::Utc::now() - chrono::Duration::days(1)).date_naive();
    let created_at = format!("{day}T12:00:00Z");
    {
        let store = state.store.lock().expect("store");
        let embedding = vec![0.1f32; 8];
        // Two on-node (local) + one on-chain (anchored) on the same UTC day.
        for (i, (mode, vis)) in [
            (WriteMode::Local, Visibility::Private),
            (WriteMode::Local, Visibility::Public),
            (WriteMode::Anchored, Visibility::Public),
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
                    &created_at,
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
    assert_eq!(buckets[0]["date"], day.to_string());
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

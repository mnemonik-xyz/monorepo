//! Integration tests for the visibility-filter anonymous recall path
//! (Task 4 / Decision 5 / AC13 — agent-native-distribution).
//!
//! Two TDD anchors:
//!
//! 1. `filters_private_rows` — seed DB with 1 private + 1 public row that
//!    both match a recall query string; call `recall` without
//!    `Authorization`; only the public row appears in the response.
//! 2. `authenticated_recall_returns_both` — same DB, call recall with a
//!    valid Bearer for the owner; both rows appear.
//!
//! `recall` is wired through the per-tool allowlist in
//! `oauth::bearer_auth_middleware` so anonymous `tools/call mnemonic_recall`
//! is reachable; the dispatcher's `visibility_filter` predicate adds the
//! `Some(Public)` argument when `jwt_sub.is_none()` — AC13's "only public
//! rows" surfaces through that storage filter.

#![cfg(feature = "test-support")]

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use serde_json::json;

/// Seed two rows under `owner` matching the query "shared keyword": one
/// `Visibility::Private`, one `Visibility::Public`. Identical embedding so
/// both rank equally on cosine similarity — the test asserts the
/// visibility predicate is what drops the private row, not the score.
fn seed_one_private_one_public(server: &TestServer, owner: &str) {
    let store = server.state.store.lock().expect("store");
    // Same constant embedding StubEmbedder uses — guarantees both rows
    // surface equally in the cosine scoring.
    let embedding = vec![0.1f32; 8];

    store
        .save_attestation(
            "private-id",
            "shared keyword private row",
            "hash-priv",
            &["seed".to_string()],
            "local:priv",
            "local:priv-ar",
            owner,
            owner,
            "2026-06-04T00:00:00Z",
            WriteMode::Participate,
            Visibility::Private,
            &embedding,
        )
        .expect("seed private");

    store
        .save_attestation(
            "public-id",
            "shared keyword public row",
            "hash-pub",
            &["seed".to_string()],
            "local:pub",
            "local:pub-ar",
            owner,
            owner,
            "2026-06-04T00:00:01Z",
            WriteMode::Participate,
            Visibility::Public,
            &embedding,
        )
        .expect("seed public");
}

#[tokio::test]
async fn filters_private_rows() {
    let server = TestServer::builder().build();
    // The dispatcher's owner_pubkey fallback for anonymous callers is the
    // server keypair (see mcp::mcp_handler). Seed both rows under that
    // pubkey so the search query sees them at all (the owner predicate
    // is still applied by storage::search).
    let owner = server.server_pubkey();
    seed_one_private_one_public(&server, &owner);

    // Anonymous — no `sub`, no Authorization. The middleware allowlists
    // tools/call mnemonic_recall under Decision 5/AC13.
    let result = server
        .call_tool(
            None,
            "mnemonic_recall",
            json!({ "query": "shared keyword", "limit": 10 }),
        )
        .await;

    assert_eq!(
        result.status,
        axum::http::StatusCode::OK,
        "anonymous recall must be 200: {:?}",
        result.envelope,
    );
    let inner = result.result_text();
    let rows = inner["results"].as_array().expect("results array");
    // Only the public row appears. The private row was filtered server-
    // side via `AND a.visibility = 'public'`.
    assert_eq!(
        rows.len(),
        1,
        "anonymous recall returns exactly 1 row: {inner}"
    );
    assert_eq!(rows[0]["attestation_id"], "public-id");
    assert_eq!(rows[0]["visibility"], "public");
}

#[tokio::test]
async fn authenticated_recall_returns_both() {
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    seed_one_private_one_public(&server, &owner);

    // Authenticated — Bearer JWT bound to `owner`. The dispatcher passes
    // `None` to the visibility filter, so both rows appear.
    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_recall",
            json!({ "query": "shared keyword", "limit": 10 }),
        )
        .await;

    assert_eq!(result.status, axum::http::StatusCode::OK);
    let inner = result.result_text();
    let rows = inner["results"].as_array().expect("results array");
    assert_eq!(
        rows.len(),
        2,
        "authenticated recall returns both rows: {inner}"
    );
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert!(ids.contains(&"public-id"));
    assert!(ids.contains(&"private-id"));
}

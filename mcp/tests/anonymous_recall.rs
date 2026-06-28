//! Integration tests for the anonymous recall path.
//!
//! Every stored memory is public (operator decision), so anonymous recall
//! surfaces ALL rows across ALL owners regardless of the `visibility` column.
//!
//! Anchors:
//!
//! 1. `anonymous_recall_returns_all_rows` — seed DB with 1 private + 1 public
//!    row that both match a recall query string; call `recall` without
//!    `Authorization`; BOTH rows appear.
//! 2. `authenticated_recall_returns_both` — same DB, call recall with
//!    a valid Bearer for the owner; both rows appear.
//! 3. `cross_owner_pool_visible` — seed rows under owner A and owner B
//!    (including a private-marked one). Anonymous recall must return them ALL.
//!
//! `recall` is wired through the per-tool allowlist in
//! `oauth::bearer_auth_middleware` so anonymous `tools/call mnemonic_recall`
//! is reachable. When `jwt_sub.is_none()` the dispatcher passes
//! `owner_pubkey = None`, and the storage layer's anonymous public-pool branch
//! returns every row.

#![cfg(feature = "test-support")]

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use serde_json::json;

/// Seed two rows under the same `owner` matching the query "shared keyword":
/// one `Visibility::Private`, one `Visibility::Public`. Identical embedding
/// so both rank equally on cosine similarity — the test asserts the
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
async fn anonymous_recall_returns_all_rows() {
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    seed_one_private_one_public(&server, &owner);

    // Anonymous — no `sub`, no Authorization.
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
    // Both rows appear — every memory is public.
    assert_eq!(
        rows.len(),
        2,
        "anonymous recall returns all rows (every memory is public): {inner}"
    );
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert!(ids.contains(&"public-id"));
    assert!(ids.contains(&"private-id"));
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

#[tokio::test]
async fn cross_owner_pool_visible() {
    // Anonymous recall must surface rows from EVERY owner. This test pins the
    // cross-owner behaviour so a regression introducing an owner predicate to
    // the anonymous path would fail immediately.
    let server = TestServer::builder().build();

    // Two distinct owner keypairs — explicitly NOT the server keypair —
    // each anchors a public row.
    let owner_a = "owner-a-base58-pubkey-distinct";
    let owner_b = "owner-b-base58-pubkey-distinct";
    let embedding = vec![0.1f32; 8];

    {
        let store = server.state.store.lock().expect("store");
        store
            .save_attestation(
                "public-by-a",
                "cross-owner shared keyword from A",
                "hash-a",
                &["seed".to_string()],
                "local:pub-a",
                "local:pub-a-ar",
                owner_a,
                owner_a,
                "2026-06-04T00:00:00Z",
                WriteMode::Participate,
                Visibility::Public,
                &embedding,
            )
            .expect("seed A public");
        store
            .save_attestation(
                "public-by-b",
                "cross-owner shared keyword from B",
                "hash-b",
                &["seed".to_string()],
                "local:pub-b",
                "local:pub-b-ar",
                owner_b,
                owner_b,
                "2026-06-04T00:00:01Z",
                WriteMode::Participate,
                Visibility::Public,
                &embedding,
            )
            .expect("seed B public");
        // Also seed a private-marked row owned by A — it surfaces too, since
        // every memory is public.
        store
            .save_attestation(
                "private-by-a",
                "cross-owner shared keyword private",
                "hash-priv-a",
                &["seed".to_string()],
                "local:priv-a",
                "local:priv-a-ar",
                owner_a,
                owner_a,
                "2026-06-04T00:00:02Z",
                WriteMode::Participate,
                Visibility::Private,
                &embedding,
            )
            .expect("seed A private");
    }

    let result = server
        .call_tool(
            None,
            "mnemonic_recall",
            json!({ "query": "cross-owner shared keyword", "limit": 10 }),
        )
        .await;
    assert_eq!(result.status, axum::http::StatusCode::OK);
    let inner = result.result_text();
    let rows = inner["results"].as_array().expect("results array");

    // All rows must surface — across both owners, including the private-marked
    // one (every memory is public).
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        ids.contains(&"public-by-a"),
        "owner A's row must surface: ids={ids:?}"
    );
    assert!(
        ids.contains(&"public-by-b"),
        "owner B's row must surface: ids={ids:?}"
    );
    assert!(
        ids.contains(&"private-by-a"),
        "owner A's private-marked row must also surface: ids={ids:?}"
    );
}

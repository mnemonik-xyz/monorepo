//! Integration tests for the anonymous recall path.
//!
//! Private means private (owner decision 2026-09-27): anonymous recall
//! surfaces only `visibility = 'public'` rows, across all owners. A private
//! row goes only to its owner.
//!
//! Anchors:
//!
//! 1. `anonymous_recall_returns_public_rows_only` — seed DB with 1 private +
//!    1 public row that both match a recall query string; call `recall`
//!    without `Authorization`; ONLY the public row appears.
//! 2. `authenticated_recall_returns_both` — same DB, call recall with
//!    a valid Bearer for the owner; both rows appear.
//! 3. `cross_owner_pool_visible` — seed public rows under owner A and owner B
//!    plus a private row of A. Anonymous recall returns both public rows and
//!    never the private one.
//! 4. `foreign_public_rows_not_in_authenticated_recall` — an authenticated
//!    caller sees only own rows (no other owner's rows, public or private).
//!
//! `recall` is wired through the per-tool allowlist in
//! `oauth::bearer_auth_middleware` so anonymous `tools/call mnemonic_recall`
//! is reachable. When `jwt_sub.is_none()` the dispatcher passes
//! `owner_pubkey = None` + `Some(Visibility::Public)`, and the storage layer
//! binds `visibility = 'public'` in SQL.

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
async fn anonymous_recall_returns_public_rows_only() {
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
    // Only the public row appears — a private row goes only to its owner.
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(ids, vec!["public-id"], "anonymous recall: {inner}");
    assert!(
        !inner.to_string().contains("private row"),
        "private content must not appear anywhere in the response: {inner}"
    );
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
        // Also seed a private row owned by A — it must NOT surface.
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

    // Both owners' public rows surface; the private row does not.
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
        !ids.contains(&"private-by-a"),
        "owner A's private row must not surface: ids={ids:?}"
    );
    assert_eq!(ids.len(), 2, "ids={ids:?}");
}

#[tokio::test]
async fn foreign_public_rows_not_in_authenticated_recall() {
    // Authenticated recall is owner-scoped: own rows of any visibility, and
    // no rows of other owners (public or private). Pins the current scope so
    // this PR does not widen it.
    let server = TestServer::builder().build();
    let me = server.server_pubkey();
    seed_one_private_one_public(&server, &me);
    let other = "other-owner-base58-pubkey";
    {
        let store = server.state.store.lock().expect("store");
        store
            .save_attestation(
                "foreign-public",
                "shared keyword foreign public",
                "hash-foreign",
                &["seed".to_string()],
                "local:foreign",
                "local:foreign-ar",
                other,
                other,
                "2026-06-04T00:00:05Z",
                WriteMode::Local,
                Visibility::Public,
                &[0.1f32; 8],
            )
            .expect("seed foreign");
    }

    let result = server
        .call_tool(
            Some(&me),
            "mnemonic_recall",
            json!({ "query": "shared keyword", "limit": 10 }),
        )
        .await;
    assert_eq!(result.status, axum::http::StatusCode::OK);
    let inner = result.result_text();
    let mut ids: Vec<&str> = inner["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["private-id", "public-id"], "{inner}");

    // The same foreign public row IS in the anonymous pool.
    let anon = server
        .call_tool(
            None,
            "mnemonic_recall",
            json!({ "query": "shared keyword", "limit": 10 }),
        )
        .await;
    let anon_inner = anon.result_text();
    let anon_ids: Vec<&str> = anon_inner["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert!(anon_ids.contains(&"foreign-public"), "{anon_inner}");
    assert!(!anon_ids.contains(&"private-id"), "{anon_inner}");
}

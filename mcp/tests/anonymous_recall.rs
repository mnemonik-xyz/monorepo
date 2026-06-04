//! Integration tests for the visibility-filter anonymous recall path
//! (Task 4 / Decision 5 / AC13 — agent-native-distribution).
//!
//! Anchors:
//!
//! 1. `filters_private_rows` — seed DB with 1 private + 1 public row
//!    that both match a recall query string; call `recall` without
//!    `Authorization`; only the public row appears.
//! 2. `authenticated_recall_returns_both` — same DB, call recall with
//!    a valid Bearer for the owner; both rows appear.
//! 3. `cross_owner_public_pool_visible` — round 2 / SAR1-M1: seed a
//!    public row under owner A and a public row under owner B. Anonymous
//!    recall must return BOTH rows. The round-1 implementation scoped
//!    anonymous recall to the server keypair, hiding rows owned by any
//!    other user; round 2 drops the owner predicate when `jwt_sub` is
//!    absent so the cross-user public pool surfaces correctly.
//!
//! `recall` is wired through the per-tool allowlist in
//! `oauth::bearer_auth_middleware` so anonymous `tools/call mnemonic_recall`
//! is reachable. The dispatcher passes `owner_pubkey = None` AND
//! `visibility_filter = Some(Public)` when `jwt_sub.is_none()` —
//! AC13's "public part of the pool" surfaces through the dropped owner
//! predicate paired with the public visibility filter.

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
async fn filters_private_rows() {
    let server = TestServer::builder().build();
    // The owner_pubkey chosen for seeding is independent of any dispatcher
    // fallback — round 2 / SAR1-M1 dropped the owner predicate for
    // anonymous-public recall, so the seeded rows surface regardless of
    // owner. We still use the server keypair here for symmetry with
    // `authenticated_recall_returns_both` (which DOES depend on the owner
    // matching the JWT sub).
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

#[tokio::test]
async fn cross_owner_public_pool_visible() {
    // SAR1-M1 (round 1 security audit, agent-native-distribution Task 4):
    // anonymous recall must surface public rows from EVERY owner, not just
    // the server keypair. Round-1 scoped to the server keypair, hiding
    // other users' public rows entirely. This test pins the cross-owner
    // behaviour so a regression introducing an owner predicate to the
    // anonymous-public path would fail immediately.
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
        // Also seed a private row owned by A — to prove the visibility
        // filter still gates these out for anonymous callers regardless
        // of owner.
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

    // Both public rows must surface — across both owners. The private
    // row from owner A must NOT surface.
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["attestation_id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        ids.contains(&"public-by-a"),
        "owner A's public row must surface: ids={ids:?}"
    );
    assert!(
        ids.contains(&"public-by-b"),
        "owner B's public row must surface: ids={ids:?}"
    );
    assert!(
        !ids.contains(&"private-by-a"),
        "owner A's private row must NOT surface anonymously: ids={ids:?}"
    );
    assert!(
        rows.iter().all(|r| r["visibility"] == "public"),
        "every anonymous-recall row must be visibility=public: {inner}"
    );
}

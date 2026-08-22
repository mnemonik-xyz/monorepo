//! T4 integration tests — `verify` routes by stored `write_mode` and is
//! tenant-isolated on the storage lookup.
//!
//! Four scenarios:
//!
//! 1. `verify_routes_local_for_local_row` — a row tagged
//!    `write_mode='local'` produces the local-shape envelope regardless of
//!    the operator's `STORAGE_MODE`. Pins the routing-by-stored-mode rule.
//! 2. `verify_routes_participate_for_participate_row` — a row tagged
//!    `write_mode='participate'` produces the participate-shape envelope
//!    regardless of `STORAGE_MODE`. Same pin from the other side.
//! 3. `tenant_isolation_local` / `tenant_isolation_participate` — caller A
//!    writes a row, caller B asks to verify A's `solana_tx`. Must return
//!    the `not_found` shape with NO leakage of `content_hash`,
//!    `signer_pubkey`, content, or preview fields. Repeated for both
//!    `local` and `participate` row shapes.
//! 4. `recall_surfaces_write_mode` — seed mixed-mode rows under one owner;
//!    `recall` must surface each row's stored `write_mode` tag.
//!
//! All tests exercise the public MCP tool dispatcher (POST /mcp) under the
//! same OAuth middleware as production; the multi-tenant scenarios use the
//! T4 `mint_test_jwt` / `with_token` harness primitives to drive two
//! distinct authenticated callers against one shared `SqliteStore`.

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use serde_json::json;

// Fixed pubkey-shaped strings for the multi-tenant tests. They don't need
// to be valid Ed25519 base58 — the OAuth middleware only checks the JWT
// signature and stuffs `sub` straight into `owner_pubkey`.
const USER_A_PUBKEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const USER_B_PUBKEY: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

// Leaky fields a verify response must NEVER contain when the caller is
// asking about another tenant's row. Asserted against EVERY tenant-
// isolation case to keep the contract explicit at the call site rather
// than hidden in a helper.
const LEAKY_FIELDS: &[&str] = &[
    "content_hash",
    "signer",
    "signer_pubkey",
    "content",
    "content_preview",
    "preview",
];

/// Seed a row directly via `save_attestation` so the test can pin a
/// specific `(owner, write_mode, solana_tx, arweave_tx)` quartet without
/// going through the env-dependent inline / deferred sign flow. Returns
/// the `solana_tx` and `arweave_tx` for the row.
///
/// The stored `content_hash` is the real thing — `blake3` over the canonical
/// CBOR artifact, built exactly as `sign_memory_inline` builds it — so a
/// happy-path local verify comes back `verified` and the test can attribute a
/// `tampered` result to routing rather than a synthetic fixture.
///
/// This previously stored `blake3(content)` instead, which matched the old
/// (broken) raw-content comparison in `verify_local` and so kept the routing
/// assertion green while every real row on disk reported `tampered`. Build the
/// fixture through the same construction as production, or the test proves
/// nothing about production.
fn seed_row(
    server: &TestServer,
    owner: &str,
    write_mode: WriteMode,
    seed_id: &str,
) -> (String, String) {
    let attestation_id = format!("att-{seed_id}");
    let (sol, ar) = match write_mode {
        WriteMode::Local => (
            format!("local:{seed_id}-sol"),
            format!("local:{seed_id}-ar"),
        ),
        WriteMode::Participate => (
            // Real-looking (non-`local:`) prefix so any downstream
            // discrimination by id-shape sees this as participate.
            format!("solx{seed_id}aRRRRYWfLh8x2y2y9X7Cxe1aN3aN3aN3aN3"),
            format!("PdhTPLPmHvX0iE6iAtJ8X5Y0WqQ8MzC8KvU9JhQ0aN0{seed_id}"),
        ),
    };
    let now = chrono::Utc::now().to_rfc3339();
    let content = format!("seeded content for {seed_id}");
    let embedding = [0.1f32; 8];
    let compressed = server.state.compressor.compress(&embedding);
    let artifact = serde_json::json!({
        "artifact_id": attestation_id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": format!("did:sol:{owner}"),
        "created_at": now,
        "tags": ["t"],
        "metadata": {
            "embed_provider": server.state.embedder.provider_name(),
            "embed_dim": embedding.len(),
            "turbo_bits": compressed.bit_width,
            "embedding_compressed": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                compressed.to_bytes(),
            ),
        },
    });
    let content_hash = mnemonic_core::codec::hash::hash_artifact(
        &artifact,
        &mnemonic_core::codec::schema::MEMORY_V1,
    )
    .expect("hash artifact");
    let store = server.state.store.lock().unwrap();
    store
        .save_attestation(
            &attestation_id,
            &content,
            &content_hash,
            &["t".to_string()],
            &sol,
            &ar,
            owner, // signer
            owner, // owner
            &now,
            write_mode,
            Visibility::Private,
            &embedding,
        )
        .expect("save row");
    (sol, ar)
}

// ── 1. Local-tagged rows always route to verify_local ──────────────────────

#[tokio::test]
async fn verify_routes_local_for_local_row() {
    // The routing decision must be driven by the row's stored
    // `write_mode`, not the operator's STORAGE_MODE env. We run the
    // assertion under both env values so a regression that re-introduces
    // env-driven branching trips at least one case.
    for storage_mode in ["local", "full"] {
        let server = TestServer::builder().storage_mode(storage_mode).build();
        let owner = server.server_pubkey();
        let (sol, _ar) = seed_row(&server, &owner, WriteMode::Local, "local-route");

        let result = server
            .call_tool(Some(&owner), "mnemonic_verify", json!({"solana_tx": sol}))
            .await;
        assert!(
            result.error().is_none(),
            "[{storage_mode}] envelope: {:?}",
            result.envelope
        );
        let inner = result.result_text();
        // verify_local always emits `storage_mode: "local"` regardless of
        // the operator's actual mode — this is the load-bearing shape that
        // distinguishes the local route from the participate route.
        assert_eq!(
            inner["storage_mode"], "local",
            "[{storage_mode}] expected local-shape envelope; got {inner:?}"
        );
        // Local rows route via `verify_local` which echoes the stored
        // solana_tx (and never reads from the Solana network).
        assert_eq!(
            inner["solana_tx"].as_str(),
            Some(sol.as_str()),
            "[{storage_mode}] expected local row's solana_tx echoed"
        );
        // Verified status is the happy-path expectation; tampering
        // detection is exercised elsewhere.
        assert_eq!(
            inner["status"], "verified",
            "[{storage_mode}] expected status=verified, got {inner:?}"
        );
    }
}

// ── 2. Participate-tagged rows always route to verify_participate ──────────

#[tokio::test]
async fn verify_routes_participate_for_participate_row() {
    // Same shape as #1 but from the other side. A `participate` row must
    // route to the Arweave / Solana fetch path even under
    // `STORAGE_MODE=local` — the env-var branch is gone in T4.
    //
    // The participate path reaches out to the configured `SolanaClient`,
    // which in test mode points at `http://localhost:0`. The relevant
    // assertion is therefore that we DID try the participate path — i.e.
    // the response shape is `anchor_not_found` (or an outright network
    // error) rather than the `local`-shape envelope. The shape contract
    // is enough to pin the routing decision; we don't re-test the deep
    // anchor-verify happy path here (that lives in the COSE round-trip
    // suite).
    for storage_mode in ["local", "full"] {
        let server = TestServer::builder().storage_mode(storage_mode).build();
        let owner = server.server_pubkey();
        let (sol, _ar) = seed_row(&server, &owner, WriteMode::Participate, "participate-route");

        let result = server
            .call_tool(Some(&owner), "mnemonic_verify", json!({"solana_tx": sol}))
            .await;

        // Two acceptable outcomes prove routing went to the participate
        // branch: (a) the tool returned a JSON-RPC error (network-fetch
        // failure on the test SolanaClient pointing at localhost:0), or
        // (b) the tool returned a `participate`-side status such as
        // `anchor_not_found`. EITHER outcome rules out the local route.
        if let Some(err) = result.error() {
            // The participate path bubbled a fetch failure; confirm the
            // message is NOT the local "provide solana_tx or arweave_tx"
            // signal.
            let msg = err["message"].as_str().unwrap_or("");
            assert!(
                !msg.contains("provide solana_tx"),
                "[{storage_mode}] participate path must not echo local error"
            );
        } else {
            let inner = result.result_text();
            // The local route stamps `storage_mode: "local"` on its
            // success/not_found envelope; participate route never does.
            assert_ne!(
                inner["storage_mode"], "local",
                "[{storage_mode}] participate row routed to LOCAL path: {inner:?}"
            );
            // The participate path emits one of these statuses on a
            // non-network test SolanaClient.
            let status = inner["status"].as_str().unwrap_or("");
            assert!(
                matches!(
                    status,
                    "anchor_not_found" | "arweave_not_found" | "verified" | "tampered" | "error"
                ),
                "[{storage_mode}] unexpected participate-path status: {inner:?}"
            );
        }
    }
}

// ── 3a. Tenant isolation: local row ────────────────────────────────────────

#[tokio::test]
async fn tenant_isolation_local() {
    // Caller A owns a local-tagged row. Caller B (a different OAuth
    // identity authenticated against the same DB) asks to verify A's tx.
    // The storage layer's `find_write_mode_by_tx` scope filter must
    // return `Ok(None)` — `verify` must surface this as the
    // `not_found` shape with no identifying fields.
    let server = TestServer::builder().storage_mode("local").build();
    let token_b = server.mint_test_jwt(USER_B_PUBKEY);

    // Seed A's local row directly (signer == owner == A), mirroring the
    // participate variant (3b). Wave 3 removed operator-inline-signing for
    // remote users, so A's `mode: "local"` write now (correctly) routes to
    // the client-signing deferred path — which `USER_A_PUBKEY` (a fake,
    // keyless constant) cannot complete. This test only needs a local-tagged
    // row owned by A to exercise the tenant-scoped routing isolation, not the
    // write path itself; `seed_row` provides exactly that.
    let (a_sol_tx, _ar) = seed_row(&server, USER_A_PUBKEY, WriteMode::Local, "tenant-iso-l");

    // Sanity: B's `mnemonic_recall` does not surface A's row (already
    // covered by recall_owner_isolation, but cheap to assert here for
    // diagnostic clarity on a future regression).
    let recall_b = server
        .with_token(&token_b)
        .call_tool("mnemonic_recall", json!({"query": "secret", "limit": 10}))
        .await;
    let recall_inner = recall_b.result_text();
    let rows = recall_inner["results"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        rows.is_empty(),
        "B's recall must not surface A's row: {rows:?}"
    );

    // Caller B verifies A's tx.
    let resp_b = server
        .with_token(&token_b)
        .call_tool("mnemonic_verify", json!({"solana_tx": a_sol_tx}))
        .await;
    assert!(
        resp_b.error().is_none(),
        "B's verify produced an error envelope: {:?}",
        resp_b.envelope
    );
    let result = resp_b.result_text();
    assert_eq!(
        result["status"], "not_found",
        "expected not_found, got {result:?}"
    );
    // No leaky field — neither the row's identifying hash, the signer,
    // nor any content preview must appear in the response B sees.
    for leaky in LEAKY_FIELDS {
        assert!(
            result.get(leaky).is_none(),
            "leaked field {leaky:?} in tenant-isolation response: {result:?}"
        );
    }
}

// ── 3b. Tenant isolation: participate row ──────────────────────────────────

#[tokio::test]
async fn tenant_isolation_participate() {
    // Same shape as `tenant_isolation_local`, but A's row is tagged
    // `participate`. The routing query (`find_write_mode_by_tx`) is
    // tenant-scoped — B must see the `not_found` shape and zero
    // identifying fields.
    let server = TestServer::builder().storage_mode("local").build();
    let token_b = server.mint_test_jwt(USER_B_PUBKEY);

    // Seed A's participate row directly. The MCP sign path with
    // `mode: "participate"` under STORAGE_MODE=local would (correctly)
    // reject as UnsupportedMode; this test only needs a participate-
    // tagged row to exercise the routing isolation, not the participate
    // happy path.
    let (a_sol_tx, _ar) = seed_row(
        &server,
        USER_A_PUBKEY,
        WriteMode::Participate,
        "tenant-iso-p",
    );

    let resp_b = server
        .with_token(&token_b)
        .call_tool("mnemonic_verify", json!({"solana_tx": a_sol_tx}))
        .await;
    assert!(
        resp_b.error().is_none(),
        "B's verify produced error envelope: {:?}",
        resp_b.envelope
    );
    let result = resp_b.result_text();
    assert_eq!(
        result["status"], "not_found",
        "expected not_found for cross-tenant participate row, got {result:?}"
    );
    for leaky in LEAKY_FIELDS {
        assert!(
            result.get(leaky).is_none(),
            "leaked field {leaky:?} in participate tenant-isolation response: {result:?}"
        );
    }
}

// ── 4. Recall surfaces stored write_mode ───────────────────────────────────

#[tokio::test]
async fn recall_surfaces_write_mode() {
    // Seed one local + one participate row under the same owner; recall
    // must surface each row's stored `write_mode` so a mixed-mode client
    // can render provenance.
    let server = TestServer::builder().storage_mode("local").build();
    let owner = server.server_pubkey();
    seed_row(&server, &owner, WriteMode::Local, "recall-local");
    seed_row(
        &server,
        &owner,
        WriteMode::Participate,
        "recall-participate",
    );

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_recall",
            json!({"query": "seeded", "limit": 10}),
        )
        .await;
    assert!(
        result.error().is_none(),
        "recall envelope: {:?}",
        result.envelope
    );
    let inner = result.result_text();
    let results = inner["results"].as_array().cloned().unwrap_or_default();
    assert_eq!(results.len(), 2, "expected both rows in recall: {inner:?}");
    // Bucket results by their solana_tx prefix to map them back to the
    // expected mode without relying on result ordering.
    let mut modes: std::collections::HashMap<String, String> = Default::default();
    for row in &results {
        let sol = row["solana_tx"].as_str().expect("solana_tx").to_string();
        let mode = row["write_mode"]
            .as_str()
            .expect("write_mode must be surfaced in recall result envelope")
            .to_string();
        modes.insert(sol, mode);
    }
    let local_seed_sol = "local:recall-local-sol".to_string();
    let participate_seed_sol =
        "solxrecall-participateaRRRRYWfLh8x2y2y9X7Cxe1aN3aN3aN3aN3".to_string();
    assert_eq!(
        modes.get(&local_seed_sol).map(|s| s.as_str()),
        Some("local"),
        "local row must carry write_mode=local; modes={modes:?}"
    );
    assert_eq!(
        modes.get(&participate_seed_sol).map(|s| s.as_str()),
        Some("participate"),
        "participate row must carry write_mode=participate; modes={modes:?}"
    );
}

// ── 5. verify_local checks the real artifact hash, not a raw-content digest ──

/// Regression pin for the `verify_local` hash-domain bug.
///
/// `sign_memory` stores `blake3(canonical_cbor(artifact))`, but `verify_local`
/// used to recompute `blake3(content)` — a different preimage — and compare the
/// two for equality. The comparison could never succeed, so every CBOR-era row
/// verified as `tampered`. The bug survived from April because the only local
/// verify fixture stored `blake3(content)` too, matching the broken check.
///
/// This test drives the real `sign_memory` path and asserts the round trip, so
/// any future divergence between the two constructions fails here.
#[tokio::test]
async fn local_sign_then_verify_round_trips() {
    let server = TestServer::builder().storage_mode("local").build();
    let owner = server.server_pubkey();

    let signed = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({"content": "round trip through the real sign path", "mode": "local"}),
        )
        .await;
    assert!(
        signed.error().is_none(),
        "sign envelope: {:?}",
        signed.envelope
    );
    let signed = signed.result_text();
    let sol = signed["solana_tx"].as_str().expect("solana_tx").to_string();

    let result = server
        .call_tool(Some(&owner), "mnemonic_verify", json!({"solana_tx": sol}))
        .await;
    assert!(
        result.error().is_none(),
        "verify envelope: {:?}",
        result.envelope
    );
    let inner = result.result_text();

    assert_eq!(
        inner["status"], "verified",
        "a row written by sign_memory must verify; got {inner:?}"
    );
    assert_eq!(
        inner["content_hash"].as_str(),
        signed["content_hash"].as_str(),
        "verify must echo the hash sign_memory stored"
    );
    assert_eq!(
        inner["checks"]["artifact_reconstructed"], true,
        "the CBOR artifact must be rebuilt, not fall through to a legacy digest"
    );
}

/// The flip side: a row whose `content` column was edited behind the store's
/// back must still be caught. Without this, "always verified" would pass the
/// test above just as happily as a correct implementation.
#[tokio::test]
async fn local_verify_detects_edited_content() {
    let server = TestServer::builder().storage_mode("local").build();
    let owner = server.server_pubkey();

    let signed = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({"content": "original content", "mode": "local"}),
        )
        .await;
    let signed = signed.result_text();
    let sol = signed["solana_tx"].as_str().expect("solana_tx").to_string();

    // Edit the content column directly, leaving content_hash untouched —
    // exactly the tamper this check exists to catch.
    {
        let store = server.state.store.lock().unwrap();
        store
            .conn()
            .execute(
                "UPDATE attestations SET content = ?1 WHERE solana_tx = ?2",
                rusqlite::params!["tampered content", &sol],
            )
            .expect("tamper");
    }

    let result = server
        .call_tool(Some(&owner), "mnemonic_verify", json!({"solana_tx": sol}))
        .await;
    let inner = result.result_text();
    assert_eq!(
        inner["status"], "tampered",
        "an edited content column must be detected; got {inner:?}"
    );
}

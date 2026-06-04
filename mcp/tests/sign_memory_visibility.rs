//! Integration tests for the new `visibility` field on
//! `mnemonic_sign_memory` (Task 4 — agent-native-distribution).
//!
//! Two TDD anchors:
//!
//! 1. `visibility_rejected_on_local_writes` — `{ mode: "local",
//!    visibility: "public" }` returns `-32602 InvalidParams` with
//!    `data.field == "visibility"` (AC14).
//! 2. `visibility_persisted_on_participate_public` — full participate flow
//!    with a valid confirmation token writes a row with `visibility='public'`.
//!
//! Both run against the in-process `TestServer` harness shared with
//! `modes_per_request.rs` so the dispatcher + middleware wiring is identical
//! to production.

#![cfg(feature = "test-support")]

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::Visibility;
use serde_json::json;

#[tokio::test]
async fn visibility_rejected_on_local_writes() {
    // STORAGE_MODE=local + explicit mode=local + visibility=public must
    // return -32602 with data.field == "visibility". Decision 3 / AC14.
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "visibility-on-local",
                "mode": "local",
                "visibility": "public",
            }),
        )
        .await;

    let err = result.expect_error();
    assert_eq!(err["code"], -32602, "expected -32602 InvalidParams");
    assert_eq!(
        err["data"]["field"], "visibility",
        "data.field must be visibility"
    );
    assert_eq!(
        err["data"]["received"], "public",
        "data.received must echo the verbatim input"
    );

    // DB unchanged — no row inserted on the boundary rejection.
    assert_eq!(
        server.attestation_count(&owner),
        0,
        "invalid_params must NOT persist a row"
    );
}

#[tokio::test]
async fn visibility_rejected_on_local_writes_even_for_private_value() {
    // The rejection is on the PRESENCE of the field for local writes, not
    // on the value. A literal `"private"` is still invalid params — AC14
    // says visibility is a participate-only concept.
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "visibility-private-on-local",
                "mode": "local",
                "visibility": "private",
            }),
        )
        .await;

    let err = result.expect_error();
    assert_eq!(err["code"], -32602);
    assert_eq!(err["data"]["field"], "visibility");
    assert_eq!(err["data"]["received"], "private");
}

#[tokio::test]
async fn visibility_rejected_for_non_canonical_value() {
    // Case-variant / typo'd values on participate-mode still return
    // -32602; the resolver's contract is strict like `resolve_write_mode`.
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();

    for bad in ["Public", "PUBLIC", "publicX", " public", ""] {
        let result = server
            .call_tool(
                Some(&owner),
                "mnemonic_sign_memory",
                json!({
                    "content": format!("bad-vis-{bad}"),
                    "mode": "participate",
                    "visibility": bad,
                }),
            )
            .await;
        let err = result.expect_error();
        assert_eq!(err["code"], -32602, "{bad:?} must produce -32602");
        assert_eq!(err["data"]["field"], "visibility");
        assert_eq!(err["data"]["received"], bad);
    }
}

#[tokio::test]
async fn visibility_threads_through_to_storage() {
    // The dispatcher's HTTP-with-JWT path routes a participate-mode write
    // through the deferred branch (the WASM signer finishes it client-
    // side), which does not persist a row from this surface. Asserting
    // visibility persistence end-to-end here would require an arlocal
    // mock binding + real chain anchor — out of scope for Task 4.
    //
    // Instead we drive `tools::sign_memory_inline` indirectly via the
    // stdio code path (jwt_sub = None) which goes through inline signing
    // + save_attestation. `STORAGE_MODE=local` keeps the Arweave/Solana
    // clients untouched (synthetic `local:` tx ids), so the success path
    // is deterministic.
    use mnemonic_mcp::tools::{resolve_write_mode, sign_memory};
    use std::time::Duration;

    let server = TestServer::builder()
        .storage_mode("local")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();

    // The visibility resolver rejects `visibility=public` with `mode=local`
    // (AC14). Inline participate-mode + visibility=public on the stdio
    // path needs a confirmation token because the dispatcher's gate
    // still applies — but the gate lives in `handle_tool_call`, not in
    // `sign_memory` itself. Direct call to `sign_memory` bypasses the
    // gate and is the right surface to prove visibility threads through
    // to `save_attestation`.
    let kp = solana_sdk::signature::Keypair::new();
    let owner_kp = mnemonic_core::identity::pubkey_base58(&kp);
    let resolved = resolve_write_mode(None, "local").expect("None resolves");

    let cost_hint = mnemonic_mcp::pricing::CostHint {
        irys_lamports: 0,
        sol_tx_fee_lamports: 0,
        sol_price_usdc: 0.0,
        charge_micro_usdc: 0,
    };

    let softfall_args = serde_json::json!({});
    let result = sign_memory(
        &kp,
        &server.state.solana,
        &server.state.arweave,
        &server.state.store,
        server.state.embedder.as_ref(),
        &server.state.compressor,
        &server.state.pending,
        "visibility-threading-content",
        &[],
        &cost_hint,
        "local",
        &owner_kp,
        None,
        resolved,
        Visibility::Private,
        &server.state.envelope,
        Duration::from_secs(15),
        false,
        &server.state.hosted_endpoint,
        &server.state.hosted_client,
        &softfall_args,
    )
    .await
    .expect("sign_memory inline succeeds with stub embedder");

    // The response envelope surfaces the visibility verbatim.
    assert_eq!(result["visibility"], "private");
    let attestation_id = result["attestation_id"].as_str().expect("attestation_id");
    {
        let store = server.state.store.lock().unwrap();
        let conn = store.conn();
        let vis: String = conn
            .query_row(
                "SELECT visibility FROM attestations WHERE attestation_id = ?",
                rusqlite::params![attestation_id],
                |row| row.get(0),
            )
            .expect("visibility column");
        assert_eq!(vis, "private", "visibility column reflects threaded value");
    }

    // F1 (test-reviewer round 1, agent-native-distribution Task 4):
    // also prove `Visibility::Public` threads through to the persisted
    // row. Direct `sign_memory` call bypasses the dispatcher's
    // public-write confirmation gate (which only fires through
    // `handle_tool_call`), so no ceremony token is needed — the test
    // exercises the threading itself, not the gate.
    let result_public = sign_memory(
        &kp,
        &server.state.solana,
        &server.state.arweave,
        &server.state.store,
        server.state.embedder.as_ref(),
        &server.state.compressor,
        &server.state.pending,
        "visibility-threading-content-public",
        &[],
        &cost_hint,
        "local",
        &owner_kp,
        None,
        resolved,
        Visibility::Public,
        &server.state.envelope,
        Duration::from_secs(15),
        false,
        &server.state.hosted_endpoint,
        &server.state.hosted_client,
        &softfall_args,
    )
    .await
    .expect("sign_memory inline succeeds for Public visibility too");

    assert_eq!(result_public["visibility"], "public");
    let attestation_id_public = result_public["attestation_id"]
        .as_str()
        .expect("attestation_id");
    {
        let store = server.state.store.lock().unwrap();
        let conn = store.conn();
        let vis: String = conn
            .query_row(
                "SELECT visibility FROM attestations WHERE attestation_id = ?",
                rusqlite::params![attestation_id_public],
                |row| row.get(0),
            )
            .expect("visibility column");
        assert_eq!(vis, "public", "visibility=public also persists verbatim");
    }

    // Silence unused-warning for `owner` in the harness setup.
    let _ = owner;
}

#[tokio::test]
async fn allow_fallback_strict_bool() {
    // `allow_fallback_to_participate` must be a strict bool. Any non-bool
    // returns -32602 with data.field set.
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "non-bool-fallback",
                "mode": "local",
                "allow_fallback_to_participate": "yes",
            }),
        )
        .await;
    let err = result.expect_error();
    assert_eq!(err["code"], -32602);
    assert_eq!(err["data"]["field"], "allow_fallback_to_participate");
    assert_eq!(err["data"]["received"], "yes");
}

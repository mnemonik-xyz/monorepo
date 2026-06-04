//! Integration tests for the Decision 5b public-write confirmation gate
//! (Task 4 — agent-native-distribution).
//!
//! Anchors:
//!
//! - `mint_without_jwt_returns_unauthorized`
//! - `mint_with_jwt_returns_token`
//! - `consume_succeeds_for_matching_args`
//! - `consume_replay_rejected`
//! - `consume_with_different_content_hash_rejected`
//! - `consume_with_different_owner_rejected` (the cross-owner replay test
//!   from Decision 5b)
//! - `consume_with_no_token_field_rejected`

#![cfg(feature = "test-support")]

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::Visibility;
use serde_json::{json, Value};

/// Round 2 / SAR1-L2: mint() now requires `content_hash` to be exactly
/// 64 ASCII-hex characters. Helper picks a deterministic fixture hash.
fn fixture_hash() -> String {
    blake3::hash(b"fixture-content").to_hex().to_string()
}

#[tokio::test]
async fn mint_without_jwt_returns_unauthorized() {
    // request_public_write_confirmation is NOT in ALLOWLIST_METHODS — the
    // bearer-auth middleware rejects an anonymous call with -32001 BEFORE
    // any content_hash validation runs.
    let server = TestServer::builder().build();
    let result = server
        .call_tool(
            None,
            "request_public_write_confirmation",
            json!({ "content_hash": fixture_hash() }),
        )
        .await;
    assert_eq!(
        result.status,
        axum::http::StatusCode::UNAUTHORIZED,
        "anonymous mint must be 401"
    );
    let err = result.expect_error();
    assert_eq!(err["code"], -32001);
}

#[tokio::test]
async fn mint_with_jwt_returns_token() {
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    let result = server
        .call_tool(
            Some(&owner),
            "request_public_write_confirmation",
            json!({ "content_hash": fixture_hash() }),
        )
        .await;
    assert_eq!(result.status, axum::http::StatusCode::OK);
    let inner = result.result_text();
    assert!(inner["confirmation_token"].is_string());
    assert!(inner["jti"].is_string());
    assert!(inner["expires_at"].is_number());
}

#[tokio::test]
async fn mint_with_malformed_content_hash_rejected() {
    // SAR1-L2 (round 1 security audit): content_hash must be 64
    // ASCII-hex characters. A short or non-hex value returns
    // -32602 InvalidParams BEFORE the ledger is touched, so a malicious
    // authenticated caller cannot inflate the in-process DashMap with
    // garbage entries.
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    for bad in [
        "abcd",
        "not-hex-input-here-just-letters-and-such",
        "",
        &"f".repeat(63),
        &"g".repeat(64),
    ] {
        let result = server
            .call_tool(
                Some(&owner),
                "request_public_write_confirmation",
                json!({ "content_hash": bad }),
            )
            .await;
        let err = result.expect_error();
        assert_eq!(err["code"], -32602, "{bad:?} must return -32602");
        assert_eq!(err["data"]["field"], "content_hash");
    }
    assert_eq!(
        server.state.confirmation_ledger.len(),
        0,
        "malformed mint must NOT add an entry to the ledger"
    );
}

#[tokio::test]
async fn consume_succeeds_for_matching_args() {
    // Mint a token via the ledger directly (faster than going through the
    // tool surface twice — the tool surface is exercised by the previous
    // anchor) and verify sign_memory consumes it cleanly. The Arweave
    // write at http://localhost:0 will fail downstream, but the consume
    // step must succeed — verified by asserting the error is NOT -32095.
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let content = "consume-matching";
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    let (token, jti, _) =
        server
            .state
            .confirmation_ledger
            .mint(&content_hash, &owner, Visibility::Public);

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": content,
                "mode": "participate",
                "visibility": "public",
                "public_write_confirmation": token,
                "jti": jti.to_string(),
            }),
        )
        .await;

    let err = result.error();
    if let Some(e) = err {
        let code = e["code"].as_i64().unwrap_or(0);
        assert_ne!(
            code, -32095,
            "valid confirmation token must NOT be rejected as -32095: {e}"
        );
    }
    // Ledger must have removed the entry post-consume (single-use).
    //
    // ORDERING INTENT (code-reviewer round 1 CR-5): the gate consumes the
    // token BEFORE the Arweave/Solana downstream IO. So even when the
    // downstream write fails (as it does here against http://localhost:0),
    // the ledger row is gone. If this assertion ever flips to `> 0`, the
    // public-write gate has regressed — somebody reordered the consume
    // after the IO, opening a window where a failed write leaves an
    // unconsumed token in the ledger.
    assert_eq!(server.state.confirmation_ledger.len(), 0);
}

#[tokio::test]
async fn consume_replay_rejected() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let content = "replay-content";
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    let (token, jti, _) =
        server
            .state
            .confirmation_ledger
            .mint(&content_hash, &owner, Visibility::Public);

    // First consume — bypass the Arweave failure path by accepting any
    // outcome; the consume call removed the entry on success regardless.
    let _ = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": content,
                "mode": "participate",
                "visibility": "public",
                "public_write_confirmation": token.clone(),
                "jti": jti.to_string(),
            }),
        )
        .await;

    // Second consume — same token, second sign_memory call. -32095.
    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": content,
                "mode": "participate",
                "visibility": "public",
                "public_write_confirmation": token,
                "jti": jti.to_string(),
            }),
        )
        .await;
    let err = result.expect_error();
    assert_eq!(err["code"], -32095, "replay must return -32095: {err}");
    assert_eq!(err["data"]["kind"], "PublicWriteRequiresConfirmation");
    assert_eq!(err["data"]["content_hash"], content_hash);
}

#[tokio::test]
async fn consume_with_different_content_hash_rejected() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let bound_hash = blake3::hash(b"H1").to_hex().to_string();
    let (token, jti, _) =
        server
            .state
            .confirmation_ledger
            .mint(&bound_hash, &owner, Visibility::Public);

    // sign_memory computes content_hash = blake3(content). Send content
    // whose hash differs from `bound_hash` — the consume reconstruction
    // mismatches.
    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "H2",
                "mode": "participate",
                "visibility": "public",
                "public_write_confirmation": token,
                "jti": jti.to_string(),
            }),
        )
        .await;
    let err = result.expect_error();
    assert_eq!(err["code"], -32095);
}

#[tokio::test]
async fn consume_with_different_owner_rejected() {
    // The cross-owner replay test from Decision 5b. Owner A mints; owner
    // B (different JWT) presents A's token. HMAC reconstruction at consume
    // uses B's owner from `claims.sub` — mismatch → -32095.
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner_a = "owner-A-base58";
    let owner_b = "owner-B-base58";
    let content = "cross-owner-content";
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    let (token, jti, _) =
        server
            .state
            .confirmation_ledger
            .mint(&content_hash, owner_a, Visibility::Public);

    // Owner B's JWT — the dispatcher's `claims.sub` becomes owner_b, the
    // confirmation gate's `owner_pubkey` for the consume reconstruction
    // is therefore owner_b. HMAC mismatches the stored A-bound entry.
    let result = server
        .call_tool(
            Some(owner_b),
            "mnemonic_sign_memory",
            json!({
                "content": content,
                "mode": "participate",
                "visibility": "public",
                "public_write_confirmation": token,
                "jti": jti.to_string(),
            }),
        )
        .await;
    let err = result.expect_error();
    assert_eq!(
        err["code"], -32095,
        "cross-owner replay must return -32095: {err}"
    );
}

#[tokio::test]
async fn consume_with_no_token_field_rejected() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "no-token",
                "mode": "participate",
                "visibility": "public",
            }),
        )
        .await;
    let err = result.expect_error();
    assert_eq!(err["code"], -32095);
    assert_eq!(err["data"]["kind"], "PublicWriteRequiresConfirmation");
}

#[tokio::test]
async fn consume_with_malformed_jti_rejected() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let content = "malformed-jti";
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    let (token, _real_jti, _) =
        server
            .state
            .confirmation_ledger
            .mint(&content_hash, &owner, Visibility::Public);

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": content,
                "mode": "participate",
                "visibility": "public",
                "public_write_confirmation": token,
                "jti": "not-a-valid-uuid",
            }),
        )
        .await;
    let err = result.expect_error();
    assert_eq!(err["code"], -32095);
}

#[tokio::test]
async fn participate_private_without_token_succeeds_path() {
    // The gate fires ONLY for participate + public. participate + private
    // (the default) must not hit the gate.
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "default-private-participate",
                "mode": "participate",
            }),
        )
        .await;
    // Regardless of downstream success (Arweave at http://localhost:0
    // may fail), the gate must not have rejected us with -32095.
    if let Some(err) = result.error() {
        assert_ne!(
            err["code"].as_i64().unwrap_or(0),
            -32095,
            "default-private must bypass the gate: {err}"
        );
    }
}

// Helper to keep clippy happy about unused imports across the parametric
// shapes above.
#[allow(dead_code)]
fn _ensure_value_import_is_used(v: Value) -> Value {
    v
}

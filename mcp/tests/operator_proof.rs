//! Integration tests for `mnemonic_operator_proof`.
//!
//! Clients verify a pinned operator key before they send a JWT, so the tool
//! must be reachable without `Authorization`. It must not become a signing
//! oracle: the server composes the signed message, and the caller supplies
//! only a fixed-length hex nonce.

#![cfg(feature = "test-support")]

mod _helpers;

use std::str::FromStr;

use _helpers::TestServer;
use axum::http::StatusCode;
use mnemonic_core::identity;
use serde_json::json;
use solana_sdk::pubkey::Pubkey;

const NONCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn anonymous_proof_signs_server_composed_origin_bound_message() {
    let server = TestServer::builder().build();
    let result = server
        .call_tool(None, "mnemonic_operator_proof", json!({ "nonce": NONCE }))
        .await;
    assert_eq!(result.status, StatusCode::OK, "{:?}", result.envelope);

    let proof = result.result_text();
    let origin = mnemonic_mcp::oauth::server_origin();
    assert_eq!(proof["public_key"], server.server_pubkey());
    assert_eq!(proof["origin"], origin);
    assert_eq!(proof["nonce"], NONCE);

    let message = format!("mnemonic.operator-selection.v1\n{origin}\n{NONCE}");
    let signature = hex::decode(proof["signature"].as_str().expect("signature")).expect("hex");
    let pubkey = Pubkey::from_str(&server.server_pubkey()).expect("pubkey");
    assert!(identity::verify_signature(
        &pubkey,
        message.as_bytes(),
        &signature
    ));
    // The raw nonce alone is never what gets signed.
    assert!(!identity::verify_signature(
        &pubkey,
        NONCE.as_bytes(),
        &signature
    ));
}

#[tokio::test]
async fn proof_rejects_caller_chosen_messages() {
    let server = TestServer::builder().build();
    let upper = NONCE.to_uppercase();
    let long = format!("{NONCE}00");
    for nonce in [
        "",
        "short",
        upper.as_str(),
        long.as_str(),
        "mnemonic.operator-selection.v1\nx",
    ] {
        let result = server
            .call_tool(None, "mnemonic_operator_proof", json!({ "nonce": nonce }))
            .await;
        assert!(
            result.envelope["error"].is_object(),
            "nonce {nonce:?} must be rejected: {:?}",
            result.envelope
        );
    }
    let missing = server
        .call_tool(None, "mnemonic_operator_proof", json!({}))
        .await;
    assert!(
        missing.envelope["error"].is_object(),
        "{:?}",
        missing.envelope
    );
}

#[tokio::test]
async fn arbitrary_challenge_signing_still_requires_a_token() {
    let server = TestServer::builder().build();
    let result = server
        .call_tool(
            None,
            "mnemonic_prove_identity",
            json!({ "challenge": "anything" }),
        )
        .await;
    assert_eq!(
        result.status,
        StatusCode::UNAUTHORIZED,
        "{:?}",
        result.envelope
    );
}

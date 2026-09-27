//! Integration tests for the delivery guarantee under the non-custodial
//! (Wave 4) payment model. Custodial `balance`-mode tests were removed when
//! Wave 4 deleted the api-key ledger.
//!
//! The harness mounts `mcp_handler` WITHOUT the OAuth middleware, so an HTTP
//! call here carries no JWT and `mcp_handler` falls back to owner = operator.
//! Over HTTP the operator key never signs a memory, so inline participate
//! delivery exists only on the single-tenant stdio transport.
//!
//! Scenarios:
//!
//! 1. `happy_path` — `#[ignore]` sentinel; the real success path needs an
//!    arlocal + solana-test-validator harness. The failure tests below cover
//!    the same code paths in their failure direction.
//! 2. `stdio_participate_demotes_on_refetch_failure` — induced Arweave
//!    refetch failure on the stdio inline path (the agent signs its own
//!    memory). The row is demoted to `local`, the typed `-32011` error names
//!    the stage, and no cost row is written.
//! 3. `x402_participate_without_jwt_is_refused_before_chain_write` — an
//!    HTTP participate call without a JWT (valid x402 proof) is refused
//!    before any chain write: no operator signature, no row, and the x402
//!    nonce stays reusable.
//! 4. `quota_exceeded_x402_short_circuits_before_chain_write` — a payment
//!    subject (blake3(tx_sig)) at the quota threshold short-circuits with
//!    `DeliveryQuotaExceeded` BEFORE any chain call.

#[path = "_helpers/delivery_harness.rs"]
mod delivery_harness;

use delivery_harness::{
    build_state_and_router_x402, call_sign_memory_participate_x402, MockArweave, MockSolana,
};

use std::time::Duration;

use axum::http::StatusCode;

const CHEAP_COST: i64 = 1_000; // 0.001 USDC per write

// A real-looking Solana tx signature (base58, ~88 chars). Doesn't have to
// verify on-chain — the mock dispatcher just substring-matches on it.
const TX_SIG: &str =
    "5VfYdM3GjRZqkBdYNz2hVnQYsBfP1k8fL3jHkMb7vYqXrJzGw2XaRpUyMcNvDsW4eLkR1tFqGxKyPmAhU6Dv8nQT";
// Synthetic operator treasury + mainnet USDC mint. The mock proof says this
// exact owner+mint received `CHEAP_COST` micro-USDC.
const TREASURY: &str = "TreaSurYMockPayToMnemonik11111111111111111";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

// ── 1. Happy path — see #[ignore] note above ────────────────────────────────

#[ignore = "requires real arlocal + solana-test-validator harness; per-stage failure tests below exercise the same code paths in their non-failure direction"]
#[tokio::test]
async fn happy_path() {
    // Sentinel — keeps the slot in the file so future infra wiring can drop
    // the `#[ignore]` without restructuring.
}

// ── 2. Stdio inline participate: demotion on refetch failure ────────────────

/// On stdio the operator key is the local agent's own identity, so inline
/// participate is legitimate. Anchor PUT succeeds; GET returns 404 → the
/// refetch budget exhausts and the delivery check exits at `refetch`. The
/// row must be demoted to `local` and no cost row written.
#[tokio::test]
async fn stdio_participate_demotes_on_refetch_failure() {
    let arweave = MockArweave::read_fails("AR_TX_STDIO");
    arweave.install();
    let solana =
        MockSolana::happy_with_x402_payment(TX_SIG, TREASURY, USDC_MINT, CHEAP_COST as u64);
    let (state, _app) = build_state_and_router_x402(
        &arweave.base_url(),
        &solana.base_url(),
        CHEAP_COST,
        5,
        Duration::from_secs(60),
        Duration::from_secs(2),
        TREASURY,
        USDC_MINT,
    );

    let req: mnemonic_mcp::mcp::JsonRpcRequest = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "mnemonic_sign_memory",
            "arguments": {"content": "hello-stdio", "mode": "participate"},
        },
    }))
    .expect("request");
    let owner = state.keypair.pubkey_base58();
    let resp = mnemonic_mcp::mcp::handle_request(
        &req,
        &state,
        &owner,
        None,
        mnemonic_mcp::tools::Transport::Stdio,
    )
    .await;
    let envelope = serde_json::to_value(&resp).expect("response serializes");

    let err = envelope["error"]
        .as_object()
        .expect("expected JSON-RPC error envelope");
    assert_eq!(err["code"], -32011, "{envelope}");
    let data = err["data"].as_object().expect("data");
    assert_eq!(data["stage"], "refetch");
    assert_eq!(data["row_demoted_to"], "local");
    let attestation_id = data["attestation_id"]
        .as_str()
        .expect("attestation_id in -32011 error data")
        .to_string();

    let store = state.store.lock().unwrap();
    let (write_mode, signer): (String, String) = store
        .conn()
        .query_row(
            "SELECT write_mode, signer_pubkey FROM attestations WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row exists after demotion");
    assert_eq!(write_mode, "local");
    // The agent signed its own memory: signer == owner == operator key.
    assert_eq!(signer, owner);
    let costs_count: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM attestation_costs WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(costs_count, 0, "demoted writes must not record a cost row");
}

// ── 3. HTTP participate without a JWT: refused before any chain write ───────

/// Before the transport guard this call reached inline operator signing
/// (Arweave upload + Solana memo under the operator key). Now `sign_memory`
/// refuses it: over HTTP the operator key never signs a memory. The refusal
/// leaves no row and no Arweave upload, and it does NOT consume the x402
/// nonce (`consume_x402_nonce_after_success` runs only on success), so the
/// payment proof stays reusable for a client-signed retry.
#[tokio::test]
async fn x402_participate_without_jwt_is_refused_before_chain_write() {
    let arweave = MockArweave::read_fails("AR_TX_X402");
    let post_tx_mock = arweave.install();
    let solana =
        MockSolana::happy_with_x402_payment(TX_SIG, TREASURY, USDC_MINT, CHEAP_COST as u64);
    let (state, app) = build_state_and_router_x402(
        &arweave.base_url(),
        &solana.base_url(),
        CHEAP_COST,
        5,
        Duration::from_secs(60),
        Duration::from_secs(2),
        TREASURY,
        USDC_MINT,
    );

    let (status, envelope) = call_sign_memory_participate_x402(&app, TX_SIG, "hello-x402").await;
    assert_eq!(status, StatusCode::OK);
    let err = envelope["error"]
        .as_object()
        .expect("expected JSON-RPC error envelope");
    assert_eq!(err["code"], -32603, "{envelope}");
    assert!(
        err["message"]
            .as_str()
            .unwrap_or_default()
            .contains("client-signed"),
        "{envelope}"
    );
    assert_eq!(post_tx_mock.calls(), 0, "no Arweave upload may happen");

    let store = state.store.lock().unwrap();
    let rows: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM attestations", [], |r| r.get(0))
        .expect("count attestations");
    assert_eq!(rows, 0, "a refused write must not persist a row");
    let nonce_consumed: bool = store
        .conn()
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM x402_nonces WHERE tx_sig = ?)",
            rusqlite::params![TX_SIG],
            |r| r.get::<_, bool>(0),
        )
        .expect("x402_nonces query");
    assert!(
        !nonce_consumed,
        "a refused write must leave the x402 nonce reusable"
    );
    let costs: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM attestation_costs", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(costs, 0, "a refused write must not record a cost row");
}

// ── 4. Quota guard (x402) short-circuits before chain write ─────────────────

/// The outcome-based DoS guard, keyed on `blake3(x402 tx_sig)`. It runs in
/// `mcp_handler` before the payment check and before dispatch, so an
/// over-quota subject never reaches a chain write. HTTP can no longer
/// produce inline demotions (the operator key never signs there), so the
/// test seeds the counter with `threshold` failures directly.
#[tokio::test]
async fn quota_exceeded_x402_short_circuits_before_chain_write() {
    let arweave = MockArweave::read_fails("AR_TX_QUOTA_X402");
    let post_tx_mock = arweave.install();
    let solana =
        MockSolana::happy_with_x402_payment(TX_SIG, TREASURY, USDC_MINT, CHEAP_COST as u64);
    let (state, app) = build_state_and_router_x402(
        &arweave.base_url(),
        &solana.base_url(),
        CHEAP_COST,
        /* threshold */ 3,
        /* window */ Duration::from_secs(60),
        Duration::from_millis(300),
        TREASURY,
        USDC_MINT,
    );

    let subject = mnemonic_mcp::payment::hash_api_key(TX_SIG);
    for _ in 0..3 {
        state.refunds_by_subject.record_failure(&subject);
    }
    assert!(state.refunds_by_subject.is_over(&subject));

    let (status, env) = call_sign_memory_participate_x402(&app, TX_SIG, "quota-bump-final").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let err = env["error"].as_object().expect("error");
    assert_eq!(err["code"], -32011);
    assert_eq!(err["data"]["kind"], "DeliveryQuotaExceeded");

    // The short-circuit must not spend any Arweave fee.
    assert_eq!(
        post_tx_mock.calls(),
        0,
        "quota short-circuit must not spend Arweave fees"
    );
    assert_eq!(state.delivery_metrics.quota_short_circuit(), 1);
}

//! Integration tests for the delivery guarantee under the non-custodial
//! (Wave 4) payment model. Custodial `balance`-mode tests were removed when
//! Wave 4 deleted the api-key ledger; the delivery-confirmation contract is
//! now exercised entirely over the x402 rail (the only paid path).
//!
//! Scenarios:
//!
//! 1. `happy_path` — `#[ignore]` sentinel; the real success path needs an
//!    arlocal + solana-test-validator harness. The failure tests below cover
//!    the same code paths in their failure direction.
//! 2. `demotion_on_x402_leaves_nonce_reusable` — induced Arweave refetch
//!    failure under PAYMENT_MODE=x402. Asserts the nonce deferral
//!    (`mark_x402_nonce` fires only after delivery success): after a failed
//!    delivery the `x402_nonces` table is empty, so the caller can retry with
//!    the same `X-Payment` header without an "already consumed" rejection.
//! 3. `quota_exceeded_x402_short_circuits_before_chain_write` — N consecutive
//!    demotions for the same payment subject (blake3(tx_sig)); the next
//!    `participate` call short-circuits with `DeliveryQuotaExceeded` BEFORE
//!    any chain call (the outcome-based DoS guard, now keyed on the x402
//!    tx_sig).

#[path = "_helpers/delivery_harness.rs"]
mod delivery_harness;

use delivery_harness::{
    build_state_and_router_x402, call_sign_memory_participate_x402, MockArweave, MockSolana,
};

use std::time::Duration;

use axum::http::StatusCode;

const CHEAP_COST: i64 = 1_000; // 0.001 USDC per write

// ── 1. Happy path — see #[ignore] note above ────────────────────────────────

#[ignore = "requires real arlocal + solana-test-validator harness; per-stage failure tests below exercise the same code paths in their non-failure direction"]
#[tokio::test]
async fn happy_path() {
    // Sentinel — keeps the slot in the file so future infra wiring can drop
    // the `#[ignore]` without restructuring.
}

// ── 7. T3.5 — x402 nonce reusable after demotion ────────────────────────────

/// `demotion_on_x402_leaves_nonce_reusable` — under PAYMENT_MODE=x402, an
/// induced delivery failure must NOT consume the x402 nonce. This pins the
/// T3-round-2 deferral (`consume_x402_nonce_after_success` runs only on a
/// confirmed delivery) end-to-end: an attacker / unlucky caller can retry
/// with the same `X-Payment` header without seeing a misleading "already
/// consumed" rejection, and the operator hasn't double-billed.
///
/// Mocks `getTransaction` on Solana so `verify_usdc_transfer` passes
/// without us minting an actual on-chain transaction. The delivery still
/// fails for the regular T3 reason (corrupted Arweave GET), so the
/// post-anchor demotion + refund path is what's actually exercised here.
#[tokio::test]
async fn demotion_on_x402_leaves_nonce_reusable() {
    // A real-looking Solana tx signature (base58, ~88 chars). Doesn't have to
    // verify on-chain — the mock dispatcher just substring-matches on it.
    const TX_SIG: &str =
        "5VfYdM3GjRZqkBdYNz2hVnQYsBfP1k8fL3jHkMb7vYqXrJzGw2XaRpUyMcNvDsW4eLkR1tFqGxKyPmAhU6Dv8nQT";
    // Synthetic operator treasury + mainnet USDC mint. The mock proof says
    // this exact owner+mint received `CHEAP_COST` micro-USDC.
    const TREASURY: &str = "TreaSurYMockPayToMnemonik11111111111111111";
    const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    // Anchor PUT succeeds; GET returns 404 → refetch budget exhausts, the
    // delivery check exits at the `refetch` stage — same shape as
    // `demotion_on_refetch_failure` but under PAYMENT_MODE=x402 instead of
    // `balance`. (Using `read_fails` rather than `corrupted_get` because
    // the latter routes through `verify_cose` and ends at stage=`verify`,
    // which is a separate test in this file.)
    let arweave = MockArweave::read_fails("AR_TX_X402");
    arweave.install();
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

    // Submit the request with X-Payment pointing at TX_SIG.
    let (status, envelope) = call_sign_memory_participate_x402(&app, TX_SIG, "hello-x402").await;
    assert_eq!(status, StatusCode::OK);

    // Same -32011 shape as the other demotion tests.
    let err = envelope["error"]
        .as_object()
        .expect("expected JSON-RPC error envelope");
    assert_eq!(err["code"], -32011);
    let data = err["data"].as_object().expect("data");
    assert_eq!(data["stage"], "refetch");
    assert_eq!(data["row_demoted_to"], "local");
    let attestation_id = data["attestation_id"]
        .as_str()
        .expect("attestation_id in -32011 error data")
        .to_string();

    let store = state.store.lock().unwrap();

    // Demotion landed in storage.
    let row_write_mode: String = store
        .conn()
        .query_row(
            "SELECT write_mode FROM attestations WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |r| r.get::<_, String>(0),
        )
        .expect("row exists after demotion");
    assert_eq!(row_write_mode, "local");

    // ── THE T3.5 LOAD-BEARING ASSERTION ─────────────────────────────────
    // x402 nonce was NOT consumed: `mark_x402_nonce` fires only via
    // `consume_x402_nonce_after_success` on a confirmed delivery. A failed
    // delivery must leave the row absent so retry with the same payment
    // succeeds without a misleading "already consumed" rejection.
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
        "T3 R2 nonce-deferral: failed delivery must leave x402 nonce \
         reusable. tx_sig=`{TX_SIG}` should NOT appear in x402_nonces, \
         but it does."
    );

    // No cost row written: Participate cost-record fires only on success.
    let costs_count: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM attestation_costs WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        costs_count, 0,
        "demoted writes must not record an attestation_costs row"
    );

    drop(store);

    // Counter increments under the per-stage label for operator observability.
    assert_eq!(state.delivery_metrics.not_confirmed("refetch"), 1);
}

// ── 3. Quota guard (x402) short-circuits before chain write ─────────────────

/// The outcome-based DoS guard, now keyed on `blake3(x402 tx_sig)` (Wave 4
/// removed the custodial `blake3(api_key)` subject). Reusing the SAME tx_sig
/// is legitimate after a failed delivery (the nonce is never consumed on
/// failure — see test 2), so each retry bumps the same quota subject. After
/// `threshold` consecutive demotions the next `participate` call must
/// short-circuit with `DeliveryQuotaExceeded` BEFORE any Arweave spend.
#[tokio::test]
async fn quota_exceeded_x402_short_circuits_before_chain_write() {
    const TX_SIG: &str =
        "5VfYdM3GjRZqkBdYNz2hVnQYsBfP1k8fL3jHkMb7vYqXrJzGw2XaRpUyMcNvDsW4eLkR1tFqGxKyPmAhU6Dv8nQT";
    const TREASURY: &str = "TreaSurYMockPayToMnemonik11111111111111111";
    const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    // Anchor PUT succeeds, GET 404s → every call demotes at the refetch stage.
    let arweave = MockArweave::read_fails("AR_TX_QUOTA_X402");
    let post_tx_mock = arweave.install();
    let solana =
        MockSolana::happy_with_x402_payment(TX_SIG, TREASURY, USDC_MINT, CHEAP_COST as u64);

    // Tight threshold (3) so the 4th call trips the guard — keeps the test fast.
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

    // Three demotions (same tx_sig → same quota subject) reach the threshold.
    for i in 0..3 {
        let (status, env) =
            call_sign_memory_participate_x402(&app, TX_SIG, &format!("quota-bump-{i}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            env["error"]["data"]["kind"], "DeliveryNotConfirmed",
            "iteration {i} must demote before the quota fires"
        );
    }

    // Capture the Arweave POST hit count BEFORE the short-circuited call.
    let pre_arweave_hits = post_tx_mock.calls();

    let (status, env) = call_sign_memory_participate_x402(&app, TX_SIG, "quota-bump-final").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let err = env["error"].as_object().expect("error");
    assert_eq!(err["code"], -32011);
    assert_eq!(err["data"]["kind"], "DeliveryQuotaExceeded");

    // The short-circuit must not spend any further Arweave fee.
    assert_eq!(
        post_tx_mock.calls(),
        pre_arweave_hits,
        "quota short-circuit must not spend Arweave fees"
    );
    assert_eq!(state.delivery_metrics.quota_short_circuit(), 1);
}

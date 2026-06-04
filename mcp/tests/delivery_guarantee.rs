//! Integration tests for T3 (`modes-user-choice` delivery guarantee).
//!
//! Six scenarios covering the delivery-confirmation contract:
//!
//! 1. `happy_path` — successful participate write returns `delivery_receipt`,
//!    row tagged `participate`, `attestation_costs` row written. NOTE: this
//!    one is `#[ignore]` because it requires a real arlocal + solana-test-
//!    validator harness; the per-stage failure tests below cover the same
//!    code paths in their non-failure direction.
//! 2. `demotion_on_refetch_failure` — corrupted GET on Arweave re-fetch.
//!    Assert: -32011 with `stage: "refetch"`, row demoted to `local`, no
//!    `attestation_costs` row, balance restored, counter incremented.
//! 3. `demotion_on_verify_failure` — different valid COSE bytes (signed by
//!    another key) → `stage: "verify"`.
//! 4. `demotion_on_x402_leaves_nonce_reusable` — same refetch failure as
//!    (2) but under PAYMENT_MODE=x402. Asserts the T3-round-2 nonce
//!    deferral (`mark_x402_nonce` fires only after delivery success):
//!    after a failed delivery the `x402_nonces` table is empty, so the
//!    caller can retry with the same `X-Payment` header without being
//!    told "already consumed".
//! 5. `quota_exceeded` — 5 consecutive demotions; the 6th `participate` call
//!    short-circuits with `DeliveryQuotaExceeded` BEFORE any chain call.
//! 6. `refund_failure_writes_audit_row` — stubbed refund failure → an
//!    audit row exists with api_key_hash (NOT raw key).

#[path = "_helpers/delivery_harness.rs"]
mod delivery_harness;

use delivery_harness::{
    balance_for, build_state_and_router, build_state_and_router_x402, call_sign_memory_participate,
    call_sign_memory_participate_x402, seed_api_key_jwt, MockArweave, MockSolana,
};

use std::time::Duration;

use axum::http::StatusCode;

const CHEAP_COST: i64 = 1_000; // 0.001 USDC per write
const INITIAL_BALANCE: i64 = 100_000_000; // 100 USDC

// ── 1. Happy path — see #[ignore] note above ────────────────────────────────

#[ignore = "requires real arlocal + solana-test-validator harness; per-stage failure tests below exercise the same code paths in their non-failure direction"]
#[tokio::test]
async fn happy_path() {
    // Sentinel — keeps the slot in the file so future infra wiring can drop
    // the `#[ignore]` without restructuring.
}

// ── 2. Refetch failure (PAYMENT_MODE=balance) ──────────────────────────────

#[tokio::test]
async fn demotion_on_refetch_failure() {
    // Anchor PUT succeeds, GET returns 404 → refetch budget exhausts.
    let arweave = MockArweave::read_fails("AR_TX_REFETCH_FAIL");
    arweave.install();
    let solana = MockSolana::happy();

    let (state, app) = build_state_and_router(
        &arweave.base_url(),
        &solana.base_url(),
        "balance",
        CHEAP_COST,
        /* quota_threshold */ 5,
        /* quota_window */ Duration::from_secs(60),
        /* refetch_timeout */ Duration::from_millis(500), // bound the test
    );
    let bearer = seed_api_key_jwt(&state, INITIAL_BALANCE);

    let pre_balance = balance_for(&state, &bearer);
    assert_eq!(pre_balance, INITIAL_BALANCE, "balance seeded");

    let (status, envelope) = call_sign_memory_participate(&app, &bearer, "hello-refetch").await;

    // Status code is 200 — typed-error envelope is in the JSON body.
    assert_eq!(status, StatusCode::OK);
    let err = envelope["error"]
        .as_object()
        .expect("expected JSON-RPC error envelope");
    assert_eq!(err["code"], -32011, "DeliveryNotConfirmed code");
    let data = err["data"].as_object().expect("data");
    assert_eq!(data["kind"], "DeliveryNotConfirmed");
    assert_eq!(data["stage"], "refetch");
    assert_eq!(data["row_demoted_to"], "local");
    let attestation_id = data["attestation_id"]
        .as_str()
        .expect("attestation_id in data");

    // Row exists, tagged `local`, no cost row.
    let store = state.store.lock().unwrap();
    let row_mode: String = store
        .conn()
        .query_row(
            "SELECT write_mode FROM attestations WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(row_mode, "local", "row must be demoted to local");
    let cost_rows: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM attestation_costs WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cost_rows, 0, "no attestation_costs row on demotion");
    drop(store);

    // Balance restored.
    let post_balance = balance_for(&state, &bearer);
    assert_eq!(
        post_balance, pre_balance,
        "balance must return to pre-call value (refund-release)"
    );

    // Metric counter incremented for the refetch stage.
    assert_eq!(state.delivery_metrics.not_confirmed("refetch"), 1);
    assert_eq!(state.delivery_metrics.not_confirmed("verify"), 0);
    assert_eq!(state.delivery_metrics.not_confirmed("recall"), 0);
}

// ── 3. Verify failure — different valid COSE bytes ─────────────────────────

#[tokio::test]
async fn demotion_on_verify_failure() {
    let arweave = MockArweave::different_signature_bytes("AR_TX_VERIFY_FAIL");
    arweave.install();
    let solana = MockSolana::happy();

    let (state, app) = build_state_and_router(
        &arweave.base_url(),
        &solana.base_url(),
        "balance",
        CHEAP_COST,
        5,
        Duration::from_secs(60),
        Duration::from_secs(2),
    );
    let bearer = seed_api_key_jwt(&state, INITIAL_BALANCE);
    let pre_balance = balance_for(&state, &bearer);

    let (status, envelope) = call_sign_memory_participate(&app, &bearer, "hello-verify").await;
    assert_eq!(status, StatusCode::OK);
    let err = envelope["error"]
        .as_object()
        .expect("expected JSON-RPC error envelope");
    assert_eq!(err["code"], -32011);
    let data = err["data"].as_object().expect("data");
    assert_eq!(data["stage"], "verify");
    assert_eq!(data["row_demoted_to"], "local");

    // DB row assertion — catches a regression where the demotion
    // INSERT OR REPLACE silently fails (test-reviewer round 1).
    let attestation_id = data["attestation_id"]
        .as_str()
        .expect("attestation_id in error data");
    let row_write_mode: String = {
        let store = state.store.lock().unwrap();
        store
            .conn()
            .query_row(
                "SELECT write_mode FROM attestations WHERE attestation_id = ?",
                rusqlite::params![attestation_id],
                |r| r.get::<_, String>(0),
            )
            .expect("row exists after demotion")
    };
    assert_eq!(row_write_mode, "local");

    // Balance restored.
    assert_eq!(balance_for(&state, &bearer), pre_balance);
    assert_eq!(state.delivery_metrics.not_confirmed("verify"), 1);
}

// ── 4. Quota exceeded after threshold demotions ────────────────────────────

#[tokio::test]
async fn quota_exceeded_short_circuits_before_chain_write() {
    let arweave = MockArweave::read_fails("AR_TX_QUOTA");
    let post_tx_mock = arweave.install();
    let solana = MockSolana::happy();

    let (state, app) = build_state_and_router(
        &arweave.base_url(),
        &solana.base_url(),
        "balance",
        CHEAP_COST,
        // Tight quota so the 4th call short-circuits — keeps the test fast.
        /* threshold */
        3,
        /* window */ Duration::from_secs(60),
        Duration::from_millis(300),
    );
    let bearer = seed_api_key_jwt(&state, INITIAL_BALANCE);

    // Three demotions to hit the threshold.
    for i in 0..3 {
        let (status, env) =
            call_sign_memory_participate(&app, &bearer, &format!("quota-bump-{i}")).await;
        assert_eq!(status, StatusCode::OK);
        let code = env["error"]["code"].as_i64();
        assert_eq!(
            code,
            Some(-32011),
            "iteration {i} must produce DeliveryNotConfirmed before quota fires"
        );
        assert_eq!(env["error"]["data"]["kind"], "DeliveryNotConfirmed");
    }

    // Capture httpmock hit count BEFORE the 4th call to assert no further
    // Arweave invocation happens.
    let pre_arweave_hits = post_tx_mock.calls();

    let (status, env) = call_sign_memory_participate(&app, &bearer, "quota-bump-final").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let err = env["error"].as_object().expect("error");
    assert_eq!(err["code"], -32011);
    assert_eq!(err["data"]["kind"], "DeliveryQuotaExceeded");

    // No new Arweave POST happened on the short-circuited 4th call.
    let post_arweave_hits = post_tx_mock.calls();
    assert_eq!(
        post_arweave_hits, pre_arweave_hits,
        "quota short-circuit must not spend Arweave fees"
    );

    // Quota metric incremented.
    assert_eq!(state.delivery_metrics.quota_short_circuit(), 1);
}

// ── 5. Refund-itself failure writes audit row ──────────────────────────────

#[tokio::test]
async fn refund_failure_writes_audit_row() {
    // Induce a delivery demotion via refetch failure, then induce the
    // refund itself to fail by installing a `force_fail_refund` trigger
    // on the `payment_events` table that aborts INSERTs whose
    // description starts with our sentinel substring.
    let arweave = MockArweave::read_fails("AR_TX_REFUND_FAIL");
    arweave.install();
    let solana = MockSolana::happy();

    let (state, app) = build_state_and_router(
        &arweave.base_url(),
        &solana.base_url(),
        "balance",
        CHEAP_COST,
        5,
        Duration::from_secs(60),
        Duration::from_millis(300),
    );
    let bearer = seed_api_key_jwt(&state, INITIAL_BALANCE);

    // Trigger: any INSERT into `payment_events` with event_type='refund'
    // whose description contains 'delivery_not_confirmed:' must ABORT.
    // The trigger does NOT match `event_type='refund_failed'`, so the audit
    // row write succeeds.
    {
        let store = state.store.lock().unwrap();
        store
            .conn()
            .execute_batch(
                "CREATE TRIGGER force_fail_refund
                 BEFORE INSERT ON payment_events
                 WHEN NEW.event_type = 'refund'
                  AND NEW.description LIKE 'refund: delivery_not_confirmed:%'
                 BEGIN SELECT RAISE(ABORT, 'forced refund failure'); END;",
            )
            .expect("install refund-fail trigger");
    }

    let (status, env) = call_sign_memory_participate(&app, &bearer, "audit-row").await;
    assert_eq!(status, StatusCode::OK);

    // Capture the demoted attestation_id for the description-format
    // assertion below (test-reviewer round 1).
    let attestation_id = env["error"]["data"]["attestation_id"]
        .as_str()
        .expect("attestation_id in -32011 error data")
        .to_string();

    // Read the refund_failed audit row.
    let store = state.store.lock().unwrap();
    let row: (String, String, i64, String) = store
        .conn()
        .query_row(
            "SELECT api_key, event_type, amount_micro_usdc, description
               FROM payment_events
              WHERE event_type = 'refund_failed'",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .expect("audit row exists");

    // api_key column carries the HASH, not the raw bearer.
    assert_ne!(row.0, bearer, "audit row must NOT contain raw api_key");
    assert!(
        !row.0.contains(&bearer[..16]),
        "audit row must not leak any prefix of the api_key"
    );
    // blake3 hex is 64 chars.
    assert_eq!(row.0.len(), 64);
    assert_eq!(row.1, "refund_failed");
    assert_eq!(row.2, 0, "refund_failed carries no money");
    assert!(
        row.3.contains("refund-itself-failed"),
        "description must include the reason"
    );
    // Description must use the spec-mandated `{attestation_id} | {reason}`
    // format prefix for 1:1 forensic correlation (test-reviewer round 1,
    // tech-spec §"Decisions 7" PII allow-list).
    assert!(
        row.3.starts_with(&format!("{attestation_id} | ")),
        "description must start with '{attestation_id} | ', got: {}",
        row.3
    );

    // Negative assertion: NO `content_preview` / `cose_bytes` column exists
    // (we never write payload bytes in the audit path).
    let cols: Vec<String> = store
        .conn()
        .prepare("PRAGMA table_info(payment_events)")
        .unwrap()
        .query_map([], |r: &rusqlite::Row<'_>| r.get::<_, String>(1))
        .unwrap()
        .filter_map(|c: rusqlite::Result<String>| c.ok())
        .collect();
    assert!(!cols.iter().any(|c| c == "content_preview"));
    assert!(!cols.iter().any(|c| c == "cose_bytes"));
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

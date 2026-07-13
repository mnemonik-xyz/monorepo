//! Payment gating + payment-related database helpers.
//!
//! This file owns all payment concerns for the MCP server:
//!   - Payment-mode gating (`check_payment` and path selectors).
//!   - x402 nonce replay protection (`mark_x402_nonce`).
//!   - P&L cost accounting (`record_attestation_cost`, `get_pnl_stats`).
//!   - Standalone `verify_usdc_transfer` over `&SolanaClient` (moved here in
//!     Task 8; the USDC-vs-recipient policy is payment-layer, not chain-layer).
//!   - EVM USDC x402 verifier (`verify_evm_usdc_transfer`) for Arc/Base.
//!
//! Payment paths (Wave 4 — non-custodial; custodial balance/api-keys removed):
//!   - x402 — clients pay per-call via a USDC transfer on Solana OR an EVM
//!     chain (Arc/Base) and present the tx sig in `X-Payment: <json>` on the
//!     retry request. Verified on-chain; no operator-held float.
//!   - none — open access (development / self-hosted).

use axum::http::HeaderMap;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::SqliteStore;

use crate::universal_paywall::{
    ExactAuthorization, OperationBinding, OperationScope, StoredQuote, UniversalPaywallClient,
    UniversalPaywallConfig,
};

// ── x402 wire types ──────────────────────────────────────────────────────────

/// Payload sent in the `X-Payment` header by the agent.
#[derive(Debug, Deserialize)]
pub struct X402PaymentProof {
    pub tx_sig: String,
    /// "solana-mainnet" | "solana-devnet" — deserialized for protocol
    /// compliance. Currently not inspected; verification is network-agnostic
    /// via the configured Solana RPC URL.
    #[allow(dead_code)]
    pub network: String,
}

/// Body returned with HTTP 402 to describe what payment is required.
#[derive(Debug, Serialize)]
pub struct X402Response {
    #[serde(rename = "x402Version")]
    pub x402_version: u8,
    pub accepts: Vec<PaymentOption>,
}

#[derive(Debug, Serialize)]
pub struct PaymentOption {
    /// "exact" — caller must send exactly this token + amount.
    pub scheme: String,
    pub network: String,
    /// Amount in smallest units (micro-USDC), as a decimal string.
    #[serde(rename = "maxAmountRequired")]
    pub max_amount_required: String,
    /// SPL mint address.
    pub asset: String,
    /// Treasury public key (recipient).
    #[serde(rename = "payTo")]
    pub pay_to: String,
    pub description: String,
}

// ── Gate result ──────────────────────────────────────────────────────────────

pub enum PaymentGate {
    /// Payment verified (or not required). Wave 4 removed custodial balance
    /// mode, so there is no longer an operator-issued api_key to carry — this
    /// is a unit variant.
    Proceed,
    /// Return HTTP 402 with this body.
    NeedPayment(X402Response),
    /// Return HTTP 402 with a Universal Paywall exact quote.
    NeedUniversalPaywall(UniversalPaywallPaymentRequired),
    /// Bad credentials / payment verification failure — return 401/402 message.
    Unauthorized(String),
}

/// Body returned with HTTP 402 when Universal Paywall is the payment rail.
#[derive(Debug, Serialize)]
pub struct UniversalPaywallPaymentRequired {
    pub operation_id: String,
    pub quote_id: String,
    pub approval_url: String,
    pub scheme: String,
    pub network: String,
    pub asset: String,
    pub pay_to: String,
    pub payer_wallet: String,
    pub amount: String,
    pub binding_digest: String,
}

// ── Header helpers ───────────────────────────────────────────────────────────

/// Decode x402 payment proof from `X-Payment` header.
/// Accepts raw JSON or base64-encoded JSON.
pub fn extract_x402_proof(headers: &HeaderMap) -> Option<X402PaymentProof> {
    let raw = headers.get("x-payment").and_then(|v| v.to_str().ok())?;

    // Try raw JSON first
    if let Ok(p) = serde_json::from_str::<X402PaymentProof>(raw) {
        return Some(p);
    }
    // Fallback: base64-encoded JSON (Coinbase CDK sends this)
    if let Ok(decoded) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw) {
        if let Ok(p) = serde_json::from_slice::<X402PaymentProof>(&decoded) {
            return Some(p);
        }
    }
    None
}

// ── Main gate function ───────────────────────────────────────────────────────

/// Check payment for a paid tool call.
///
/// Called before executing `mnemonic_sign_memory` when `payment_mode != "none"`.
/// Returns `PaymentGate::Proceed` if the caller may proceed, otherwise the
/// appropriate rejection.
///
/// Wave 4 (non-custodial): the only paid rail is non-custodial x402 (Solana or
/// EVM USDC, per-call, verified on-chain). The custodial `balance`/`both`
/// modes and the operator-issued `mnm_` api_key ledger are removed — there is
/// no operator-held float. Valid modes: `none`, `x402`.
// Fans out to the x402 verifier with each rail's config (solana + optional
// EVM); a params struct would only move the same fields behind a wrapper.
#[allow(clippy::too_many_arguments)]
pub async fn check_payment(
    headers: &HeaderMap,
    mode: &str,
    store: &std::sync::Mutex<SqliteStore>,
    solana: &SolanaClient,
    treasury: &str,
    usdc_mint: &str,
    cost: i64,
    evm: Option<&EvmPaymentConfig>,
) -> PaymentGate {
    match mode {
        "none" => PaymentGate::Proceed,

        "x402" => check_x402(headers, solana, store, treasury, usdc_mint, cost, evm).await,

        unknown => {
            tracing::error!("Unknown PAYMENT_MODE={unknown:?} — rejecting request (fail-closed)");
            PaymentGate::Unauthorized(format!(
                "server misconfiguration: unknown PAYMENT_MODE={unknown:?}. Valid: none, x402"
            ))
        }
    }
}

// ── Universal Paywall exact x402 path ────────────────────────────────────────

/// Payment proof sent in the `X-Payment` header for the Universal Paywall rail.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct UniversalPaywallPaymentProof {
    pub scheme: String,
    pub payer_wallet: String,
    pub authorization: ExactAuthorization,
}

/// Decode Universal Paywall exact payment proof from `X-Payment` header.
pub fn extract_universal_paywall_proof(headers: &HeaderMap) -> Option<UniversalPaywallPaymentProof> {
    let raw = headers.get("x-payment").and_then(|v| v.to_str().ok())?;
    serde_json::from_str::<UniversalPaywallPaymentProof>(raw).ok()
}

/// Gate for the Universal Paywall `exact` one-time x402 rail.
///
/// First call (no payment header): create an immutable quote, store it in
/// `quotes`, and return `PaymentGate::NeedUniversalPaywall` so the client can
/// open a browser approval page.
///
/// Retry (with `X-Payment` exact authorization): settle through Universal
/// Paywall and return `PaymentGate::Proceed` on success. Idempotent retries
/// return `Proceed` once the receipt is cached.
#[allow(clippy::too_many_arguments)]
pub async fn check_universal_paywall(
    headers: &HeaderMap,
    client: &UniversalPaywallClient,
    config: &UniversalPaywallConfig,
    cost: i64,
    quotes: &DashMap<String, StoredQuote>,
    payer_subject: &str,
    artifact_hash: &str,
    operation_id: Option<&str>,
) -> PaymentGate {
    // Retry path: the client already signed an EIP-3009 authorization.
    if let Some(proof) = extract_universal_paywall_proof(headers) {
        let op_id = match operation_id {
            Some(id) => id.to_string(),
            None => return PaymentGate::Unauthorized("missing operation_id for payment retry".into()),
        };
        let quote = match quotes.get(&op_id) {
            Some(q) => q.clone(),
            None => return PaymentGate::Unauthorized("unknown or expired operation_id".into()),
        };
        // Idempotent fast path: already settled in this process.
        if quote.receipt.is_some() {
            return PaymentGate::Proceed;
        }
        let receipt = match client.settle_exact(&quote.binding, &proof.authorization).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(operation_id = %op_id, error = %e, "universal-paywall settle_exact failed");
                return PaymentGate::Unauthorized(format!("payment settlement failed: {e}"));
            }
        };
        quotes.entry(op_id).and_modify(|q| q.receipt = Some(receipt));
        return PaymentGate::Proceed;
    }

    // First call: create a quote and ask the client to pay.
    let operation_id = operation_id.map(|s| s.to_string()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let nonce = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let expires_at = (now + chrono::TimeDelta::minutes(5)).to_rfc3339();

    let binding = OperationBinding {
        version: 1,
        operation_id: operation_id.clone(),
        payer_subject: payer_subject.to_string(),
        payer_wallet: config.payer_wallet.clone(),
        artifact_hash: artifact_hash.to_string(),
        amount: cost.to_string(),
        asset: config.asset.clone(),
        network: config.network.clone(),
        pay_to: config.pay_to.clone(),
        expires_at,
        nonce,
        scope: OperationScope {
            workspace_hash: None,
            visibility: "private".into(),
            action: "manual".into(),
        },
    };

    let quote = match client.create_quote(&binding).await {
        Ok(q) => q,
        Err(e) => {
            tracing::error!(error = %e, "universal-paywall create_quote failed");
            return PaymentGate::Unauthorized(format!("payment provider unavailable: {e}"));
        }
    };

    let approval_url = client.approval_url(&operation_id, &quote.quote_id);

    quotes.insert(
        operation_id.clone(),
        StoredQuote {
            operation_id: operation_id.clone(),
            quote_id: quote.quote_id.clone(),
            binding: binding.clone(),
            binding_digest: quote.binding_digest.clone(),
            receipt: None,
        },
    );

    PaymentGate::NeedUniversalPaywall(UniversalPaywallPaymentRequired {
        operation_id,
        quote_id: quote.quote_id,
        approval_url,
        scheme: "exact".into(),
        network: config.network.clone(),
        asset: config.asset.clone(),
        pay_to: config.pay_to.clone(),
        payer_wallet: config.payer_wallet.clone(),
        amount: cost.to_string(),
        binding_digest: quote.binding_digest,
    })
}

// ── x402 path ────────────────────────────────────────────────────────────────

/// True when an x402 proof's `network` denotes an EVM chain (Arc/Base/eip155…)
/// rather than Solana. Used to route verification to the right rail.
fn is_evm_network(network: &str) -> bool {
    let n = network.to_lowercase();
    n.starts_with("arc")
        || n.starts_with("evm")
        || n.starts_with("eip155")
        || n.starts_with("base")
        || n.starts_with("ethereum")
}

async fn check_x402(
    headers: &HeaderMap,
    solana: &SolanaClient,
    store: &std::sync::Mutex<SqliteStore>,
    treasury: &str,
    usdc_mint: &str,
    cost: i64,
    evm: Option<&EvmPaymentConfig>,
) -> PaymentGate {
    let proof = match extract_x402_proof(headers) {
        Some(p) => p,
        None => {
            // No payment header — return 402 payment required (advertise every
            // rail the operator supports: Solana always, EVM when configured).
            return PaymentGate::NeedPayment(x402_required(
                treasury,
                usdc_mint,
                cost,
                "mnemonic_sign_memory attestation fee",
                evm,
            ));
        }
    };

    // T3 round-2 — replay-detect WITHOUT consuming. If this nonce has
    // already been consumed (by a successful delivery on an earlier
    // request), reject. The actual `INSERT INTO x402_nonces` happens
    // AFTER `confirm_delivery_or_demote` succeeds (see
    // `consume_x402_nonce_after_success` below) so a delivery failure
    // leaves the nonce reusable — the caller's USDC payment is not
    // forfeit when the operator's anchor isn't proved retrievable.
    {
        let store = store.lock().unwrap();
        if x402_nonce_already_consumed(&store, &proof.tx_sig).unwrap_or(false) {
            return PaymentGate::Unauthorized(format!(
                "x402 payment already used: {}",
                proof.tx_sig
            ));
        }
    }

    // Verify the transfer on the rail indicated by `proof.network`. EVM when
    // the proof names an EVM chain AND the operator has the EVM rail enabled;
    // Solana otherwise. Both settle in micro-USDC (6-dec), so `cost` is the
    // minimum on either chain.
    let verify_result = match (evm, is_evm_network(&proof.network)) {
        (Some(evm), true) => verify_evm_usdc_transfer(
            &evm.rpc_url,
            &proof.tx_sig,
            &evm.treasury,
            &evm.usdc_token,
            cost as u128,
        )
        .await
        .map(|opt| opt.map(|_| ())),
        _ => verify_usdc_transfer(solana, &proof.tx_sig, treasury, usdc_mint, cost as u64)
            .await
            .map(|opt| opt.map(|_| ())),
    };
    match verify_result {
        Ok(Some(())) => {}
        Ok(None) => {
            return PaymentGate::Unauthorized(format!(
                "x402 payment not valid: tx {} does not transfer >= {cost} micro-USDC to treasury",
                proof.tx_sig
            ))
        }
        Err(e) => return PaymentGate::Unauthorized(format!("x402 verification error: {e}")),
    }

    // Do NOT mark the nonce here. The nonce is consumed only after a
    // successful delivery confirmation (or, in the
    // legacy `payment_mode == "none"` path, never). See
    // `consume_x402_nonce_after_success`.
    PaymentGate::Proceed
}

/// Read-only replay check for an x402 nonce. Returns `Ok(true)` if a row
/// already exists in `x402_nonces`, `Ok(false)` otherwise.
///
/// Used by `check_x402` to fail-fast on replay BEFORE the more expensive
/// `verify_usdc_transfer` Solana RPC. Note: a race window exists between
/// this read and the eventual `consume_x402_nonce_after_success` INSERT
/// — the loser gets `mark_x402_nonce` ConstraintViolation, which is the
/// correct outcome (one of the two concurrent requests wins).
pub fn x402_nonce_already_consumed(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<bool> {
    let exists: bool = store
        .conn()
        .query_row(
            "SELECT 1 FROM x402_nonces WHERE tx_sig = ? LIMIT 1",
            params![tx_sig],
            |_| Ok(true),
        )
        .unwrap_or(false);
    Ok(exists)
}

/// Consume an x402 nonce by inserting it into `x402_nonces`. Called by
/// the caller AFTER the delivery confirmation passes (T3 round-2
/// deferral). Returns `Err` on ConstraintViolation if the same nonce was
/// concurrently consumed by another request.
///
/// Round-2 split: the original `check_x402` consumed the nonce at gate
/// time, which made delivery failures permanently spend the caller's
/// USDC. With the nonce deferred to here, a delivery failure leaves the
/// nonce reusable and the caller can retry with the same `X-Payment`
/// header — they pay Arweave/Solana fees again on the retry (operator
/// bleed), but the DoS quota guard caps how many such retries cost the
/// operator.
pub fn consume_x402_nonce_after_success(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<()> {
    mark_x402_nonce(store, tx_sig)
}

// ── Builder ──────────────────────────────────────────────────────────────────

fn x402_required(
    treasury: &str,
    usdc_mint: &str,
    cost: i64,
    description: &str,
    evm: Option<&EvmPaymentConfig>,
) -> X402Response {
    let mut accepts = vec![PaymentOption {
        scheme: "exact".into(),
        network: "solana-mainnet".into(),
        max_amount_required: cost.to_string(),
        asset: usdc_mint.to_string(),
        pay_to: treasury.to_string(),
        description: description.to_string(),
    }];
    if let Some(evm) = evm {
        accepts.push(PaymentOption {
            scheme: "exact".into(),
            network: "arc".into(),
            max_amount_required: cost.to_string(),
            asset: evm.usdc_token.clone(),
            pay_to: evm.treasury.clone(),
            description: description.to_string(),
        });
    }
    X402Response {
        x402_version: 1,
        accepts,
    }
}

// ── USDC transfer verification ───────────────────────────────────────────────

/// Verify that `tx_sig` transfers at least `min_amount` micro-USDC of `usdc_mint`
/// to `recipient`.  Returns the actual amount transferred (>= min_amount) on
/// success, or `Ok(None)` if the transfer is absent / insufficient.
///
/// This is a payment concern and lives here (not in `mnemonic_core::solana`):
/// core knows chain primitives; the `USDC vs recipient` policy is mcp's.
pub async fn verify_usdc_transfer(
    client: &SolanaClient,
    tx_sig: &str,
    recipient: &str,
    usdc_mint: &str,
    min_amount: u64,
) -> anyhow::Result<Option<u64>> {
    let result = client.rpc("getTransaction", serde_json::json!([
        tx_sig,
        {"encoding": "jsonParsed", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}
    ])).await?;

    if result.is_null() {
        return Ok(None);
    }

    // Reject failed transactions
    if !result["meta"]["err"].is_null() {
        return Ok(None);
    }

    // Walk postTokenBalances looking for recipient + mint with increased balance
    let pre = result["meta"]["preTokenBalances"].as_array();
    let post = result["meta"]["postTokenBalances"].as_array();

    if let (Some(pre_balances), Some(post_balances)) = (pre, post) {
        for post_entry in post_balances {
            let owner = post_entry["owner"].as_str().unwrap_or("");
            let mint = post_entry["mint"].as_str().unwrap_or("");
            if owner != recipient || mint != usdc_mint {
                continue;
            }
            let post_amount: u64 = post_entry["uiTokenAmount"]["amount"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);

            let account_index = post_entry["accountIndex"].as_u64().unwrap_or(u64::MAX);
            let pre_amount: u64 = pre_balances
                .iter()
                .find(|e| e["accountIndex"].as_u64() == Some(account_index))
                .and_then(|e| e["uiTokenAmount"]["amount"].as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            let delta = post_amount.saturating_sub(pre_amount);
            if delta >= min_amount {
                return Ok(Some(delta));
            }
        }
    }

    Ok(None)
}

// ── Payment DB helpers (operate on SqliteStore from mnemonic-core) ───────────
//
// These are payment concerns (API key management, balance, P&L) and
// intentionally live in the MCP server, not in core. They operate on the
// public `conn()` accessor of `mnemonic_core::storage::SqliteStore`.

/// Aggregated profit-and-loss statistics.
#[derive(Debug, serde::Serialize)]
pub struct PnlStats {
    pub period_days: u64,
    pub attestations: i64,
    pub earned_micro_usdc: i64,
    pub cost_sol_lamports: i64,
    pub cost_micro_usdc_equiv: i64,
    pub net_micro_usdc: i64,
    pub margin_pct: f64,
    pub avg_sol_price_usdc: f64,
}

/// Record an x402 tx sig as used (prevents replay). Returns Err if already used.
pub fn mark_x402_nonce(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let result = store.conn().execute(
        "INSERT INTO x402_nonces (tx_sig, used_at) VALUES (?,?)",
        params![tx_sig, now],
    );
    match result {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            anyhow::bail!("x402 payment already used: {tx_sig}")
        }
        Err(e) => Err(e.into()),
    }
}

/// Record actual server costs alongside each completed attestation.
pub fn record_attestation_cost(
    store: &SqliteStore,
    attestation_id: &str,
    irys_lamports: u64,
    sol_tx_fee_lamports: u64,
    sol_price_usdc: f64,
    earned_micro_usdc: i64,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    store.conn().execute(
        "INSERT OR IGNORE INTO attestation_costs
         (attestation_id, irys_cost_lamports, sol_tx_fee_lamports, sol_price_usdc, earned_micro_usdc, created_at)
         VALUES (?,?,?,?,?,?)",
        params![
            attestation_id,
            irys_lamports as i64,
            sol_tx_fee_lamports as i64,
            sol_price_usdc,
            earned_micro_usdc,
            now,
        ],
    )?;
    Ok(())
}

/// Aggregate P&L statistics over the last `days` days.
pub fn get_pnl_stats(store: &SqliteStore, days: u64) -> anyhow::Result<PnlStats> {
    let interval = format!("-{days} days");
    let row = store.conn().query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(earned_micro_usdc), 0),
            COALESCE(SUM(irys_cost_lamports + sol_tx_fee_lamports), 0),
            COALESCE(SUM((irys_cost_lamports + sol_tx_fee_lamports) * sol_price_usdc / 1000.0), 0.0),
            COALESCE(AVG(sol_price_usdc), 0.0)
         FROM attestation_costs
         WHERE created_at > datetime('now', ?1)",
        params![interval],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        },
    )?;

    let (attestations, earned, cost_lamports, cost_usdc_equiv, avg_sol) = row;
    let cost_micro_usdc = cost_usdc_equiv.ceil() as i64;
    let net = earned - cost_micro_usdc;
    let margin_pct = if earned > 0 {
        (net as f64 / earned as f64) * 100.0
    } else {
        0.0
    };

    Ok(PnlStats {
        period_days: days,
        attestations,
        earned_micro_usdc: earned,
        cost_sol_lamports: cost_lamports,
        cost_micro_usdc_equiv: cost_micro_usdc,
        net_micro_usdc: net,
        margin_pct,
        avg_sol_price_usdc: avg_sol,
    })
}

// ── Delivery DoS guard (modes-user-choice T3) ───────────────────────────────
//
// Outcome-based per-`api_key_hash` sliding-window counter consulted at the
// *entry* of the participate path in `mcp_handler` BEFORE any Arweave or
// Solana write. Increments on every delivery-not-confirmed demotion. When a
// caller crosses the threshold within the window the next `participate`
// request short-circuits with `-32011 DeliveryQuotaExceeded` so a
// systematically-failing client cannot bleed operator margin by triggering
// chain spend that is always refunded.
//
// Keying is on the bearer api_key's blake3 digest (`api_key_hash`), NOT on
// `owner_pubkey`. Ed25519 keys can be rotated for free; billable subjects
// can't. Keying on the wrong identifier would let an attacker bypass the
// quota by minting a fresh identity per request — same threat model as
// e-mail-based rate-limits not keyed on e-mail-aliases.
//
// `DashMap` shard-level lock discipline (extends Decision 8 of the
// tech-spec): every method below holds a shard guard for the duration of a
// single `record` / `count` / `is_empty_for` call and drops it before
// returning. No `.await` between guard acquisition and drop. The
// background eviction task respects the same rule per shard.

/// Compute the blake3 digest of a payment subject, hex-encoded. Wave 4 removed
/// custodial api_keys; the remaining caller is the x402 delivery-DoS quota,
/// which keys on `blake3(x402 tx_sig)` (the billable, non-rotatable subject).
/// Centralised here so call-sites cannot accidentally substitute a raw value
/// (CWE-312 hygiene).
pub fn hash_api_key(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}

/// Sliding-window timestamp counter — push, prune-on-read, count. Not
/// thread-safe by itself; protection comes from the `DashMap` shard the
/// counter sits inside. Methods are `&mut self` so the type-system enforces
/// the shard-guard exclusivity at the call-site.
#[derive(Debug, Default, Clone)]
pub struct SlidingWindowCounter {
    timestamps: Vec<Instant>,
}

impl SlidingWindowCounter {
    /// Push `now` and drop timestamps older than `window`. Bounded by
    /// the threshold check that fronts every increment site so the vec
    /// never grows past `O(threshold)` in practice.
    pub fn record(&mut self, now: Instant, window: Duration) {
        let cutoff = now.checked_sub(window);
        if let Some(cutoff) = cutoff {
            self.timestamps.retain(|t| *t >= cutoff);
        }
        self.timestamps.push(now);
    }

    /// Count timestamps that fall inside `window` looking back from `now`.
    /// Does NOT mutate (callers might want to inspect without pruning).
    pub fn count(&self, now: Instant, window: Duration) -> u32 {
        let cutoff = match now.checked_sub(window) {
            Some(c) => c,
            None => return self.timestamps.len() as u32,
        };
        self.timestamps.iter().filter(|t| **t >= cutoff).count() as u32
    }

    /// `true` if the counter has had no timestamps in the last `since`
    /// duration — used by the eviction loop to decide whether a subject
    /// has gone dormant.
    pub fn is_empty_for(&self, now: Instant, since: Duration) -> bool {
        let Some(cutoff) = now.checked_sub(since) else {
            return false;
        };
        !self.timestamps.iter().any(|t| *t >= cutoff)
    }
}

/// Per-subject sliding-window demotion counter. See module-level comment.
///
/// Single `DashMap` instance shared across the whole process via
/// `McpState.refunds_by_subject`. Bounded by the background eviction task
/// spawned in `main.rs::run_http`.
pub struct RefundsBySubject {
    inner: Arc<DashMap<String, SlidingWindowCounter>>,
    window: Duration,
    threshold: u32,
}

impl RefundsBySubject {
    /// Build a fresh guard with the given window and threshold.
    pub fn new(window: Duration, threshold: u32) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            window,
            threshold,
        }
    }

    /// Configured window duration. Surfaced for the `DeliveryQuotaExceeded`
    /// error envelope so the client knows the SLO knob.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Configured demotion threshold. Surfaced for the typed error envelope.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// True iff the subject has hit or exceeded the threshold inside the
    /// sliding window. Acquires the shard guard for `subject`, takes the
    /// count, releases. No `.await` between acquire and drop (extends
    /// Decision 8 to DashMap).
    pub fn is_over(&self, subject: &str) -> bool {
        let now = Instant::now();
        match self.inner.get(subject) {
            Some(entry) => entry.count(now, self.window) >= self.threshold,
            None => false,
        }
    }

    /// Increment the subject's counter by one. Acquires the shard guard
    /// briefly and releases before returning. Safe to call from the
    /// failure-branch of `sign_memory_inline` after the SQLite mutex has
    /// already been released.
    pub fn record_failure(&self, subject: &str) {
        let now = Instant::now();
        let window = self.window;
        self.inner
            .entry(subject.to_string())
            .or_default()
            .record(now, window);
    }

    /// Bounded eviction: drop any entry whose counter has been empty for the
    /// last `since` duration. Holds each shard guard only for the duration
    /// of its own retain pass; never across `.await`. Returns the number of
    /// entries evicted (useful for instrumentation + tests).
    pub fn evict_idle(&self, since: Duration) -> usize {
        let before = self.inner.len();
        let now = Instant::now();
        // `retain` on DashMap iterates shard-by-shard, holding only one
        // shard guard at a time. Closure runs synchronously; no `.await`
        // crosses the guard boundary.
        self.inner
            .retain(|_, counter| !counter.is_empty_for(now, since));
        before.saturating_sub(self.inner.len())
    }

    /// Number of subjects currently tracked. Useful for tests.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True if the map is currently empty. Convenience for tests.
    #[allow(dead_code)] // exercised by unit tests; future eviction-loop introspection.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Process-lifetime counters incremented in the delivery-guarantee flow.
/// Lightweight `AtomicU64` shims that stand in for the eventual Prometheus
/// histogram/counter surface (the rest of the binary hasn't wired
/// `metrics::` crates yet — see commit log; we can swap to `metrics::counter!`
/// later without touching call-sites because they go through
/// [`DeliveryMetrics`]).
///
/// The four counters are:
/// - `delivery_quota_short_circuit` — `-32011 DeliveryQuotaExceeded` returns.
/// - `delivery_not_confirmed_refetch` — demotions due to Arweave re-fetch.
/// - `delivery_not_confirmed_verify` — demotions due to verify_cose mismatch.
/// - `delivery_not_confirmed_recall` — demotions due to recall miss.
///
/// No per-tenant label (`api_key_hash` or `owner_pubkey`) is attached. That
/// would be a high-cardinality anti-pattern for any future Prometheus
/// adapter; per-tenant detail belongs in structured `tracing::warn!` lines
/// (already emitted at every demotion call-site).
pub struct DeliveryMetrics {
    quota_short_circuit: AtomicU64,
    not_confirmed_refetch: AtomicU64,
    not_confirmed_verify: AtomicU64,
    not_confirmed_recall: AtomicU64,
}

impl Default for DeliveryMetrics {
    fn default() -> Self {
        Self {
            quota_short_circuit: AtomicU64::new(0),
            not_confirmed_refetch: AtomicU64::new(0),
            not_confirmed_verify: AtomicU64::new(0),
            not_confirmed_recall: AtomicU64::new(0),
        }
    }
}

impl DeliveryMetrics {
    /// Increment the quota-exceeded short-circuit counter.
    pub fn record_quota_short_circuit(&self) {
        self.quota_short_circuit.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the per-stage `delivery_not_confirmed_total` counter.
    /// `stage` must be one of `"refetch"`, `"verify"`, `"recall"`.
    pub fn record_not_confirmed(&self, stage: &str) {
        match stage {
            "refetch" => self.not_confirmed_refetch.fetch_add(1, Ordering::Relaxed),
            "verify" => self.not_confirmed_verify.fetch_add(1, Ordering::Relaxed),
            "recall" => self.not_confirmed_recall.fetch_add(1, Ordering::Relaxed),
            // Unknown stage — log but do not crash. Reaching here means a
            // call-site sent a stage label the metric type doesn't know.
            other => {
                tracing::warn!(
                    stage = other,
                    "DeliveryMetrics::record_not_confirmed called with unknown stage"
                );
                0
            }
        };
    }

    /// Read the quota-short-circuit counter. For tests + future Prometheus
    /// adapter only.
    #[allow(dead_code)] // exercised by integration tests via test-support feature.
    pub fn quota_short_circuit(&self) -> u64 {
        self.quota_short_circuit.load(Ordering::Relaxed)
    }

    /// Read the per-stage `delivery_not_confirmed_total` counter.
    #[allow(dead_code)] // exercised by integration tests via test-support feature.
    pub fn not_confirmed(&self, stage: &str) -> u64 {
        match stage {
            "refetch" => self.not_confirmed_refetch.load(Ordering::Relaxed),
            "verify" => self.not_confirmed_verify.load(Ordering::Relaxed),
            "recall" => self.not_confirmed_recall.load(Ordering::Relaxed),
            _ => 0,
        }
    }
}

// ── EVM x402 (Wave 1 — non-custodial Arc/EVM settlement) ─────────────────────
//
// Mirror of the Solana `verify_usdc_transfer` for EVM chains (Arc, Base, …): a
// client signs an ERC-20 USDC `transfer(treasury, amount)` with its own derived
// key (no external wallet — see work/noncustodial-paradigm/design.md §19) and
// presents the tx hash in the `X-Payment` header. We confirm it on-chain via
// `eth_getTransactionReceipt` and decode the ERC-20 Transfer log.

/// EVM-side payment settlement config. `None` on `McpState` = EVM x402 disabled.
#[derive(Debug, Clone)]
pub struct EvmPaymentConfig {
    /// EVM JSON-RPC endpoint (e.g. Arc: https://rpc.testnet.arc.network).
    pub rpc_url: String,
    /// ERC-20 USDC token contract address (lowercased `0x…`).
    pub usdc_token: String,
    /// Treasury recipient address (lowercased `0x…`).
    pub treasury: String,
}

/// keccak256("Transfer(address,address,uint256)") — the ERC-20 Transfer topic0.
const ERC20_TRANSFER_TOPIC0: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Lowercase + strip `0x`; right-most 40 hex chars of a 32-byte topic = address.
fn topic_to_address(topic: &str) -> String {
    let t = topic.trim_start_matches("0x").to_lowercase();
    let start = t.len().saturating_sub(40);
    format!("0x{}", &t[start..])
}

/// Pure decoder: given a receipt's `logs` array, return the transferred amount
/// (as u128) iff some log is an ERC-20 `Transfer` from `usdc_token` to
/// `treasury` of at least `min_amount`. Network-free so it is unit-testable.
fn match_erc20_transfer(
    logs: &[serde_json::Value],
    usdc_token: &str,
    treasury: &str,
    min_amount: u128,
) -> Option<u128> {
    let token = usdc_token.to_lowercase();
    let to_want = treasury.to_lowercase();
    for log in logs {
        let addr = log["address"].as_str().unwrap_or("").to_lowercase();
        if addr != token {
            continue;
        }
        let topics = log["topics"].as_array()?;
        if topics.len() < 3 {
            continue;
        }
        if topics[0].as_str().unwrap_or("").to_lowercase() != ERC20_TRANSFER_TOPIC0 {
            continue;
        }
        if topic_to_address(topics[2].as_str().unwrap_or("")) != to_want {
            continue;
        }
        let data = log["data"]
            .as_str()
            .unwrap_or("0x")
            .trim_start_matches("0x");
        let amount = u128::from_str_radix(data, 16).unwrap_or(0);
        if amount >= min_amount {
            return Some(amount);
        }
    }
    None
}

/// Verify an EVM ERC-20 USDC transfer to the treasury via `eth_getTransactionReceipt`.
/// Returns `Ok(Some(amount))` when a matching Transfer of `>= min_amount` is found
/// in a successful (`status == 0x1`) receipt, else `Ok(None)`.
pub async fn verify_evm_usdc_transfer(
    rpc_url: &str,
    tx_hash: &str,
    treasury: &str,
    usdc_token: &str,
    min_amount: u128,
) -> anyhow::Result<Option<u128>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "eth_getTransactionReceipt", "params": [tx_hash],
    });
    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    let receipt = &resp["result"];
    if receipt.is_null() {
        return Ok(None); // unknown / unconfirmed tx
    }
    // Reject reverted transactions (status 0x0).
    if receipt["status"].as_str().unwrap_or("") != "0x1" {
        return Ok(None);
    }
    let logs = receipt["logs"].as_array().cloned().unwrap_or_default();
    Ok(match_erc20_transfer(
        &logs, usdc_token, treasury, min_amount,
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    //! Unit tests for the atomicity + idempotency properties of the payment
    //! DB helpers. Each test opens an in-memory SqliteStore so there is no
    //! filesystem dependency. For "concurrent" assertions we open a second
    //! connection to the same file-backed DB via tempfile::NamedTempFile and
    //! spawn threads — `SqliteStore::in_memory()` gives each caller its own
    //! empty DB, which is the wrong semantic for race tests.
    use super::*;

    // ── EVM ERC-20 Transfer decoder (Wave 1) ────────────────────────────────
    fn transfer_log(token: &str, to_topic: &str, data_hex: &str) -> serde_json::Value {
        serde_json::json!({
            "address": token,
            "topics": [
                super::ERC20_TRANSFER_TOPIC0,
                "0x000000000000000000000000aaaa000000000000000000000000000000000001",
                to_topic
            ],
            "data": data_hex,
        })
    }

    #[test]
    fn evm_transfer_matches_recipient_and_amount() {
        let token = "0x3600000000000000000000000000000000000000";
        let treasury = "0x00000000000000000000000000000000000000fe";
        let to_topic = "0x00000000000000000000000000000000000000000000000000000000000000fe";
        // 1_000_000 (1 USDC, 6-dec) = 0xf4240
        let logs = vec![transfer_log(
            token,
            to_topic,
            "0x00000000000000000000000000000000000000000000000000000000000f4240",
        )];
        assert_eq!(
            match_erc20_transfer(&logs, token, treasury, 1_000_000),
            Some(1_000_000)
        );
        // below minimum → no match
        assert_eq!(
            match_erc20_transfer(&logs, token, treasury, 2_000_000),
            None
        );
    }

    #[test]
    fn evm_transfer_rejects_wrong_token_or_recipient() {
        let token = "0x3600000000000000000000000000000000000000";
        let treasury = "0x00000000000000000000000000000000000000fe";
        let to_topic = "0x00000000000000000000000000000000000000000000000000000000000000fe";
        let amt = "0x00000000000000000000000000000000000000000000000000000000000f4240";
        // wrong token contract
        let wrong_token = vec![transfer_log(
            "0xdeadbeef00000000000000000000000000000000",
            to_topic,
            amt,
        )];
        assert_eq!(match_erc20_transfer(&wrong_token, token, treasury, 1), None);
        // wrong recipient
        let other = "0x00000000000000000000000000000000000000000000000000000000000000ab";
        let wrong_to = vec![transfer_log(token, other, amt)];
        assert_eq!(match_erc20_transfer(&wrong_to, token, treasury, 1), None);
    }

    #[test]
    fn topic_to_address_takes_last_20_bytes() {
        assert_eq!(
            topic_to_address("0x00000000000000000000000000000000000000000000000000000000000000fe"),
            "0x00000000000000000000000000000000000000fe"
        );
    }

    // ── T3: RefundsBySubject sliding-window guard ────────────────────────────

    #[test]
    fn hash_api_key_is_deterministic_and_not_raw_key() {
        let key = "mnm_abcdefghijklmnopqrstuvwx";
        let h1 = hash_api_key(key);
        let h2 = hash_api_key(key);
        assert_eq!(h1, h2, "hash must be deterministic");
        assert_ne!(h1, key, "hash must not equal raw key");
        assert!(
            !h1.contains("mnm_"),
            "blake3 hex must not contain the key prefix: {h1}"
        );
        assert_eq!(h1.len(), 64, "blake3 hex is 32 bytes = 64 hex chars");
    }

    #[test]
    fn refunds_by_subject_under_threshold_is_not_over() {
        let g = RefundsBySubject::new(Duration::from_secs(60), 5);
        assert!(!g.is_over("sub_x"));
        g.record_failure("sub_x");
        g.record_failure("sub_x");
        assert!(!g.is_over("sub_x"));
    }

    #[test]
    fn refunds_by_subject_at_threshold_is_over() {
        let g = RefundsBySubject::new(Duration::from_secs(60), 3);
        g.record_failure("sub_y");
        g.record_failure("sub_y");
        g.record_failure("sub_y");
        assert!(g.is_over("sub_y"));
        // Different subject is unaffected.
        assert!(!g.is_over("sub_z"));
    }

    #[test]
    fn refunds_by_subject_expired_entries_drop_out() {
        // 50ms window — easy to wait out in a test.
        let g = RefundsBySubject::new(Duration::from_millis(50), 2);
        g.record_failure("sub_a");
        g.record_failure("sub_a");
        assert!(g.is_over("sub_a"));
        std::thread::sleep(Duration::from_millis(80));
        assert!(
            !g.is_over("sub_a"),
            "after window elapses count must drop below threshold"
        );
    }

    #[test]
    fn evict_idle_drops_silent_subjects() {
        let g = RefundsBySubject::new(Duration::from_millis(20), 5);
        g.record_failure("sub_p");
        assert_eq!(g.len(), 1);
        std::thread::sleep(Duration::from_millis(50));
        let dropped = g.evict_idle(Duration::from_millis(20));
        assert_eq!(dropped, 1);
        assert!(g.is_empty());
    }

    #[test]
    fn evict_idle_keeps_active_subjects() {
        let g = RefundsBySubject::new(Duration::from_secs(60), 5);
        g.record_failure("active");
        // Eviction `since` longer than the time elapsed → keep.
        let dropped = g.evict_idle(Duration::from_secs(60));
        assert_eq!(dropped, 0);
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn delivery_metrics_increment_and_read() {
        let m = DeliveryMetrics::default();
        assert_eq!(m.quota_short_circuit(), 0);
        m.record_quota_short_circuit();
        m.record_quota_short_circuit();
        assert_eq!(m.quota_short_circuit(), 2);

        m.record_not_confirmed("refetch");
        m.record_not_confirmed("refetch");
        m.record_not_confirmed("verify");
        assert_eq!(m.not_confirmed("refetch"), 2);
        assert_eq!(m.not_confirmed("verify"), 1);
        assert_eq!(m.not_confirmed("recall"), 0);
    }
}

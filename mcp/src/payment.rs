//! Payment gating + payment-related database helpers.
//!
//! This file owns all payment concerns for the MCP server:
//!   - Payment-mode gating (`check_payment` and path selectors).
//!   - x402 nonce replay protection (`claim_x402_nonce`, `release_x402_nonce`).
//!   - P&L cost accounting (`record_attestation_cost`, `get_pnl_stats`).
//!   - Standalone `verify_usdc_transfer` over `&SolanaClient` (moved here in
//!     Task 8; the USDC-vs-recipient policy is payment-layer, not chain-layer).
//!   - EVM USDC x402 verifier (`verify_evm_usdc_transfer`) for Arc/Base.
//!   - Free daily anchor quota (`try_consume_free_anchor`,
//!     `refund_free_anchor`, `check_free_anchor`, `claim_free_anchor`):
//!     Google-linked accounts only, per-account + per-IP + global caps, a
//!     size limit, and no global refund after a chain write.
//!
//! Payment paths (Wave 4 — non-custodial; custodial balance/api-keys removed):
//!   - x402 — clients pay per-call via a USDC transfer on Solana OR an EVM
//!     chain (Arc/Base) and present the tx sig in `X-Payment: <json>` on the
//!     retry request. Verified on-chain; no operator-held float.
//!   - none — open access (development / self-hosted).

use anyhow::Context;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::SqliteStore;

use crate::paid_operation::{self, NewPaidOperation, PaidOperationState};
use crate::universal_paywall::{
    ExactAuthorization, OperationBinding, OperationScope, StoredQuote, UniversalPaywallClient,
    UniversalPaywallConfig,
};

// ── Payment schema init (Task 17 — moved from mnemonic-core) ─────────────────
//
// The four payment-specific tables (`api_keys`, `payment_events`, `x402_nonces`,
// `attestation_costs`) and their associated migration are payment concerns that
// must not leak into the public `mnemonic-core` crate API. They live here, and
// `init_payment_schema` is called once at MCP startup after the core schema is
// initialized.

/// Initialize payment-specific tables and run the `payment_events` UNIQUE index
/// dedup migration.
///
/// Called once at MCP startup (after `SqliteStore::open`) from `main.rs`. Safe
/// to call on every restart — all DDL uses `IF NOT EXISTS` and the dedup is
/// idempotent (deletes nothing on a clean DB, cleans up legacy duplicates on an
/// older one).
///
/// **Core is not aware of these tables.** A downstream consumer of `mnemonic-core`
/// from crates.io will not have these tables in their database unless they also
/// call this function.
pub fn init_payment_schema(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS api_keys (
            api_key TEXT PRIMARY KEY,
            owner_pubkey TEXT NOT NULL DEFAULT '',
            balance_micro_usdc INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            last_used_at TEXT
        );

        CREATE TABLE IF NOT EXISTS payment_events (
            event_id TEXT PRIMARY KEY,
            api_key TEXT NOT NULL,
            amount_micro_usdc INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            tx_sig TEXT,
            description TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_payment_events_key ON payment_events(api_key);

        CREATE TABLE IF NOT EXISTS x402_nonces (
            tx_sig TEXT PRIMARY KEY,
            used_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS attestation_costs (
            attestation_id TEXT PRIMARY KEY,
            storage_cost_micro_usdc INTEGER NOT NULL,
            sol_tx_fee_lamports INTEGER NOT NULL,
            sol_price_usdc REAL NOT NULL,
            earned_micro_usdc INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (attestation_id) REFERENCES attestations(attestation_id)
        );",
    )
    .context("creating payment tables")?;

    migrate_attestation_costs_to_micro_usdc(conn)?;

    // Add oauth_pubkey to api_keys if missing (migrated from
    // `migrate_owner_pubkey_columns` in mnemonic-core — Task 17).
    let needs_oauth_col = {
        let mut stmt = conn.prepare("PRAGMA table_info(api_keys)")?;
        let mut rows = stmt.query([])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == "oauth_pubkey" {
                found = true;
                break;
            }
        }
        !found
    };
    if needs_oauth_col {
        conn.execute("ALTER TABLE api_keys ADD COLUMN oauth_pubkey TEXT", [])
            .context("adding api_keys.oauth_pubkey")?;
    }

    // Dedup any pre-existing duplicate non-NULL `tx_sig` rows, then create
    // the partial UNIQUE index on `payment_events(tx_sig)`. Idempotent.
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         DELETE FROM payment_events
          WHERE tx_sig IS NOT NULL
            AND rowid NOT IN (
                SELECT MIN(rowid) FROM payment_events
                 WHERE tx_sig IS NOT NULL
                 GROUP BY tx_sig
            );
         CREATE UNIQUE INDEX IF NOT EXISTS uq_payment_events_tx_sig
             ON payment_events(tx_sig) WHERE tx_sig IS NOT NULL;
         COMMIT;",
    )
    .context("migrating payment_events UNIQUE(tx_sig) index")?;

    Ok(())
}

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

// ── x402 v2 wire types (M1 — x402-v2-conformance) ────────────────────────────
//
// Replaces the bespoke `X402Response` / `PaymentOption` pair with the shapes
// the x402 v2 spec defines. The old types are kept for reference (M3 removes
// them); this block is the canonical v2 representation.
//
// x402 v2 `PaymentRequired` body:
//   { x402Version: 2, accepts: [ PaymentRequirements, … ] }
//
// A conformant off-the-shelf x402 client can parse this without any custom
// code. The human approval URL is never a top-level field here; it lives in
// `extensions` inside the relevant `accepts[]` entry.

/// x402 v2 `PaymentRequired` response body.
///
/// Emitted on HTTP 402; the full JSON is also base64-encoded and sent in the
/// `PAYMENT-REQUIRED` response header (canonical for the REST path, per spec
/// transport-v2).
#[derive(Debug, Clone, Serialize)]
pub struct PaymentRequired {
    #[serde(rename = "x402Version")]
    pub x402_version: u8,
    pub accepts: Vec<PaymentRequirements>,
    /// The caller's free daily anchor quota, so an agent can see why it must
    /// pay. Absent when the quota does not apply. Not part of the x402 spec;
    /// carried as a non-conflicting top-level extension until M3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_anchors: Option<FreeAnchorStatus>,
}

/// One payment option inside `PaymentRequired.accepts[]`.
///
/// Field names are camelCase to match the x402 v2 wire format exactly.
#[derive(Debug, Clone, Serialize)]
pub struct PaymentRequirements {
    /// "exact" — the only registered EVM scheme. `stake` is not advertised
    /// until U3 maps it onto `batch-settlement`.
    pub scheme: String,
    /// CAIP-2 network identifier, e.g. `eip155:84532`.
    pub network: String,
    /// Amount in smallest units (micro-USDC), as a decimal string.
    /// x402 v2 field name — replaces the v1 `maxAmountRequired`.
    pub amount: String,
    /// ERC-20 token contract address.
    pub asset: String,
    /// Treasury recipient address.
    #[serde(rename = "payTo")]
    pub pay_to: String,
    /// Maximum seconds the payer has to complete the payment.
    #[serde(rename = "maxTimeoutSeconds")]
    pub max_timeout_seconds: u32,
    /// Resource descriptor: URI + human description of what is being paid for.
    pub resource: ResourceInfo,
    /// Optional extensions. Used to carry the human approval URL for
    /// browser-mediated signing without polluting the top-level shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

/// x402 v2 `resource` object — what the payer is paying for.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceInfo {
    /// Canonical URI of the resource (e.g. `https://api.mnemonik.xyz/sign`).
    pub uri: String,
    /// Human-readable description shown in approval UIs.
    pub description: String,
}

// ── Legacy v1 wire types (kept through M3) ────────────────────────────────────

/// Body returned with HTTP 402 to describe what payment is required.
/// Kept for the non-Universal-Paywall x402 Solana/EVM path until M3.
/// New code should use `PaymentRequired` instead.
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct X402Response {
    #[serde(rename = "x402Version")]
    pub x402_version: u8,
    pub accepts: Vec<PaymentOption>,
    /// The caller's free daily anchor quota, so an agent can see why it must
    /// pay. Absent when the quota does not apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_anchors: Option<FreeAnchorStatus>,
}

#[allow(dead_code)]
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

// `NeedUniversalPaywall` is materially larger than the other variants.
// Boxing it would churn every construction and match site of a variant that
// M3 removes outright when the bespoke rail goes away — see
// work/x402-v2-conformance/tasks/M3-remove-the-wrapper.md.
#[allow(clippy::large_enum_variant)]
pub enum PaymentGate {
    /// Payment verified (or not required). Wave 4 removed custodial balance
    /// mode, so there is no longer an operator-issued api_key to carry — this
    /// is a unit variant.
    Proceed,
    /// Return HTTP 402 with a conformant x402 v2 body.
    NeedPayment(PaymentRequired),
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
        return Some(normalize_x402_proof(p));
    }
    // Fallback: base64-encoded JSON (Coinbase CDK sends this)
    if let Ok(decoded) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw) {
        if let Ok(p) = serde_json::from_slice::<X402PaymentProof>(&decoded) {
            return Some(normalize_x402_proof(p));
        }
    }
    None
}

/// One spelling per payment, so a replay cannot pass the `x402_nonces`
/// check with a different spelling of the same transaction. An EVM tx hash
/// is hex and case-insensitive at the RPC: store it lowercase with `0x`. A
/// Solana signature is base58 (case matters) and is only trimmed.
fn normalize_x402_proof(mut proof: X402PaymentProof) -> X402PaymentProof {
    let sig = proof.tx_sig.trim();
    proof.tx_sig = if is_evm_network(&proof.network) {
        let hex = sig
            .strip_prefix("0x")
            .or_else(|| sig.strip_prefix("0X"))
            .unwrap_or(sig);
        format!("0x{}", hex.to_ascii_lowercase())
    } else {
        sig.to_string()
    };
    proof
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

/// The Universal Paywall config that may charge an anchored write, or
/// `None` when that rail is off.
///
/// A Universal Paywall config alone does not turn charging on: the rail is
/// an x402 rail, so it charges only when `PAYMENT_MODE=x402`. On a
/// `PAYMENT_MODE=none` deploy (or any other value) this returns `None`, and
/// the sign-callback anchors without a quote. The pre-execution gate in
/// `mcp_handler` uses the same helper, so an unknown `PAYMENT_MODE` still
/// reaches `check_payment` and fails closed there.
pub fn active_universal_paywall<'a>(
    payment_mode: &str,
    config: Option<&'a UniversalPaywallConfig>,
) -> Option<&'a UniversalPaywallConfig> {
    if payment_mode == "x402" {
        config
    } else {
        None
    }
}

/// Payment proof sent in the `X-Payment` header for the Universal Paywall rail.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct UniversalPaywallPaymentProof {
    pub scheme: String,
    pub payer_wallet: String,
    pub authorization: ExactAuthorization,
}

/// Decode Universal Paywall exact payment proof from `X-Payment` header.
pub fn extract_universal_paywall_proof(
    headers: &HeaderMap,
) -> Option<UniversalPaywallPaymentProof> {
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
    store: &std::sync::Mutex<SqliteStore>,
    quotes: &DashMap<String, StoredQuote>,
    payer_subject: &str,
    payer_wallet: &str,
    artifact_hash: &str,
    operation_id: Option<&str>,
) -> PaymentGate {
    let subject_hash = blake3::hash(payer_subject.as_bytes()).to_hex().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Retry path: the client already signed an EIP-3009 authorization.
    if let Some(proof) = extract_universal_paywall_proof(headers) {
        let op_id = match operation_id {
            Some(id) => id.to_string(),
            None => {
                return PaymentGate::Unauthorized("missing operation_id for payment retry".into())
            }
        };
        let operation = match store.lock() {
            Ok(store) => match paid_operation::get(store.conn(), &op_id) {
                Ok(Some(operation)) => operation,
                Ok(None) => {
                    return PaymentGate::Unauthorized("unknown or expired operation_id".into())
                }
                Err(error) => {
                    tracing::error!(operation_id = %op_id, error = %error, "read paid operation failed");
                    return PaymentGate::Unauthorized("payment state unavailable".into());
                }
            },
            Err(_) => return PaymentGate::Unauthorized("payment state unavailable".into()),
        };
        if operation.subject_hash != subject_hash || operation.artifact_hash != artifact_hash {
            return PaymentGate::Unauthorized("operation binding does not match request".into());
        }
        match paid_operation::settled_receipt(&operation) {
            Ok(Some(_)) => return PaymentGate::Proceed,
            Ok(None) => {}
            Err(_) => {
                return PaymentGate::Unauthorized(
                    "stored payment receipt requires reconciliation".into(),
                )
            }
        }

        let quote = match quotes.get(&op_id) {
            Some(q) => q.clone(),
            None => match client.get_quote_by_operation_id(&op_id).await {
                Ok(quote) => StoredQuote {
                    operation_id: op_id.clone(),
                    quote_id: quote.quote_id,
                    binding: quote.binding,
                    binding_digest: quote.binding_digest,
                    receipt: None,
                },
                Err(error) => {
                    tracing::warn!(operation_id = %op_id, error = %error, "recover exact quote failed");
                    return PaymentGate::Unauthorized("unknown or expired operation_id".into());
                }
            },
        };
        if quote.binding.artifact_hash != artifact_hash
            || quote.binding.payer_subject != payer_subject
            || quote.binding.operation_id != op_id
            || operation.binding_digest.as_deref() != Some(quote.binding_digest.as_str())
            || operation.quote_id.as_deref() != Some(quote.quote_id.as_str())
        {
            return PaymentGate::Unauthorized("operation binding does not match request".into());
        }
        if let Ok(store) = store.lock() {
            if let Err(error) = paid_operation::mark_payment_authorizing(store.conn(), &op_id, &now)
            {
                tracing::warn!(operation_id = %op_id, error = %error, "mark payment authorizing failed");
                return PaymentGate::Unauthorized("payment state conflict".into());
            }
        } else {
            return PaymentGate::Unauthorized("payment state unavailable".into());
        }
        let receipt = match client
            .settle_exact(&quote.binding, &quote.binding_digest, &proof.authorization)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(operation_id = %op_id, error = %e, "universal-paywall settle_exact failed");
                return PaymentGate::Unauthorized(format!("payment settlement failed: {e}"));
            }
        };
        let receipt_json = match serde_json::to_string(&receipt) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(operation_id = %op_id, error = %error, "serialize provider receipt failed");
                return PaymentGate::Unauthorized("payment receipt unavailable".into());
            }
        };
        match store.lock() {
            Ok(store) => {
                if let Err(error) = paid_operation::record_provider_receipt(
                    store.conn(),
                    &op_id,
                    &receipt_json,
                    &now,
                ) {
                    tracing::error!(operation_id = %op_id, error = %error, "persist provider receipt failed");
                    return PaymentGate::Unauthorized("payment state unavailable".into());
                }
            }
            Err(_) => return PaymentGate::Unauthorized("payment state unavailable".into()),
        }
        quotes
            .entry(op_id)
            .and_modify(|q| q.receipt = Some(receipt));
        return PaymentGate::Proceed;
    }

    // First call: create a quote and ask the client to pay.
    let operation_id = operation_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let operation = match store.lock() {
        Ok(store) => match paid_operation::create_or_get(
            store.conn(),
            NewPaidOperation {
                operation_id: &operation_id,
                subject_hash: &subject_hash,
                artifact_hash,
                created_at: &now,
            },
        ) {
            Ok(operation) => operation,
            Err(error) => return PaymentGate::Unauthorized(error.to_string()),
        },
        Err(_) => return PaymentGate::Unauthorized("payment state unavailable".into()),
    };
    match paid_operation::settled_receipt(&operation) {
        Ok(Some(_)) => return PaymentGate::Proceed,
        Ok(None) => {}
        Err(_) => {
            return PaymentGate::Unauthorized(
                "stored payment receipt requires reconciliation".into(),
            )
        }
    }

    // A browser may have settled while Mnemonic was restarting. Provider
    // status is durable evidence, so recover it before returning another 402.
    if operation.quote_id.is_some() {
        if let Ok(status) = client.payment_status(&operation_id).await {
            if status.status == "settled" {
                if status.operation_id != operation_id {
                    return PaymentGate::Unauthorized("provider status operation mismatch".into());
                }
                if let Some(receipt) = status.receipt {
                    let quote = match client.get_quote_by_operation_id(&operation_id).await {
                        Ok(q) => q,
                        Err(_) => {
                            return PaymentGate::Unauthorized(
                                "settled payment quote unavailable for verification".into(),
                            )
                        }
                    };
                    if quote.binding.operation_id != operation_id
                        || quote.binding.artifact_hash != artifact_hash
                        || quote.binding.payer_subject != payer_subject
                        || operation.binding_digest.as_deref()
                            != Some(quote.binding_digest.as_str())
                        || operation.quote_id.as_deref() != Some(quote.quote_id.as_str())
                        || crate::universal_paywall::validate_payment_receipt(
                            &receipt,
                            &quote.binding,
                            &quote.binding_digest,
                        )
                        .is_err()
                    {
                        return PaymentGate::Unauthorized(
                            "provider settled receipt binding mismatch".into(),
                        );
                    }
                    if let Ok(receipt_json) = serde_json::to_string(&receipt) {
                        if let Ok(store) = store.lock() {
                            if paid_operation::record_provider_receipt(
                                store.conn(),
                                &operation_id,
                                &receipt_json,
                                &now,
                            )
                            .is_ok()
                            {
                                return PaymentGate::Proceed;
                            }
                        }
                    }
                }
            }
        }
        if let Ok(quote) = client.get_quote_by_operation_id(&operation_id).await {
            if quote.binding.artifact_hash != artifact_hash
                || quote.binding.payer_subject != payer_subject
                || quote.binding.operation_id != operation_id
                || operation.binding_digest.as_deref() != Some(quote.binding_digest.as_str())
                || operation.quote_id.as_deref() != Some(quote.quote_id.as_str())
            {
                return PaymentGate::Unauthorized(
                    "operation binding does not match request".into(),
                );
            }
            let quote_expires_at = quote.binding.expires_at.clone();
            quotes.insert(
                operation_id.clone(),
                StoredQuote {
                    operation_id: operation_id.clone(),
                    quote_id: quote.quote_id.clone(),
                    binding: quote.binding.clone(),
                    binding_digest: quote.binding_digest.clone(),
                    receipt: None,
                },
            );
            return PaymentGate::NeedUniversalPaywall(UniversalPaywallPaymentRequired {
                operation_id: operation_id.clone(),
                quote_id: quote.quote_id.clone(),
                approval_url: client.approval_url(
                    &operation_id,
                    &quote.quote_id,
                    &quote_expires_at,
                ),
                scheme: "exact".into(),
                network: quote.binding.network.clone(),
                asset: quote.binding.asset.clone(),
                pay_to: quote.binding.pay_to.clone(),
                payer_wallet: quote.binding.payer_wallet.clone(),
                amount: quote.binding.amount.clone(),
                binding_digest: quote.binding_digest,
            });
        }
    }

    let mut nonce_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = format!("0x{}", hex::encode(nonce_bytes));
    let quote_now = chrono::Utc::now();
    let expires_at = (quote_now + chrono::TimeDelta::minutes(5)).to_rfc3339();

    let binding = OperationBinding {
        version: 1,
        operation_id: operation_id.clone(),
        payer_subject: payer_subject.to_string(),
        payer_wallet: payer_wallet.to_string(),
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

    match store.lock() {
        Ok(store) => {
            if let Err(error) = paid_operation::record_quote(
                store.conn(),
                &operation_id,
                &binding.payer_wallet,
                &quote.binding_digest,
                &quote.quote_id,
                &binding.expires_at,
                &now,
            ) {
                tracing::error!(operation_id = %operation_id, error = %error, "persist payment quote failed");
                return PaymentGate::Unauthorized("payment state unavailable".into());
            }
        }
        Err(_) => return PaymentGate::Unauthorized("payment state unavailable".into()),
    }

    let approval_url = client.approval_url(&operation_id, &quote.quote_id, &binding.expires_at);

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
        payer_wallet: payer_wallet.to_string(),
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

    // Replay fast path WITHOUT consuming: reject a nonce another call
    // already owns before the RPC round-trip. The atomic reservation is
    // `claim_x402_nonce`, which `mcp_handler` runs after this gate and
    // before the paid call; a failed call releases it
    // (`release_x402_nonce`), so the caller's USDC is not forfeit.
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

    // Do NOT mark the nonce here: `mcp_handler` reserves it atomically with
    // `claim_x402_nonce` right before the paid call.
    PaymentGate::Proceed
}

/// Read-only replay check for an x402 nonce. Returns `Ok(true)` if a row
/// already exists in `x402_nonces`, `Ok(false)` otherwise.
///
/// Used by `check_x402` to fail-fast on replay BEFORE the more expensive
/// `verify_usdc_transfer` Solana RPC. Note: a race window exists between
/// this read and the paid call; the atomic `claim_x402_nonce` in
/// `mcp_handler` closes it (one of two concurrent requests wins).
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

/// Reserve an x402 payment for ONE paid call, before the call runs.
/// `Ok(true)` when this call now owns the payment, `Ok(false)` when another
/// call already used (or is using) it.
///
/// The single `INSERT OR IGNORE` on the `x402_nonces.tx_sig` primary key is
/// atomic, so two concurrent requests with the same `X-Payment` can never
/// both proceed. (Before, the nonce was inserted only after the call
/// succeeded. Two concurrent calls both passed the read-only check, both
/// parked a paid bundle, and the loser's INSERT failure was only logged: one
/// payment bought two anchors.)
pub fn claim_x402_nonce(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<bool> {
    let now = chrono::Utc::now().to_rfc3339();
    let changed = store
        .conn()
        .execute(
            "INSERT OR IGNORE INTO x402_nonces (tx_sig, used_at) VALUES (?1, ?2)",
            params![tx_sig, now],
        )
        .context("claim x402 nonce")?;
    Ok(changed == 1)
}

/// Give back a payment reserved by [`claim_x402_nonce`] when the paid call
/// failed (for example a delivery that was not confirmed), so the caller can
/// retry with the same `X-Payment` header and its USDC is not lost.
pub fn release_x402_nonce(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<()> {
    store
        .conn()
        .execute("DELETE FROM x402_nonces WHERE tx_sig = ?1", params![tx_sig])
        .context("release x402 nonce")?;
    Ok(())
}

// ── Builder ──────────────────────────────────────────────────────────────────

/// Build a conformant x402 v2 `PaymentRequired` from a Universal Paywall quote.
///
/// The human approval URL is carried in `extensions.approval_url` (not at the
/// top level) so the body has no unrecognised top-level fields. An off-the-shelf
/// x402 client that doesn't know about `extensions` simply ignores it.
pub fn up_payment_required(
    up: &UniversalPaywallPaymentRequired,
    resource_uri: &str,
) -> PaymentRequired {
    let extensions = serde_json::json!({
        "approval_url": up.approval_url,
        "operation_id": up.operation_id,
        "quote_id": up.quote_id,
        "binding_digest": up.binding_digest,
        "payer_wallet": up.payer_wallet,
    });
    let req = PaymentRequirements {
        scheme: "exact".into(),
        network: up.network.clone(),
        amount: up.amount.clone(),
        asset: up.asset.clone(),
        pay_to: up.pay_to.clone(),
        max_timeout_seconds: 300,
        resource: ResourceInfo {
            uri: resource_uri.to_string(),
            description: "Mnemonic anchored memory write".into(),
        },
        extensions: Some(extensions),
    };
    PaymentRequired {
        x402_version: 2,
        accepts: vec![req],
        free_anchors: None,
    }
}

/// Build an x402 v2 conformant `PaymentRequired` body.
///
/// `accepts[]` carries one `exact` entry per configured network. Solana is
/// always present (using the `solana-mainnet` network id — a CAIP-2 SVM
/// namespace does not yet exist in the x402 spec; kept unchanged so existing
/// Solana clients remain unaffected). EVM entries use CAIP-2 ids from
/// `EvmPaymentConfig.caip2_network`.
pub fn x402_required(
    treasury: &str,
    usdc_mint: &str,
    cost: i64,
    description: &str,
    evm: Option<&EvmPaymentConfig>,
) -> PaymentRequired {
    let resource = ResourceInfo {
        uri: String::new(),
        description: description.to_string(),
    };
    let mut accepts = vec![PaymentRequirements {
        scheme: "exact".into(),
        network: "solana-mainnet".into(),
        amount: cost.to_string(),
        asset: usdc_mint.to_string(),
        pay_to: treasury.to_string(),
        max_timeout_seconds: 300,
        resource: resource.clone(),
        extensions: None,
    }];
    if let Some(evm) = evm {
        accepts.push(PaymentRequirements {
            scheme: "exact".into(),
            network: evm.caip2_network.clone(),
            amount: cost.to_string(),
            asset: evm.usdc_token.clone(),
            pay_to: evm.treasury.clone(),
            max_timeout_seconds: 300,
            resource: resource.clone(),
            extensions: None,
        });
    }
    PaymentRequired {
        x402_version: 2,
        accepts,
        free_anchors: None,
    }
}

// ── USDC transfer verification ───────────────────────────────────────────────

/// Verify that `tx_sig` transfers at least `min_amount` micro-USDC of `usdc_mint`
/// to `recipient`. Returns the actual amount transferred (>= min_amount) on
/// success, or `Ok(None)` if the transfer is absent / insufficient.
///
/// This is a payment concern and lives here (not in `mnemonic_core::solana`):
/// core provides the typed `get_token_balance_delta` primitive; the policy
/// (which token, which recipient, minimum amount) lives here (Task 20).
pub async fn verify_usdc_transfer(
    client: &SolanaClient,
    tx_sig: &str,
    recipient: &str,
    usdc_mint: &str,
    min_amount: u64,
) -> anyhow::Result<Option<u64>> {
    match client
        .get_token_balance_delta(tx_sig, recipient, usdc_mint)
        .await?
    {
        Some(delta) if delta >= min_amount => Ok(Some(delta)),
        _ => Ok(None),
    }
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

fn table_has_column(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Databases created before the move to ArDrive Turbo stored the storage cost
/// in lamports in the second column of `attestation_costs`. Rename that column
/// (found by position, so no old name is hardcoded) to
/// `storage_cost_micro_usdc` and convert each row in place with the SOL/USDC
/// rate recorded on that row. Rows are not re-inserted, so existing foreign-key
/// state is untouched. Idempotent.
fn migrate_attestation_costs_to_micro_usdc(conn: &Connection) -> anyhow::Result<()> {
    if table_has_column(conn, "attestation_costs", "storage_cost_micro_usdc")? {
        return Ok(());
    }
    let legacy: String = conn
        .query_row(
            "SELECT name FROM pragma_table_info('attestation_costs') WHERE cid = 1",
            [],
            |row| row.get(0),
        )
        .context("reading the legacy storage-cost column")?;
    anyhow::ensure!(
        legacy
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "unexpected attestation_costs column name"
    );
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(&format!(
        "ALTER TABLE attestation_costs RENAME COLUMN {legacy} TO storage_cost_micro_usdc;
         UPDATE attestation_costs
           SET storage_cost_micro_usdc =
             CAST(ROUND(storage_cost_micro_usdc * sol_price_usdc / 1000.0) AS INTEGER);"
    ))?;
    tx.commit()
        .context("migrating attestation_costs storage cost to micro-USDC")
}

/// Record actual server costs alongside each completed attestation.
pub fn record_attestation_cost(
    store: &SqliteStore,
    attestation_id: &str,
    storage_cost_micro_usdc: u64,
    sol_tx_fee_lamports: u64,
    sol_price_usdc: f64,
    earned_micro_usdc: i64,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    store.conn().execute(
        "INSERT OR IGNORE INTO attestation_costs
         (attestation_id, storage_cost_micro_usdc, sol_tx_fee_lamports, sol_price_usdc, earned_micro_usdc, created_at)
         VALUES (?,?,?,?,?,?)",
        params![
            attestation_id,
            storage_cost_micro_usdc as i64,
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
            COALESCE(SUM(sol_tx_fee_lamports), 0),
            COALESCE(SUM(storage_cost_micro_usdc + sol_tx_fee_lamports * sol_price_usdc / 1000.0), 0.0),
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
// *entry* of the anchored path in `mcp_handler` BEFORE any Arweave or
// Solana write. Increments on every delivery-not-confirmed demotion. When a
// caller crosses the threshold within the window the next `anchored`
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

// ── Free daily anchor quota ──────────────────────────────────────────────────
//
// A free `anchored` (on-chain anchored) write needs ALL of:
//   - a Google-linked identity: the signer's Ed25519 key has a row in
//     `google_identity_links` (`oauth::google::google_sub_for_pubkey`). The
//     per-account counter is keyed on the Google account, so every key linked
//     to one Google account shares one quota. A key with no Google link, or an
//     anonymous caller, gets no free anchor (paid x402 needs no Google link);
//   - the per-account counter below `per_account` (default 10 per UTC day);
//   - the per-IP counter below `per_ip`: the real client IP
//     (`client_ip.rs`, IPv6 grouped by /64) of the agent that asked for the
//     write;
//   - the global counter below `global`, the operator's daily fee budget;
//   - the anchored COSE_Sign1 bytes at most `max_bytes` long. A larger write
//     goes the paid path.
// The quota applies only where a payment would otherwise be required: HTTP
// with `PAYMENT_MODE=x402` (`free_quota_applies`). `PAYMENT_MODE=none` is
// already free and never touches the counters; stdio is never gated.
//
// Storage: one mcp-owned table, `free_anchor_usage(subject, day, n)`.
// Subjects: `blake3("free-anchor/google:" + google_sub)` for an account,
// `blake3_keyed(salt, "free-anchor/ip:" + ip)` for an IP (the salt lives in
// `free_anchor_secret`, so a leaked table does not reveal IPv4 addresses by
// brute force), and the reserved `*` for the global counter. Raw IPs and raw
// Google ids are never stored here. `day` is the UTC date `YYYY-MM-DD`, so
// each day starts from zero without a reset job.
//
// Single consumption: a free anchor is consumed once per anchored write, at
// anchor time, in `api::sign_callback_handler` (`claim_free_anchor`). The
// pre-parking x402 gate in `mcp_handler` only peeks
// (`check_free_anchor`). A write whose delivery is not confirmed gets its
// free anchor back (`FreeAnchorGrant` refunds on drop): all three counters
// before any chain write, the per-account and per-IP counters only after one
// (the operator really paid the Arweave/Solana fees, so the global budget
// stays spent).

/// Idempotent mcp-owned migration for the free daily anchor quota.
pub const FREE_ANCHOR_USAGE_MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS free_anchor_usage (
    subject TEXT NOT NULL,
    day TEXT NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (subject, day)
);
CREATE TABLE IF NOT EXISTS free_anchor_secret (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    salt BLOB NOT NULL
);";

/// Reserved `free_anchor_usage.subject` for the global (all-keys) counter.
const GLOBAL_FREE_ANCHOR_SUBJECT: &str = "*";

/// Default `MNEMONIC_FREE_ANCHOR_MAX_BYTES`: 16 KiB of COSE_Sign1 bytes. A
/// typical memory (about 1 KiB of text, 384-dimension embedding) makes a
/// COSE_Sign1 of about 1.7 KiB (see the `typical_artifact_size` test), so
/// 16 KiB leaves room for about 15 KiB of text and tags.
pub const DEFAULT_FREE_ANCHOR_MAX_BYTES: usize = 16 * 1024;

/// Upper bound of the COSE_Sign1 envelope around the canonical CBOR payload
/// (protected header, signer key id, 64-byte Ed25519 signature). The
/// pre-parking gate adds it to the unsigned payload size; the measured
/// overhead is 140 bytes (`typical_artifact_size` test).
pub const COSE_SIGN1_OVERHEAD_BYTES: usize = 256;

/// Daily free anchor limits. `0` in any counter field means no free anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeAnchorLimits {
    /// Free anchored writes per Google account per UTC day
    /// (`MNEMONIC_FREE_ANCHORS_PER_DAY`, default 10).
    pub per_account: u32,
    /// Free anchored writes per client IP per UTC day
    /// (`MNEMONIC_FREE_ANCHORS_PER_IP_PER_DAY`, default 20).
    pub per_ip: u32,
    /// Free anchored writes per UTC day across all accounts
    /// (`MNEMONIC_FREE_ANCHORS_GLOBAL_PER_DAY`, default 1000).
    pub global: u32,
    /// Largest COSE_Sign1 (bytes) a free anchor may carry
    /// (`MNEMONIC_FREE_ANCHOR_MAX_BYTES`, default 16 KiB).
    pub max_bytes: usize,
}

impl FreeAnchorLimits {
    /// No free anchors. Test fixtures use this so the paid-path tests keep
    /// seeing a 402 on the first anchored write.
    #[allow(dead_code)] // used by test fixtures; the bin compiles this module too.
    pub const fn disabled() -> Self {
        Self {
            per_account: 0,
            per_ip: 0,
            global: 0,
            max_bytes: DEFAULT_FREE_ANCHOR_MAX_BYTES,
        }
    }

    /// True when at least one free anchor can exist today.
    pub fn is_enabled(&self) -> bool {
        self.per_account > 0 && self.per_ip > 0 && self.global > 0 && self.max_bytes > 0
    }
}

/// Why a write gets no free anchor. The `reason` string goes into the
/// `free_anchors` block of `mnemonic_whoami` and of every 402 body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeAnchorDenied {
    /// The quota is off on this deploy (a limit is 0).
    Disabled,
    /// No authenticated caller.
    AuthenticationRequired,
    /// The signer's key is not linked to a Google account.
    GoogleAccountRequired,
    /// The anchored bytes are larger than `max_bytes`.
    TooLarge,
    /// The Google account used its free anchors for today.
    AccountQuotaUsed,
    /// The client IP used its free anchors for today.
    IpQuotaUsed,
    /// The operator's global daily budget is spent.
    GlobalQuotaUsed,
    /// A Universal Paywall quote already exists for this write: it stays on
    /// the paid path, so it is never both charged and free.
    PaidOperationInProgress,
}

impl FreeAnchorDenied {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Disabled => "free_quota_disabled",
            Self::AuthenticationRequired => "authentication_required",
            Self::GoogleAccountRequired => "google_account_required",
            Self::TooLarge => "too_large",
            Self::AccountQuotaUsed => "account_quota_used",
            Self::IpQuotaUsed => "ip_quota_used",
            Self::GlobalQuotaUsed => "global_quota_used",
            Self::PaidOperationInProgress => "paid_operation_in_progress",
        }
    }
}

/// How to become eligible after `google_account_required`.
pub const GOOGLE_LINK_HINT: &str = "Sign in with Google in the Mnemonic browser extension \
     and link this agent key (POST /oauth/google/link). Paid x402 writes need no Google account.";

/// The caller's free anchor quota, as shown by `mnemonic_whoami` and in the
/// 402 payment-required bodies.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FreeAnchorStatus {
    /// True when the caller can get a free anchor at all (Google-linked key,
    /// quota on). A spent quota keeps `eligible: true` with `remaining: 0`.
    pub eligible: bool,
    /// Why the last write (or this caller) gets no free anchor. Absent when
    /// a free anchor is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// How to become eligible. Present with `google_account_required`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_hint: Option<&'static str>,
    /// Free anchored writes per Google account per UTC day.
    pub per_day: u32,
    /// Free anchored writes per client IP per UTC day.
    pub per_ip_per_day: u32,
    /// Largest COSE_Sign1 (bytes) a free anchor may carry.
    pub max_bytes: usize,
    /// Free anchored writes left today for the caller's Google account
    /// (never more than the global or known per-IP remainder). Absent when
    /// the caller is not eligible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u32>,
    /// Free anchored writes left today for the caller's IP. Absent when the
    /// IP is not known (for example in `mnemonic_whoami` over a transport
    /// without one) or the caller is not eligible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_remaining: Option<u32>,
    /// Free anchored writes left today across all accounts. Absent when the
    /// caller is not eligible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_remaining: Option<u32>,
    /// Next UTC midnight (RFC 3339), when all counters start again.
    pub resets_at: String,
}

/// True when an anchored write on this deploy would otherwise be paid, so
/// the free daily quota applies. `PAYMENT_MODE=none` is free already; an
/// unknown mode fails closed in `check_payment` and gets no free anchors.
pub fn free_quota_applies(payment_mode: &str) -> bool {
    payment_mode == "x402"
}

/// Create the free quota tables and the IP-hash salt. Idempotent.
pub fn migrate_free_anchor_usage(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(FREE_ANCHOR_USAGE_MIGRATION_SQL)
        .context("create free_anchor_usage table")?;
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    conn.execute(
        "INSERT OR IGNORE INTO free_anchor_secret (id, salt) VALUES (1, ?1)",
        params![salt.to_vec()],
    )
    .context("create free anchor salt")?;
    Ok(())
}

/// UTC day key (`YYYY-MM-DD`) for `now`.
pub fn utc_day(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// Next UTC midnight after `now`, as RFC 3339 (`2026-09-28T00:00:00Z`).
pub fn next_utc_midnight(now: DateTime<Utc>) -> String {
    let next_day = now.date_naive() + chrono::Days::new(1);
    format!("{}T00:00:00Z", next_day.format("%Y-%m-%d"))
}

/// The counter subjects for one free anchor: the Google account and the
/// client IP, both hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeAnchorKeys {
    account: String,
    ip: String,
}

impl FreeAnchorKeys {
    /// Build keys from raw parts (unit tests and fixtures).
    pub fn from_parts(conn: &Connection, google_sub: &str, ip: IpAddr) -> anyhow::Result<Self> {
        Ok(Self {
            account: account_subject(google_sub),
            ip: ip_subject(conn, ip)?,
        })
    }
}

fn account_subject(google_sub: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"free-anchor/google:");
    hasher.update(google_sub.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn ip_subject(conn: &Connection, ip: IpAddr) -> anyhow::Result<String> {
    let salt: Vec<u8> = conn
        .query_row(
            "SELECT salt FROM free_anchor_secret WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .context("read free anchor salt")?;
    let key: [u8; 32] = salt
        .try_into()
        .map_err(|_| anyhow::anyhow!("free anchor salt has the wrong length"))?;
    let bucket = crate::client_ip::rate_limit_key(ip);
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(b"free-anchor/ip:");
    hasher.update(bucket.to_string().as_bytes());
    Ok(hasher.finalize().to_hex().to_string())
}

/// Resolve the counter keys for the signer `pubkey` asking from `client_ip`.
/// `Ok(None)` when the key is not linked to a Google account. An unknown IP
/// (in-process callers only) shares one bucket, so it never bypasses the
/// per-IP limit.
pub fn free_anchor_keys(
    conn: &Connection,
    pubkey: &str,
    client_ip: Option<IpAddr>,
) -> anyhow::Result<Option<FreeAnchorKeys>> {
    if pubkey.is_empty() {
        return Ok(None);
    }
    let Some(google_sub) = crate::oauth::google::google_sub_for_pubkey(conn, pubkey)? else {
        return Ok(None);
    };
    let ip = client_ip.unwrap_or(crate::client_ip::UNKNOWN_CLIENT);
    Ok(Some(FreeAnchorKeys::from_parts(conn, &google_sub, ip)?))
}

/// Atomically consume one free anchor for `keys` on `day`. Returns
/// `Ok(None)` when the per-account, per-IP and global counters all stayed
/// within their limits, or the first limit that was reached.
///
/// One `BEGIN IMMEDIATE` transaction with three conditional UPSERTs: each
/// increments only while `n < limit`. When one does not change a row the
/// transaction rolls back, so no increment leaks. Concurrent callers on any
/// number of connections can never push a counter past its limit: the write
/// lock serialises them and the `WHERE n < ?` guard runs inside it.
pub fn try_consume_free_anchor(
    conn: &Connection,
    keys: &FreeAnchorKeys,
    day: &str,
    limits: FreeAnchorLimits,
) -> anyhow::Result<Option<FreeAnchorDenied>> {
    if !limits.is_enabled() {
        return Ok(Some(FreeAnchorDenied::Disabled));
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin free anchor transaction")?;
    let bump = |counter: &str, limit: u32| -> anyhow::Result<bool> {
        let changed = tx
            .execute(
                "INSERT INTO free_anchor_usage (subject, day, n) VALUES (?1, ?2, 1) \
                 ON CONFLICT(subject, day) DO UPDATE SET n = n + 1 WHERE n < ?3",
                params![counter, day, i64::from(limit)],
            )
            .context("increment free anchor counter")?;
        Ok(changed == 1)
    };
    // Dropping `tx` without `commit` rolls every increment back.
    if !bump(&keys.account, limits.per_account)? {
        return Ok(Some(FreeAnchorDenied::AccountQuotaUsed));
    }
    if !bump(&keys.ip, limits.per_ip)? {
        return Ok(Some(FreeAnchorDenied::IpQuotaUsed));
    }
    if !bump(GLOBAL_FREE_ANCHOR_SUBJECT, limits.global)? {
        return Ok(Some(FreeAnchorDenied::GlobalQuotaUsed));
    }
    tx.commit().context("commit free anchor transaction")?;
    Ok(None)
}

/// Give back one free anchor consumed on `day`: decrement the per-account and
/// per-IP counters, and the global counter only when `refund_global` (no
/// chain write happened), never below 0. Use the day of the consumption, not
/// today, so a write that fails after midnight refunds the right day.
pub fn refund_free_anchor(
    conn: &Connection,
    keys: &FreeAnchorKeys,
    day: &str,
    refund_global: bool,
) -> anyhow::Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin free anchor refund")?;
    let mut counters = vec![keys.account.as_str(), keys.ip.as_str()];
    if refund_global {
        counters.push(GLOBAL_FREE_ANCHOR_SUBJECT);
    }
    for counter in counters {
        tx.execute(
            "UPDATE free_anchor_usage SET n = n - 1 WHERE subject = ?1 AND day = ?2 AND n > 0",
            params![counter, day],
        )
        .context("refund free anchor counter")?;
    }
    tx.commit().context("commit free anchor refund")?;
    Ok(())
}

/// Free anchors left on `day` for each counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeAnchorsRemaining {
    pub account: u32,
    pub ip: u32,
    pub global: u32,
}

impl FreeAnchorsRemaining {
    /// The first counter that is spent, if any.
    pub fn spent(&self) -> Option<FreeAnchorDenied> {
        if self.account == 0 {
            Some(FreeAnchorDenied::AccountQuotaUsed)
        } else if self.ip == 0 {
            Some(FreeAnchorDenied::IpQuotaUsed)
        } else if self.global == 0 {
            Some(FreeAnchorDenied::GlobalQuotaUsed)
        } else {
            None
        }
    }
}

fn counter_used(conn: &Connection, counter: &str, day: &str) -> anyhow::Result<u32> {
    let n: Option<i64> = conn
        .query_row(
            "SELECT n FROM free_anchor_usage WHERE subject = ?1 AND day = ?2",
            params![counter, day],
            |row| row.get(0),
        )
        .optional()
        .context("read free anchor counter")?;
    Ok(u32::try_from(n.unwrap_or(0).max(0)).unwrap_or(u32::MAX))
}

/// Free anchors left on `day` for `keys`.
pub fn free_anchors_remaining(
    conn: &Connection,
    keys: &FreeAnchorKeys,
    day: &str,
    limits: FreeAnchorLimits,
) -> anyhow::Result<FreeAnchorsRemaining> {
    Ok(FreeAnchorsRemaining {
        account: limits
            .per_account
            .saturating_sub(counter_used(conn, &keys.account, day)?),
        ip: limits
            .per_ip
            .saturating_sub(counter_used(conn, &keys.ip, day)?),
        global: limits
            .global
            .saturating_sub(counter_used(conn, GLOBAL_FREE_ANCHOR_SUBJECT, day)?),
    })
}

/// Peek (no consumption): can the signer `pubkey`, asking from `client_ip`,
/// take a free anchor for `anchored_bytes` bytes on `day`? `Ok(Ok(keys))`
/// when yes, `Ok(Err(reason))` when not. The pre-parking x402 gate uses this
/// to skip the 402.
pub fn check_free_anchor(
    conn: &Connection,
    pubkey: &str,
    client_ip: Option<IpAddr>,
    anchored_bytes: usize,
    day: &str,
    limits: FreeAnchorLimits,
) -> anyhow::Result<Result<FreeAnchorKeys, FreeAnchorDenied>> {
    if !limits.is_enabled() {
        return Ok(Err(FreeAnchorDenied::Disabled));
    }
    if pubkey.is_empty() {
        return Ok(Err(FreeAnchorDenied::AuthenticationRequired));
    }
    let Some(keys) = free_anchor_keys(conn, pubkey, client_ip)? else {
        return Ok(Err(FreeAnchorDenied::GoogleAccountRequired));
    };
    if anchored_bytes > limits.max_bytes {
        return Ok(Err(FreeAnchorDenied::TooLarge));
    }
    match free_anchors_remaining(conn, &keys, day, limits)?.spent() {
        Some(reason) => Ok(Err(reason)),
        None => Ok(Ok(keys)),
    }
}

/// Build the `free_anchors` block for the caller `pubkey` (asking from
/// `client_ip`, when known) at `now`. `denied` names why the current write
/// gets no free anchor, when the caller already knows (for example
/// `too_large`); otherwise the reason comes from the counters.
pub fn free_anchor_status(
    conn: &Connection,
    pubkey: Option<&str>,
    client_ip: Option<IpAddr>,
    now: DateTime<Utc>,
    limits: FreeAnchorLimits,
    denied: Option<FreeAnchorDenied>,
) -> anyhow::Result<FreeAnchorStatus> {
    let mut status = FreeAnchorStatus {
        eligible: false,
        reason: None,
        link_hint: None,
        per_day: limits.per_account,
        per_ip_per_day: limits.per_ip,
        max_bytes: limits.max_bytes,
        remaining: None,
        ip_remaining: None,
        global_remaining: None,
        resets_at: next_utc_midnight(now),
    };
    let keys = match pubkey.filter(|p| !p.is_empty()) {
        None => {
            status.reason = Some(FreeAnchorDenied::AuthenticationRequired.reason());
            return Ok(status);
        }
        Some(pubkey) => free_anchor_keys(conn, pubkey, client_ip)?,
    };
    let Some(keys) = keys else {
        status.reason = Some(FreeAnchorDenied::GoogleAccountRequired.reason());
        status.link_hint = Some(GOOGLE_LINK_HINT);
        return Ok(status);
    };
    if !limits.is_enabled() {
        status.reason = Some(FreeAnchorDenied::Disabled.reason());
        return Ok(status);
    }
    let left = free_anchors_remaining(conn, &keys, &utc_day(now), limits)?;
    status.eligible = true;
    status.global_remaining = Some(left.global);
    // No account anchor is usable once the global (or known IP) cap is spent.
    let mut usable = left.account.min(left.global);
    if client_ip.is_some() {
        status.ip_remaining = Some(left.ip);
        usable = usable.min(left.ip);
    }
    status.remaining = Some(usable);
    let spent = if client_ip.is_some() {
        left.spent()
    } else {
        FreeAnchorsRemaining { ip: 1, ..left }.spent()
    };
    status.reason = denied.or(spent).map(FreeAnchorDenied::reason);
    Ok(status)
}

/// One free anchor consumed for an anchored write in progress.
///
/// Dropping the grant without calling [`FreeAnchorGrant::keep`] refunds the
/// anchor. The sign-callback holds it across the anchoring steps, so every
/// early return (upload failure, delivery not confirmed, replayed bundle,
/// cancelled request) gives the anchor back without a refund call at each
/// return site. After [`FreeAnchorGrant::mark_chain_write`] the refund skips
/// the global counter: the operator already paid the chain fees. `Drop`
/// locks the store synchronously: the caller must not hold the store lock
/// when the grant goes out of scope.
#[must_use = "dropping a FreeAnchorGrant refunds the free anchor"]
pub struct FreeAnchorGrant<'a> {
    store: &'a std::sync::Mutex<SqliteStore>,
    keys: FreeAnchorKeys,
    day: String,
    kept: bool,
    chain_written: bool,
}

impl FreeAnchorGrant<'_> {
    /// The anchored write is confirmed: keep the consumption.
    pub fn keep(mut self) {
        self.kept = true;
    }

    /// A chain write (Arweave upload) happened: from now on a refund gives
    /// back the per-account and per-IP anchors but not the global one.
    pub fn mark_chain_write(&mut self) {
        self.chain_written = true;
    }
}

impl Drop for FreeAnchorGrant<'_> {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        match self.store.lock() {
            Ok(store) => {
                if let Err(error) =
                    refund_free_anchor(store.conn(), &self.keys, &self.day, !self.chain_written)
                {
                    tracing::error!(day = %self.day, error = %error, "free anchor refund failed");
                }
            }
            Err(_) => tracing::error!(day = %self.day, "free anchor refund: store mutex poisoned"),
        }
    }
}

/// Result of [`claim_free_anchor`].
pub enum FreeAnchorClaim<'a> {
    /// One free anchor is consumed; drop the grant to refund it.
    Granted(FreeAnchorGrant<'a>),
    /// No free anchor for this write, and why.
    Denied(FreeAnchorDenied),
}

/// Consume one free anchor today for the signer `pubkey`, whose write was
/// requested from `client_ip` and anchors `anchored_bytes` bytes. Returns a
/// refund-on-drop grant, or the reason there is none.
///
/// `paid_operation_id` names a Universal Paywall operation. When that
/// operation already has a quote (the payer may be mid-payment, or has
/// paid), the write stays on the paid path, so one write is never both
/// charged and counted as free.
///
/// The store lock is taken and released inside; no `.await` runs under it.
pub fn claim_free_anchor<'a>(
    store: &'a std::sync::Mutex<SqliteStore>,
    pubkey: &str,
    client_ip: Option<IpAddr>,
    anchored_bytes: usize,
    paid_operation_id: Option<&str>,
    limits: FreeAnchorLimits,
) -> anyhow::Result<FreeAnchorClaim<'a>> {
    if !limits.is_enabled() {
        return Ok(FreeAnchorClaim::Denied(FreeAnchorDenied::Disabled));
    }
    let day = utc_day(Utc::now());
    let guard = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store mutex poisoned"))?;
    if let Some(operation_id) = paid_operation_id {
        if let Some(operation) = paid_operation::get(guard.conn(), operation_id)? {
            if operation.state != PaidOperationState::AwaitingSignature {
                return Ok(FreeAnchorClaim::Denied(
                    FreeAnchorDenied::PaidOperationInProgress,
                ));
            }
        }
    }
    let keys = match check_free_anchor(
        guard.conn(),
        pubkey,
        client_ip,
        anchored_bytes,
        &day,
        limits,
    )? {
        Ok(keys) => keys,
        Err(reason) => return Ok(FreeAnchorClaim::Denied(reason)),
    };
    if let Some(reason) = try_consume_free_anchor(guard.conn(), &keys, &day, limits)? {
        return Ok(FreeAnchorClaim::Denied(reason));
    }
    drop(guard);
    Ok(FreeAnchorClaim::Granted(FreeAnchorGrant {
        store,
        keys,
        day,
        kept: false,
        chain_written: false,
    }))
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
    /// CAIP-2 network identifier, e.g. `eip155:84532`. Used in the conformant
    /// x402 v2 `PaymentRequired` body. Defaults to `"eip155:1"` when not set
    /// (EVM_CHAIN_ID env var absent).
    pub caip2_network: String,
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

    // ── Universal Paywall activation follows PAYMENT_MODE ───────────────────
    fn up_config() -> UniversalPaywallConfig {
        UniversalPaywallConfig {
            url: "http://localhost:0".into(),
            api_key: "k".into(),
            network: "eip155:84532".into(),
            asset: "0x0000000000000000000000000000000000000001".into(),
            pay_to: "0x0000000000000000000000000000000000000002".into(),
            payer_wallet: String::new(),
            approval_url_base: String::new(),
        }
    }

    #[test]
    fn universal_paywall_charges_only_in_x402_mode() {
        let cfg = up_config();
        assert!(active_universal_paywall("x402", Some(&cfg)).is_some());
        // A configured Universal Paywall must not charge on a free deploy,
        // nor on an unknown mode (that one fails closed in `check_payment`).
        assert!(active_universal_paywall("none", Some(&cfg)).is_none());
        assert!(active_universal_paywall("balance", Some(&cfg)).is_none());
        assert!(active_universal_paywall("x402", None).is_none());
    }

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

    // ── Free daily anchor quota ─────────────────────────────────────────────

    const DAY: &str = "2026-09-27";

    #[test]
    fn attestation_costs_migrate_lamports_to_micro_usdc_once() {
        let conn = Connection::open_in_memory().unwrap();
        // Pre-Turbo layout: the second column held the storage cost in lamports.
        // `attestations` exists in every real database (foreign-key target).
        conn.execute_batch(
            "CREATE TABLE attestations (attestation_id TEXT PRIMARY KEY);
             INSERT INTO attestations VALUES ('a1');
             CREATE TABLE attestation_costs (
                attestation_id TEXT PRIMARY KEY,
                storage_cost_lamports_legacy INTEGER NOT NULL,
                sol_tx_fee_lamports INTEGER NOT NULL,
                sol_price_usdc REAL NOT NULL,
                earned_micro_usdc INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (attestation_id) REFERENCES attestations(attestation_id)
             );
             INSERT INTO attestation_costs VALUES ('a1', 2000000, 5000, 150.0, 400000, '2026-09-01T00:00:00Z');",
        )
        .unwrap();
        migrate_attestation_costs_to_micro_usdc(&conn).unwrap();
        // 2_000_000 lamports at $150/SOL = 300_000 micro-USDC.
        let row: (i64, i64, f64, i64, String) = conn
            .query_row(
                "SELECT storage_cost_micro_usdc, sol_tx_fee_lamports, sol_price_usdc, earned_micro_usdc, created_at
                 FROM attestation_costs WHERE attestation_id = 'a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (300_000, 5000, 150.0, 400_000, "2026-09-01T00:00:00Z".into())
        );
        // Idempotent: a second run leaves the converted value alone.
        migrate_attestation_costs_to_micro_usdc(&conn).unwrap();
        let again: i64 = conn
            .query_row(
                "SELECT storage_cost_micro_usdc FROM attestation_costs",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(again, 300_000);
        assert!(
            !table_has_column(&conn, "attestation_costs", "storage_cost_lamports_legacy").unwrap()
        );
    }

    fn quota_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate_free_anchor_usage(&conn).unwrap();
        // Idempotent, and the salt survives a second migration.
        let salt: Vec<u8> = conn
            .query_row("SELECT salt FROM free_anchor_secret", [], |r| r.get(0))
            .unwrap();
        migrate_free_anchor_usage(&conn).unwrap();
        let again: Vec<u8> = conn
            .query_row("SELECT salt FROM free_anchor_secret", [], |r| r.get(0))
            .unwrap();
        assert_eq!(salt, again);
        conn
    }

    fn limits(per_account: u32, per_ip: u32, global: u32) -> FreeAnchorLimits {
        FreeAnchorLimits {
            per_account,
            per_ip,
            global,
            max_bytes: DEFAULT_FREE_ANCHOR_MAX_BYTES,
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn keys(conn: &Connection, account: &str, addr: &str) -> FreeAnchorKeys {
        FreeAnchorKeys::from_parts(conn, account, ip(addr)).unwrap()
    }

    fn left(
        conn: &Connection,
        k: &FreeAnchorKeys,
        day: &str,
        l: FreeAnchorLimits,
    ) -> (u32, u32, u32) {
        let r = free_anchors_remaining(conn, k, day, l).unwrap();
        (r.account, r.ip, r.global)
    }

    /// Link `pubkey` to Google account `google_sub` in `conn`.
    fn link(conn: &Connection, google_sub: &str, pubkey: &str, linked_at: i64) {
        crate::oauth::google::migrate_google_identity_links(conn).unwrap();
        conn.execute(
            "INSERT INTO google_identity_links (google_sub, pubkey_base58, linked_at) VALUES (?1, ?2, ?3)",
            params![google_sub, pubkey, linked_at],
        )
        .unwrap();
    }

    #[test]
    fn free_quota_applies_only_to_x402() {
        assert!(free_quota_applies("x402"));
        assert!(!free_quota_applies("none"));
        assert!(!free_quota_applies("balance"));
        assert!(!free_quota_applies(""));
    }

    #[test]
    fn ten_free_anchors_per_account_then_payment_and_a_second_account_has_its_own() {
        let conn = quota_conn();
        let l = limits(10, 100, 1000);
        let a = keys(&conn, "google-a", "198.51.100.1");
        for i in 0..10 {
            assert_eq!(
                try_consume_free_anchor(&conn, &a, DAY, l).unwrap(),
                None,
                "anchor {i}"
            );
        }
        assert_eq!(
            try_consume_free_anchor(&conn, &a, DAY, l).unwrap(),
            Some(FreeAnchorDenied::AccountQuotaUsed)
        );
        assert_eq!(left(&conn, &a, DAY, l), (0, 90, 990));
        let b = keys(&conn, "google-b", "198.51.100.1");
        assert_eq!(try_consume_free_anchor(&conn, &b, DAY, l).unwrap(), None);
        assert_eq!(left(&conn, &b, DAY, l), (9, 89, 989));
    }

    #[test]
    fn per_ip_share_blocks_many_accounts_from_one_ip_and_rolls_back() {
        let conn = quota_conn();
        let l = limits(10, 3, 1000);
        for n in 0..3 {
            let k = keys(&conn, &format!("acct-{n}"), "2001:db8:1:2::1");
            assert_eq!(try_consume_free_anchor(&conn, &k, DAY, l).unwrap(), None);
        }
        // Another address in the same IPv6 /64 is the same client.
        let fourth = keys(&conn, "acct-4", "2001:db8:1:2:ffff::9");
        assert_eq!(
            try_consume_free_anchor(&conn, &fourth, DAY, l).unwrap(),
            Some(FreeAnchorDenied::IpQuotaUsed)
        );
        // The rolled-back transaction left the account and global counters alone.
        assert_eq!(left(&conn, &fourth, DAY, l), (10, 0, 997));
        // A different IP still works for the same account.
        let elsewhere = keys(&conn, "acct-4", "203.0.113.9");
        assert_eq!(
            try_consume_free_anchor(&conn, &elsewhere, DAY, l).unwrap(),
            None
        );
    }

    #[test]
    fn global_cap_blocks_new_accounts_and_rolls_back_the_other_increments() {
        let conn = quota_conn();
        let l = limits(10, 100, 3);
        for n in 0..3 {
            let k = keys(&conn, &format!("g{n}"), &format!("198.51.100.{n}"));
            assert_eq!(try_consume_free_anchor(&conn, &k, DAY, l).unwrap(), None);
        }
        let k4 = keys(&conn, "g4", "198.51.100.44");
        assert_eq!(
            try_consume_free_anchor(&conn, &k4, DAY, l).unwrap(),
            Some(FreeAnchorDenied::GlobalQuotaUsed)
        );
        assert_eq!(left(&conn, &k4, DAY, l), (10, 100, 0));
    }

    #[test]
    fn zero_limits_disable_free_anchors() {
        let conn = quota_conn();
        let k = keys(&conn, "g", "198.51.100.1");
        for l in [limits(0, 10, 10), limits(10, 0, 10), limits(10, 10, 0)] {
            assert_eq!(
                try_consume_free_anchor(&conn, &k, DAY, l).unwrap(),
                Some(FreeAnchorDenied::Disabled)
            );
        }
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM free_anchor_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn refund_after_a_chain_write_keeps_the_global_counter_spent() {
        let conn = quota_conn();
        let l = limits(2, 5, 10);
        let k = keys(&conn, "g", "198.51.100.1");
        // Refund with nothing consumed stays at the limit.
        refund_free_anchor(&conn, &k, DAY, true).unwrap();
        assert_eq!(left(&conn, &k, DAY, l), (2, 5, 10));
        assert_eq!(try_consume_free_anchor(&conn, &k, DAY, l).unwrap(), None);
        assert_eq!(try_consume_free_anchor(&conn, &k, DAY, l).unwrap(), None);
        assert_eq!(left(&conn, &k, DAY, l), (0, 3, 8));
        // Failure before any chain write: all three counters come back.
        refund_free_anchor(&conn, &k, DAY, true).unwrap();
        assert_eq!(left(&conn, &k, DAY, l), (1, 4, 9));
        // Failure after the chain write: the operator paid, global stays spent.
        refund_free_anchor(&conn, &k, DAY, false).unwrap();
        assert_eq!(left(&conn, &k, DAY, l), (2, 5, 9));
        for _ in 0..5 {
            refund_free_anchor(&conn, &k, DAY, true).unwrap();
        }
        assert_eq!(left(&conn, &k, DAY, l), (2, 5, 10));
    }

    #[test]
    fn day_rollover_resets_the_quota() {
        let conn = quota_conn();
        let l = limits(10, 100, 1000);
        let k = keys(&conn, "g", "198.51.100.1");
        for _ in 0..10 {
            assert_eq!(
                try_consume_free_anchor(&conn, &k, "2026-09-27", l).unwrap(),
                None
            );
        }
        assert!(try_consume_free_anchor(&conn, &k, "2026-09-27", l)
            .unwrap()
            .is_some());
        assert_eq!(
            try_consume_free_anchor(&conn, &k, "2026-09-28", l).unwrap(),
            None
        );
        // A refund for yesterday's write lands on yesterday's counter.
        refund_free_anchor(&conn, &k, "2026-09-27", true).unwrap();
        assert_eq!(left(&conn, &k, "2026-09-27", l), (1, 91, 991));
        assert_eq!(left(&conn, &k, "2026-09-28", l), (9, 99, 999));
    }

    #[test]
    fn utc_day_and_reset_time() {
        let late = DateTime::parse_from_rfc3339("2026-09-27T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(utc_day(late), "2026-09-27");
        assert_eq!(next_utc_midnight(late), "2026-09-28T00:00:00Z");
        let new_year = DateTime::parse_from_rfc3339("2026-12-31T08:00:00+05:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(utc_day(new_year), "2026-12-31");
        assert_eq!(next_utc_midnight(new_year), "2027-01-01T00:00:00Z");
    }

    #[test]
    fn stored_subjects_never_contain_raw_ips_or_google_ids() {
        let conn = quota_conn();
        let k = keys(&conn, "108234567890", "198.51.100.77");
        assert_eq!(
            try_consume_free_anchor(&conn, &k, DAY, limits(1, 1, 1)).unwrap(),
            None
        );
        let subjects: Vec<String> = conn
            .prepare("SELECT subject FROM free_anchor_usage")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(subjects.len(), 3);
        for s in &subjects {
            assert!(
                !s.contains("198.51.100.77") && !s.contains("108234567890"),
                "{s}"
            );
        }
        // The IP hash is keyed: plain blake3 of the address does not match.
        let plain = blake3::hash(b"free-anchor/ip:198.51.100.77")
            .to_hex()
            .to_string();
        assert!(!subjects.contains(&plain));
    }

    /// Many threads, each with its OWN connection to one file-backed DB, race
    /// `try_consume_free_anchor`. The grants must never exceed any limit.
    #[test]
    fn concurrent_consumers_never_exceed_the_limits() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            migrate_free_anchor_usage(&conn).unwrap();
        }
        const THREADS: usize = 16;
        const ATTEMPTS: usize = 10;
        // 4 accounts x 10 = 40 possible, 4 IPs x 9 = 36, but the global cap is 30.
        let l = limits(10, 9, 30);
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let conn = Connection::open(&path).unwrap();
                conn.execute_batch("PRAGMA busy_timeout=10000;").unwrap();
                std::thread::spawn(move || {
                    let k = keys(
                        &conn,
                        &format!("acct-{}", t % 4),
                        &format!("198.51.100.{}", t % 4),
                    );
                    (0..ATTEMPTS)
                        .filter(|_| {
                            try_consume_free_anchor(&conn, &k, DAY, l)
                                .unwrap()
                                .is_none()
                        })
                        .count()
                })
            })
            .collect();
        let granted: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(granted, 30, "grants must stop at the global cap");

        // Same race on one account + one IP: exactly `per_account` grants.
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let conn = Connection::open(&path).unwrap();
                conn.execute_batch("PRAGMA busy_timeout=10000;").unwrap();
                std::thread::spawn(move || {
                    let k = keys(&conn, "solo", "203.0.113.1");
                    (0..ATTEMPTS)
                        .filter(|_| {
                            try_consume_free_anchor(&conn, &k, "2026-09-28", limits(25, 100, 1000))
                                .unwrap()
                                .is_none()
                        })
                        .count()
                })
            })
            .collect();
        let granted: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(granted, 25, "grants must stop at the per-account limit");
    }

    #[test]
    fn google_link_decides_eligibility_and_the_earliest_link_wins() {
        let conn = quota_conn();
        let l = limits(10, 20, 1000);
        // No link table at all (Google OAuth off): not linked, not an error.
        assert_eq!(
            check_free_anchor(&conn, "pk-1", Some(ip("198.51.100.1")), 100, DAY, l).unwrap(),
            Err(FreeAnchorDenied::GoogleAccountRequired)
        );
        assert_eq!(
            check_free_anchor(&conn, "", None, 100, DAY, l).unwrap(),
            Err(FreeAnchorDenied::AuthenticationRequired)
        );
        link(&conn, "google-late", "pk-1", 200);
        conn.execute(
            "INSERT INTO google_identity_links (google_sub, pubkey_base58, linked_at) VALUES ('google-early', 'pk-1', 100)",
            [],
        )
        .unwrap();
        assert_eq!(
            crate::oauth::google::google_sub_for_pubkey(&conn, "pk-1")
                .unwrap()
                .as_deref(),
            Some("google-early")
        );
        let k = check_free_anchor(&conn, "pk-1", Some(ip("198.51.100.1")), 100, DAY, l)
            .unwrap()
            .expect("eligible");
        assert_eq!(k, keys(&conn, "google-early", "198.51.100.1"));
        // Too large for the free tier.
        assert_eq!(
            check_free_anchor(
                &conn,
                "pk-1",
                None,
                DEFAULT_FREE_ANCHOR_MAX_BYTES + 1,
                DAY,
                l
            )
            .unwrap(),
            Err(FreeAnchorDenied::TooLarge)
        );
        assert!(
            check_free_anchor(&conn, "pk-1", None, DEFAULT_FREE_ANCHOR_MAX_BYTES, DAY, l)
                .unwrap()
                .is_ok()
        );
    }

    #[test]
    fn keys_linked_to_one_google_account_share_one_quota() {
        let conn = quota_conn();
        let l = limits(2, 100, 1000);
        // Two agent keys whose Google account is the same resolve to one
        // account counter (the counter is keyed on the Google id).
        let a = keys(&conn, "google-shared", "198.51.100.1");
        let b = keys(&conn, "google-shared", "203.0.113.2");
        assert_eq!(try_consume_free_anchor(&conn, &a, DAY, l).unwrap(), None);
        assert_eq!(try_consume_free_anchor(&conn, &b, DAY, l).unwrap(), None);
        assert_eq!(
            try_consume_free_anchor(&conn, &a, DAY, l).unwrap(),
            Some(FreeAnchorDenied::AccountQuotaUsed)
        );
    }

    #[test]
    fn free_anchor_status_for_linked_unlinked_and_anonymous_callers() {
        let conn = quota_conn();
        let now = DateTime::parse_from_rfc3339("2026-09-27T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let l = limits(10, 20, 5);
        link(&conn, "google-k", "pk-k", 1);
        let k = keys(&conn, "google-k", "198.51.100.1");
        for _ in 0..3 {
            assert_eq!(try_consume_free_anchor(&conn, &k, DAY, l).unwrap(), None);
        }
        let status =
            free_anchor_status(&conn, Some("pk-k"), Some(ip("198.51.100.1")), now, l, None)
                .unwrap();
        assert!(status.eligible);
        assert_eq!(status.reason, None);
        assert_eq!(status.per_day, 10);
        assert_eq!(status.per_ip_per_day, 20);
        assert_eq!(status.max_bytes, DEFAULT_FREE_ANCHOR_MAX_BYTES);
        // 7 left for the account, 17 for the IP, but only 2 left globally.
        assert_eq!(status.remaining, Some(2));
        assert_eq!(status.ip_remaining, Some(17));
        assert_eq!(status.global_remaining, Some(2));
        assert_eq!(status.resets_at, "2026-09-28T00:00:00Z");
        // Unknown IP (whoami): no ip_remaining.
        let status = free_anchor_status(&conn, Some("pk-k"), None, now, l, None).unwrap();
        assert_eq!(status.ip_remaining, None);
        assert_eq!(status.remaining, Some(2));
        // A caller-supplied reason wins for an eligible caller.
        let status = free_anchor_status(
            &conn,
            Some("pk-k"),
            None,
            now,
            l,
            Some(FreeAnchorDenied::TooLarge),
        )
        .unwrap();
        assert_eq!(status.reason, Some("too_large"));

        let unlinked = free_anchor_status(&conn, Some("pk-other"), None, now, l, None).unwrap();
        let json = serde_json::to_value(&unlinked).unwrap();
        assert_eq!(json["eligible"], false);
        assert_eq!(json["reason"], "google_account_required");
        assert!(json["link_hint"]
            .as_str()
            .unwrap()
            .contains("/oauth/google/link"));
        assert!(json.get("remaining").is_none());

        let anon = free_anchor_status(&conn, None, None, now, l, None).unwrap();
        let json = serde_json::to_value(&anon).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "eligible": false,
                "reason": "authentication_required",
                "per_day": 10,
                "per_ip_per_day": 20,
                "max_bytes": DEFAULT_FREE_ANCHOR_MAX_BYTES,
                "resets_at": "2026-09-28T00:00:00Z",
            })
        );
    }

    fn quota_store(pubkey: &str, google_sub: &str) -> std::sync::Mutex<SqliteStore> {
        let store = SqliteStore::in_memory().unwrap();
        migrate_free_anchor_usage(store.conn()).unwrap();
        paid_operation::migrate_paid_operations(store.conn()).unwrap();
        link(store.conn(), google_sub, pubkey, 1);
        std::sync::Mutex::new(store)
    }

    fn store_left(store: &std::sync::Mutex<SqliteStore>, google_sub: &str) -> (u32, u32, u32) {
        let guard = store.lock().unwrap();
        let k =
            FreeAnchorKeys::from_parts(guard.conn(), google_sub, crate::client_ip::UNKNOWN_CLIENT)
                .unwrap();
        left(guard.conn(), &k, &utc_day(Utc::now()), limits(2, 10, 100))
    }

    fn claim<'a>(
        store: &'a std::sync::Mutex<SqliteStore>,
        pubkey: &str,
        op: Option<&str>,
        l: FreeAnchorLimits,
    ) -> FreeAnchorClaim<'a> {
        claim_free_anchor(store, pubkey, None, 500, op, l).unwrap()
    }

    fn granted(c: FreeAnchorClaim<'_>) -> FreeAnchorGrant<'_> {
        match c {
            FreeAnchorClaim::Granted(g) => g,
            FreeAnchorClaim::Denied(reason) => panic!("expected a grant, got {reason:?}"),
        }
    }

    fn denied(c: FreeAnchorClaim<'_>) -> FreeAnchorDenied {
        match c {
            FreeAnchorClaim::Granted(_) => panic!("expected a denial"),
            FreeAnchorClaim::Denied(reason) => reason,
        }
    }

    #[test]
    fn dropped_grant_refunds_and_kept_grant_stays_consumed() {
        let store = quota_store("pk", "g");
        let l = limits(2, 10, 100);
        let grant = granted(claim(&store, "pk", None, l));
        assert_eq!(store_left(&store, "g"), (1, 9, 99));
        // Delivery not confirmed before any chain write → full refund.
        drop(grant);
        assert_eq!(store_left(&store, "g"), (2, 10, 100));

        // Delivery not confirmed AFTER the chain write → global stays spent.
        let mut grant = granted(claim(&store, "pk", None, l));
        grant.mark_chain_write();
        drop(grant);
        assert_eq!(store_left(&store, "g"), (2, 10, 99));

        granted(claim(&store, "pk", None, l)).keep();
        granted(claim(&store, "pk", None, l)).keep();
        assert_eq!(store_left(&store, "g"), (0, 8, 97));
        assert_eq!(
            denied(claim(&store, "pk", None, l)),
            FreeAnchorDenied::AccountQuotaUsed
        );
        assert_eq!(
            denied(claim(&store, "pk", None, FreeAnchorLimits::disabled())),
            FreeAnchorDenied::Disabled
        );
        assert_eq!(
            denied(claim(&store, "pk-unlinked", None, l)),
            FreeAnchorDenied::GoogleAccountRequired
        );
        let big = claim_free_anchor(&store, "pk", None, l.max_bytes + 1, None, l).unwrap();
        assert_eq!(denied(big), FreeAnchorDenied::TooLarge);
    }

    #[test]
    fn claim_skips_an_operation_already_on_the_paid_path() {
        let store = quota_store("pk", "g");
        let l = limits(2, 10, 100);
        {
            let guard = store.lock().unwrap();
            paid_operation::create_or_get(
                guard.conn(),
                NewPaidOperation {
                    operation_id: "op-quoted",
                    subject_hash: "h",
                    artifact_hash: "a",
                    created_at: "2026-09-27T00:00:00Z",
                },
            )
            .unwrap();
            paid_operation::record_quote(
                guard.conn(),
                "op-quoted",
                "0x1111111111111111111111111111111111111111",
                "0xdigest",
                "q_1",
                "2026-09-27T00:05:00Z",
                "2026-09-27T00:00:01Z",
            )
            .unwrap();
        }
        // A quoted operation stays paid: no free anchor, no counter change.
        assert_eq!(
            denied(claim(&store, "pk", Some("op-quoted"), l)),
            FreeAnchorDenied::PaidOperationInProgress
        );
        assert_eq!(store_left(&store, "g"), (2, 10, 100));
        // An operation with no paid state yet may use the free quota.
        granted(claim(&store, "pk", Some("op-new"), l)).keep();
        assert_eq!(store_left(&store, "g"), (1, 9, 99));
    }

    /// Measure a typical anchored artifact, built exactly like
    /// `tools::sign_memory_deferred`, to justify `DEFAULT_FREE_ANCHOR_MAX_BYTES`
    /// and `COSE_SIGN1_OVERHEAD_BYTES`.
    #[test]
    fn typical_artifact_size() {
        use mnemonic_core::codec::{canonical::to_canonical_cbor, schema, sign::sign_cose};
        use mnemonic_core::compress::EmbeddingCompressor;
        let keypair = solana_sdk::signature::Keypair::new();
        let content = "The user prefers concise answers and deploys on Fridays. ".repeat(18);
        assert!(content.len() >= 1000);
        let mut sizes = Vec::new();
        // 384 dims = fastembed (the default embedder). A 1536-dim OpenAI embedding
        // adds 768 base64 bytes over 384 dims, about 2.5 KiB in total;
        // building that compressor is too slow for a unit test.
        {
            let dim = 384usize;
            let embedding: Vec<f32> = (0..dim).map(|i| ((i as f32) * 0.37).sin()).collect();
            let compressed = EmbeddingCompressor::new(dim, 4, 42).compress(&embedding);
            let artifact = serde_json::json!({
                "artifact_id": uuid::Uuid::new_v4().to_string(),
                "type": "memory",
                "schema_version": 1,
                "content": content,
                "producer": "did:sol:9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
                "created_at": "2026-09-27T10:00:00+00:00",
                "tags": ["preferences", "deploy"],
                "metadata": {
                    "embed_provider": "fastembed",
                    "embed_dim": dim,
                    "turbo_bits": compressed.bit_width,
                    "embedding_compressed": base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        compressed.to_bytes(),
                    ),
                },
            });
            let cbor = to_canonical_cbor(&artifact, &schema::MEMORY_V1).unwrap();
            let cose = sign_cose(&cbor, &keypair).unwrap();
            let overhead = cose.len() - cbor.len();
            eprintln!(
                "dim {dim}: content {} B, canonical CBOR {} B, COSE_Sign1 {} B, envelope overhead {overhead} B",
                content.len(),
                cbor.len(),
                cose.len()
            );
            assert!(overhead <= COSE_SIGN1_OVERHEAD_BYTES, "overhead {overhead}");
            sizes.push(cose.len());
        }
        // A 1 KiB memory stays far below the free limit.
        assert!(sizes[0] < 4 * 1024, "{sizes:?}");
        assert!(sizes.iter().all(|s| *s * 4 < DEFAULT_FREE_ANCHOR_MAX_BYTES));
    }

    // ── x402 replay protection ──────────────────────────────────────────────

    #[test]
    fn x402_nonce_claim_is_single_use_until_released() {
        let store = SqliteStore::in_memory().unwrap();
        init_payment_schema(store.conn()).unwrap();
        assert!(claim_x402_nonce(&store, "sig-1").unwrap());
        // A second (or concurrent) request with the same payment loses.
        assert!(!claim_x402_nonce(&store, "sig-1").unwrap());
        assert!(x402_nonce_already_consumed(&store, "sig-1").unwrap());
        // A failed call gives the payment back for a retry.
        release_x402_nonce(&store, "sig-1").unwrap();
        assert!(!x402_nonce_already_consumed(&store, "sig-1").unwrap());
        assert!(claim_x402_nonce(&store, "sig-1").unwrap());
    }

    #[test]
    fn concurrent_x402_claims_have_exactly_one_winner() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        {
            let store = SqliteStore::open(&path).unwrap();
            init_payment_schema(store.conn()).unwrap();
        }
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let store = SqliteStore::open(&path).unwrap();
                    store
                        .conn()
                        .execute_batch("PRAGMA busy_timeout=10000;")
                        .unwrap();
                    claim_x402_nonce(&store, "raced-sig").unwrap()
                })
            })
            .collect();
        let winners = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
    }

    #[test]
    fn evm_tx_hash_spellings_normalize_to_one_nonce() {
        let proof = |sig: &str, network: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-payment",
                serde_json::json!({"tx_sig": sig, "network": network})
                    .to_string()
                    .parse()
                    .unwrap(),
            );
            extract_x402_proof(&headers).unwrap().tx_sig
        };
        let canonical = "0xabcdef0123";
        assert_eq!(proof("0xABCDEF0123", "arc"), canonical);
        assert_eq!(proof("0XabcDEF0123", "eip155:84532"), canonical);
        assert_eq!(proof(" abcdef0123 ", "base"), canonical);
        // Solana base58 is case-sensitive: only trimmed.
        assert_eq!(proof(" 5AbC ", "solana-mainnet"), "5AbC");
    }

    // ── M1: x402 v2 wire format (x402-v2-conformance) ──────────────────────

    /// x402_required produces a conformant v2 body with no unrecognised
    /// top-level fields and CAIP-2 network ids.
    #[test]
    fn x402_required_v2_shape_solana_only() {
        let body = x402_required(
            "treasury111",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            1000,
            "mnemonic_sign_memory attestation fee",
            None,
        );
        assert_eq!(body.x402_version, 2, "must be x402Version 2");
        assert_eq!(body.accepts.len(), 1);
        let a = &body.accepts[0];
        assert_eq!(a.scheme, "exact");
        assert_eq!(a.network, "solana-mainnet");
        assert_eq!(a.amount, "1000");
        assert_eq!(a.asset, "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        assert_eq!(a.pay_to, "treasury111");
        assert_eq!(a.max_timeout_seconds, 300);
        // No approval_url at top level — it goes in extensions for the UP rail.
        let json = serde_json::to_value(&body).unwrap();
        assert!(
            json.get("approval_url").is_none(),
            "approval_url must not be top-level"
        );
        assert!(json.get("status").is_none(), "status must not be top-level");
        assert!(
            json.get("correlation_id").is_none(),
            "correlation_id must not be top-level"
        );
    }

    #[test]
    fn x402_required_v2_shape_evm_network_caip2() {
        let evm = EvmPaymentConfig {
            rpc_url: "http://rpc.example".into(),
            usdc_token: "0xtoken".into(),
            treasury: "0xtreasury".into(),
            caip2_network: "eip155:84532".into(),
        };
        let body = x402_required("sol_treasury", "sol_mint", 500, "fee", Some(&evm));
        assert_eq!(body.x402_version, 2);
        assert_eq!(body.accepts.len(), 2, "Solana + EVM entries");
        let evm_entry = &body.accepts[1];
        assert_eq!(
            evm_entry.network, "eip155:84532",
            "EVM entry must use CAIP-2"
        );
        assert_eq!(evm_entry.amount, "500");
        assert_eq!(evm_entry.asset, "0xtoken");
        assert_eq!(evm_entry.pay_to, "0xtreasury");
    }

    #[test]
    fn up_payment_required_approval_url_in_extensions() {
        let up = UniversalPaywallPaymentRequired {
            operation_id: "op-1".into(),
            quote_id: "q-1".into(),
            approval_url: "https://example.com/approve?op=op-1".into(),
            scheme: "exact".into(),
            network: "eip155:84532".into(),
            asset: "0xasset".into(),
            pay_to: "0xpayto".into(),
            payer_wallet: "0xpayer".into(),
            amount: "2000".into(),
            binding_digest: "0xdigest".into(),
        };
        let body = up_payment_required(&up, "https://api.example.com/mcp");
        assert_eq!(body.x402_version, 2);
        assert_eq!(body.accepts.len(), 1);
        let a = &body.accepts[0];
        assert_eq!(a.scheme, "exact");
        assert_eq!(a.network, "eip155:84532");
        assert_eq!(a.amount, "2000");
        // approval_url must be in extensions, not at the top level.
        let ext = a.extensions.as_ref().unwrap();
        assert_eq!(ext["approval_url"], "https://example.com/approve?op=op-1");
        assert_eq!(ext["operation_id"], "op-1");
        assert_eq!(ext["payer_wallet"], "0xpayer");
        // Top-level must have no unrecognised fields.
        let json = serde_json::to_value(&body).unwrap();
        assert!(
            json.get("approval_url").is_none(),
            "approval_url must not be top-level"
        );
        assert!(
            json.get("operation_id").is_none(),
            "operation_id must not be top-level"
        );
    }

    /// Serialised `PaymentRequired` has exactly the fields the spec expects:
    /// x402Version (camelCase), accepts, and optionally free_anchors.
    #[test]
    fn payment_required_top_level_field_names() {
        let body = x402_required("t", "m", 100, "d", None);
        let json = serde_json::to_value(&body).unwrap();
        let obj = json.as_object().unwrap();
        let keys: Vec<_> = obj.keys().collect();
        // Only x402Version and accepts at the top level (free_anchors is absent).
        assert!(
            keys.contains(&&"x402Version".to_string()),
            "x402Version key missing"
        );
        assert!(
            keys.contains(&&"accepts".to_string()),
            "accepts key missing"
        );
        // maxAmountRequired must NOT appear (v1 field).
        let accepts = &json["accepts"][0];
        assert!(
            accepts.get("maxAmountRequired").is_none(),
            "v1 field maxAmountRequired must be absent"
        );
        assert!(
            accepts.get("amount").is_some(),
            "v2 field amount must be present"
        );
        assert!(
            accepts.get("resource").is_some(),
            "resource object must be present"
        );
        assert!(
            accepts.get("maxTimeoutSeconds").is_some(),
            "maxTimeoutSeconds must be present"
        );
    }
}

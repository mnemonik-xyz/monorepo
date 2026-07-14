//! Minimal Universal Paywall provider client for the Mnemonic MCP server.
//!
//! Implements the provider-neutral boundary described in
//! `work/universal-paywall-integration/tech-spec.md` for the `exact`
//! one-time x402 rail. The stake rail is out of scope for this milestone.

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// EVM address (0x-prefixed, 40 hex chars).
pub type HexAddress = String;

/// 32-byte hash (0x-prefixed, 64 hex chars).
pub type HexHash = String;

/// Operation binding v1 — must stay byte-compatible with
/// `packages/facilitator/schemas/operation-binding.v1.schema.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OperationBinding {
    pub version: u32,
    pub operation_id: String,
    pub payer_subject: String,
    pub payer_wallet: HexAddress,
    pub artifact_hash: String,
    pub amount: String,
    pub asset: HexAddress,
    pub network: String,
    pub pay_to: HexAddress,
    pub expires_at: String,
    pub nonce: String,
    pub scope: OperationScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OperationScope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_hash: Option<HexHash>,
    pub visibility: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct QuoteResponse {
    pub quote_id: String,
    pub binding: OperationBinding,
    pub binding_digest: HexHash,
    pub accepts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExactAuthorization {
    pub signature: HexHash,
    pub authorization: ExactAuthorizationMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactAuthorizationMessage {
    pub from: HexAddress,
    pub to: HexAddress,
    pub value: String,
    pub valid_after: String,
    pub valid_before: String,
    pub nonce: HexHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaymentAuthorization {
    pub scheme: String,
    pub payer_wallet: HexAddress,
    pub authorization: ExactAuthorization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SettleRequest {
    pub binding: OperationBinding,
    pub payment: PaymentAuthorization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SignedReceiptPayload {
    pub version: u32,
    pub service_id: String,
    pub operation_id: String,
    pub scheme: String,
    pub binding_digest: HexHash,
    pub payer_wallet: HexAddress,
    pub amount: String,
    pub asset: HexAddress,
    pub network: String,
    pub pay_to: HexAddress,
    pub settlement_tx: HexHash,
    pub settled_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SignedProviderReceipt {
    pub payload: SignedReceiptPayload,
    pub signature: ReceiptSignature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReceiptSignature {
    pub algorithm: String,
    pub key_id: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaymentReceipt {
    pub operation_id: String,
    pub scheme: String,
    pub status: String,
    pub binding_digest: HexHash,
    pub payer_wallet: HexAddress,
    pub amount: String,
    pub asset: HexAddress,
    pub network: String,
    pub pay_to: HexAddress,
    pub settlement_tx: HexHash,
    pub settled_at: String,
    pub receipt: SignedProviderReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub struct ProviderPaymentStatus {
    pub operation_id: String,
    pub status: String,
    pub receipt: Option<PaymentReceipt>,
    pub error: Option<String>,
}

/// Client configuration loaded from environment.
#[derive(Debug, Clone)]
pub struct UniversalPaywallConfig {
    pub url: String,
    pub api_key: String,
    pub network: String,
    pub asset: HexAddress,
    pub pay_to: HexAddress,
    pub payer_wallet: HexAddress,
    pub approval_url_base: String,
}

/// Thin async client over the Universal Paywall synchronous session API.
pub struct UniversalPaywallClient {
    config: UniversalPaywallConfig,
    client: reqwest::Client,
}

impl UniversalPaywallClient {
    pub fn new(config: UniversalPaywallConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn auth_header(&self) -> String {
        self.config.api_key.clone()
    }

    /// Fetch a previously created quote by operation id.
    pub async fn get_quote_by_operation_id(
        &self,
        operation_id: &str,
    ) -> anyhow::Result<QuoteResponse> {
        let url = format!("{}/v1/quotes/{}", self.config.url, operation_id);
        let resp = self
            .client
            .get(&url)
            .header("X-API-Key", self.auth_header())
            .send()
            .await
            .context("universal-paywall get_quote request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("get_quote failed (HTTP {status}): {body}");
        }
        resp.json().await.context("parse get_quote response")
    }

    /// Ask Universal Paywall to accept and identify an immutable operation quote.
    pub async fn create_quote(&self, binding: &OperationBinding) -> anyhow::Result<QuoteResponse> {
        let url = format!("{}/v1/quotes", self.config.url);
        let resp = self
            .client
            .post(&url)
            .header("X-API-Key", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "binding": binding }))
            .send()
            .await
            .context("universal-paywall create_quote request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("create_quote failed (HTTP {status}): {body}");
        }
        resp.json().await.context("parse create_quote response")
    }

    /// Settle an exact x402 authorization. Idempotent: retries return the existing receipt.
    pub async fn settle_exact(
        &self,
        binding: &OperationBinding,
        auth: &ExactAuthorization,
    ) -> anyhow::Result<PaymentReceipt> {
        let url = format!("{}/v1/payments/settle", self.config.url);
        let payment = PaymentAuthorization {
            scheme: "exact".into(),
            payer_wallet: binding.payer_wallet.clone(),
            authorization: auth.clone(),
        };
        let req_body = SettleRequest {
            binding: binding.clone(),
            payment,
        };
        let resp = self
            .client
            .post(&url)
            .header("X-API-Key", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .context("universal-paywall settle_exact request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("settle_exact failed (HTTP {status}): {body}");
        }
        resp.json().await.context("parse settle_exact response")
    }

    /// Recover durable payment status and receipt.
    #[allow(dead_code)]
    pub async fn payment_status(&self, operation_id: &str) -> anyhow::Result<ProviderPaymentStatus> {
        let url = format!("{}/v1/payments/{}", self.config.url, operation_id);
        let resp = self
            .client
            .get(&url)
            .header("X-API-Key", self.auth_header())
            .send()
            .await
            .context("universal-paywall payment_status request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("payment_status failed (HTTP {status}): {body}");
        }
        resp.json().await.context("parse payment_status response")
    }

    pub fn approval_url(&self, operation_id: &str, quote_id: &str) -> String {
        format!(
            "{}?operation_id={}&quote_id={}",
            self.config.approval_url_base, operation_id, quote_id
        )
    }
}

/// Stored quote state kept in-memory for idempotent resume.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StoredQuote {
    pub operation_id: String,
    pub quote_id: String,
    pub binding: OperationBinding,
    pub binding_digest: HexHash,
    pub receipt: Option<PaymentReceipt>,
}

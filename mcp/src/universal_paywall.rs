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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// The configured provider remains the trust anchor. Validate its response
/// against the immutable quote before making a durable settled-state claim.
/// This checks semantic binding, not the provider's detached signature.
pub fn validate_payment_receipt(
    receipt: &PaymentReceipt,
    binding: &OperationBinding,
    expected_digest: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        receipt.status == "settled" && receipt.scheme == "exact",
        "provider receipt is not settled exact payment"
    );
    anyhow::ensure!(
        receipt.operation_id == binding.operation_id
            && receipt.binding_digest.eq_ignore_ascii_case(expected_digest)
            && receipt
                .payer_wallet
                .eq_ignore_ascii_case(&binding.payer_wallet)
            && receipt.amount == binding.amount
            && receipt.asset.eq_ignore_ascii_case(&binding.asset)
            && receipt.network == binding.network
            && receipt.pay_to.eq_ignore_ascii_case(&binding.pay_to),
        "provider receipt quote binding mismatch"
    );
    anyhow::ensure!(
        !receipt.settlement_tx.is_empty()
            && chrono::DateTime::parse_from_rfc3339(&receipt.settled_at).is_ok(),
        "provider receipt lacks settlement evidence"
    );
    let signed = &receipt.receipt.payload;
    anyhow::ensure!(
        signed.version == 1
            && !signed.service_id.is_empty()
            && signed.operation_id == receipt.operation_id
            && signed.scheme == receipt.scheme
            && signed
                .binding_digest
                .eq_ignore_ascii_case(&receipt.binding_digest)
            && signed
                .payer_wallet
                .eq_ignore_ascii_case(&receipt.payer_wallet)
            && signed.amount == receipt.amount
            && signed.asset.eq_ignore_ascii_case(&receipt.asset)
            && signed.network == receipt.network
            && signed.pay_to.eq_ignore_ascii_case(&receipt.pay_to)
            && signed
                .settlement_tx
                .eq_ignore_ascii_case(&receipt.settlement_tx)
            && signed.settled_at == receipt.settled_at,
        "provider signed receipt payload mismatch"
    );
    Ok(())
}

/// Client configuration loaded from environment.
#[derive(Debug, Clone)]
pub struct UniversalPaywallConfig {
    pub url: String,
    pub api_key: String,
    pub network: String,
    pub asset: HexAddress,
    pub pay_to: HexAddress,
    // Reserved for the in-flight paid-anchoring work; not yet read from the
    // binary. Reworked by M3 (work/x402-v2-conformance).
    #[allow(dead_code)]
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
        let quote: QuoteResponse = resp.json().await.context("parse create_quote response")?;
        anyhow::ensure!(
            quote.binding == *binding
                && !quote.quote_id.is_empty()
                && !quote.binding_digest.is_empty(),
            "provider quote binding mismatch"
        );
        Ok(quote)
    }

    /// Settle an exact x402 authorization. Idempotent: retries return the existing receipt.
    pub async fn settle_exact(
        &self,
        binding: &OperationBinding,
        expected_digest: &str,
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
        let receipt: PaymentReceipt = resp.json().await.context("parse settle_exact response")?;
        validate_payment_receipt(&receipt, binding, expected_digest)?;
        Ok(receipt)
    }

    /// Recover durable payment status and receipt.
    #[allow(dead_code)]
    pub async fn payment_status(
        &self,
        operation_id: &str,
    ) -> anyhow::Result<ProviderPaymentStatus> {
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

    /// A deterministic, expiring bearer capability for the approval browser.
    /// It is derived from the facilitator API secret, never the public
    /// operation id alone, and can be regenerated after an MCP restart.
    pub fn resume_token(&self, operation_id: &str, quote_id: &str, expires_at: &str) -> String {
        let key = blake3::hash(self.config.api_key.as_bytes());
        let mut material = Vec::new();
        material.extend_from_slice(b"mnemonic:approval-resume:v1\0");
        material.extend_from_slice(operation_id.as_bytes());
        material.push(0);
        material.extend_from_slice(quote_id.as_bytes());
        material.push(0);
        material.extend_from_slice(expires_at.as_bytes());
        blake3::keyed_hash(key.as_bytes(), &material)
            .to_hex()
            .to_string()
    }

    pub fn approval_url(&self, operation_id: &str, quote_id: &str, expires_at: &str) -> String {
        let resume_token = self.resume_token(operation_id, quote_id, expires_at);
        format!(
            "{}?operation_id={}&quote_id={}&resume_token={}",
            self.config.approval_url_base, operation_id, quote_id, resume_token
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

#[cfg(test)]
mod receipt_binding_tests {
    use super::*;
    fn fixture() -> (OperationBinding, PaymentReceipt) {
        let binding = OperationBinding {
            version: 1,
            operation_id: "operation-1".into(),
            payer_subject: "author".into(),
            payer_wallet: "0xwallet".into(),
            artifact_hash: "artifact".into(),
            amount: "123".into(),
            asset: "0xasset".into(),
            network: "eip155:84532".into(),
            pay_to: "0xmerchant".into(),
            expires_at: "2026-10-03T00:00:00Z".into(),
            nonce: "nonce".into(),
            scope: OperationScope {
                workspace_hash: None,
                visibility: "private".into(),
                action: "anchor".into(),
            },
        };
        let payload = SignedReceiptPayload {
            version: 1,
            service_id: "trusted-configured-provider".into(),
            operation_id: binding.operation_id.clone(),
            scheme: "exact".into(),
            binding_digest: "0xdigest".into(),
            payer_wallet: binding.payer_wallet.clone(),
            amount: binding.amount.clone(),
            asset: binding.asset.clone(),
            network: binding.network.clone(),
            pay_to: binding.pay_to.clone(),
            settlement_tx: "0xtransaction".into(),
            settled_at: "2026-10-02T00:00:00Z".into(),
        };
        let receipt = PaymentReceipt {
            operation_id: payload.operation_id.clone(),
            scheme: payload.scheme.clone(),
            status: "settled".into(),
            binding_digest: payload.binding_digest.clone(),
            payer_wallet: payload.payer_wallet.clone(),
            amount: payload.amount.clone(),
            asset: payload.asset.clone(),
            network: payload.network.clone(),
            pay_to: payload.pay_to.clone(),
            settlement_tx: payload.settlement_tx.clone(),
            settled_at: payload.settled_at.clone(),
            receipt: SignedProviderReceipt {
                payload,
                signature: ReceiptSignature {
                    algorithm: "ed25519".into(),
                    key_id: "provider-key".into(),
                    value: "provider-signature".into(),
                },
            },
        };
        (binding, receipt)
    }
    #[test]
    fn rejects_unsettled_wrong_operation_and_conflicting_quote_or_signed_payload() {
        let (binding, receipt) = fixture();
        validate_payment_receipt(&receipt, &binding, "0xdigest").unwrap();
        let original = serde_json::to_value(&receipt).unwrap();
        for field in [
            "operation_id",
            "status",
            "scheme",
            "binding_digest",
            "payer_wallet",
            "amount",
            "asset",
            "network",
            "pay_to",
            "settlement_tx",
            "settled_at",
        ] {
            let mut changed = original.clone();
            changed[field] = serde_json::json!(if field == "settlement_tx" {
                ""
            } else {
                "wrong"
            });
            let receipt: PaymentReceipt = serde_json::from_value(changed).unwrap();
            assert!(
                validate_payment_receipt(&receipt, &binding, "0xdigest").is_err(),
                "{field}"
            );
        }
        let mut receipt = receipt;
        receipt.receipt.payload.operation_id = "another-operation".into();
        assert!(validate_payment_receipt(&receipt, &binding, "0xdigest").is_err());
    }
    #[test]
    fn cached_receipt_requires_durable_quote_identity_and_valid_settled_evidence() {
        use crate::paid_operation as paid;
        let (binding, receipt) = fixture();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        paid::migrate_paid_operations(&conn).unwrap();
        paid::create_or_get(
            &conn,
            paid::NewPaidOperation {
                operation_id: &binding.operation_id,
                subject_hash: "subject",
                artifact_hash: &binding.artifact_hash,
                created_at: "now",
            },
        )
        .unwrap();
        paid::record_quote(
            &conn,
            &binding.operation_id,
            &binding.payer_wallet,
            "0xdigest",
            "quote",
            "later",
            "now",
        )
        .unwrap();
        let mut operation = paid::record_provider_receipt(
            &conn,
            &binding.operation_id,
            &serde_json::to_string(&receipt).unwrap(),
            "now",
        )
        .unwrap();
        assert!(paid::settled_receipt(&operation).unwrap().is_some());
        operation.operation_id = "other-operation".into();
        assert!(paid::settled_receipt(&operation).is_err());
        operation.operation_id = binding.operation_id;
        operation.provider_receipt_json = Some(r#"{"status":"settled"}"#.into());
        assert!(paid::settled_receipt(&operation).is_err());
    }

    #[tokio::test]
    async fn http_success_with_wrong_receipt_is_not_accepted_as_settlement() {
        use httpmock::prelude::*;
        let server = MockServer::start();
        let (binding, mut receipt) = fixture();
        receipt.operation_id = "wrong-operation".into();
        server.mock(|when, then| {
            when.method(POST).path("/v1/payments/settle");
            then.status(200)
                .json_body(serde_json::to_value(receipt).unwrap());
        });
        let mut other_binding = binding.clone();
        other_binding.amount = "999999".into();
        server.mock(|when, then| {
            when.method(POST).path("/v1/quotes");
            then.status(200).json_body(
                serde_json::to_value(QuoteResponse {
                    quote_id: "q".into(),
                    binding: other_binding,
                    binding_digest: "0xdigest".into(),
                    accepts: vec![],
                })
                .unwrap(),
            );
        });
        let client = UniversalPaywallClient::new(UniversalPaywallConfig {
            url: server.base_url(),
            api_key: "test".into(),
            network: binding.network.clone(),
            asset: binding.asset.clone(),
            pay_to: binding.pay_to.clone(),
            payer_wallet: binding.payer_wallet.clone(),
            approval_url_base: String::new(),
        });
        assert!(client.create_quote(&binding).await.is_err());
        let authorization = ExactAuthorization {
            signature: "test".into(),
            authorization: ExactAuthorizationMessage {
                from: binding.payer_wallet.clone(),
                to: binding.pay_to.clone(),
                value: binding.amount.clone(),
                valid_after: "0".into(),
                valid_before: "1".into(),
                nonce: "nonce".into(),
            },
        };
        assert!(client
            .settle_exact(&binding, "0xdigest", &authorization)
            .await
            .is_err());
    }
}

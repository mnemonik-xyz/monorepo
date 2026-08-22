//! Durable paid-operation state and provider-neutral settlement boundary.
//!
//! Payment state is deliberately separate from the attestation/recall index.
//! A paid operation may use SQLite for restart recovery, but recalling an
//! already anchored artifact never consults this module.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mnemonic_core::storage::{SqliteStore, Visibility};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MIGRATION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS paid_operations (
    operation_id TEXT PRIMARY KEY,
    payer_subject TEXT NOT NULL,
    payer_wallet TEXT NOT NULL,
    artifact_hash TEXT NOT NULL,
    amount TEXT NOT NULL,
    asset TEXT NOT NULL,
    network TEXT NOT NULL,
    pay_to TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    nonce TEXT NOT NULL,
    workspace TEXT,
    workspace_hash TEXT,
    visibility TEXT NOT NULL DEFAULT 'private',
    action TEXT NOT NULL DEFAULT 'manual',
    signer_pubkey TEXT NOT NULL,
    cose_signed_bytes BLOB NOT NULL,
    content TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    embedding BLOB NOT NULL,
    state TEXT NOT NULL,
    payment_scheme TEXT,
    session_id TEXT,
    receipt_json TEXT,
    attestation_id TEXT,
    solana_tx TEXT,
    arweave_tx TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_paid_operations_state
    ON paid_operations(state, updated_at);
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationBinding {
    pub version: u8,
    pub operation_id: String,
    pub payer_subject: String,
    pub payer_wallet: String,
    pub artifact_hash: String,
    /// Decimal micro-USDC string. Strings avoid JSON integer-width drift.
    pub amount: String,
    pub asset: String,
    pub network: String,
    pub pay_to: String,
    pub expires_at: String,
    pub nonce: String,
    pub scope: OperationScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationScope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_hash: Option<String>,
    pub visibility: String,
    pub action: String,
}

impl OperationScope {
    pub fn new(workspace: Option<&str>, visibility: Visibility, action: &str) -> Self {
        Self {
            workspace_hash: workspace
                .map(|value| blake3::hash(value.as_bytes()).to_hex().to_string()),
            visibility: visibility.as_str().to_string(),
            action: action.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scheme", rename_all = "snake_case")]
pub enum PaymentAuthorization {
    #[serde(rename = "stake", alias = "session")]
    Stake {
        session_id: String,
        payer_wallet: String,
        #[serde(default)]
        authorization: serde_json::Value,
    },
    Exact {
        payer_wallet: String,
        authorization: serde_json::Value,
    },
}

impl PaymentAuthorization {
    pub fn payer_wallet(&self) -> &str {
        match self {
            Self::Stake { payer_wallet, .. } | Self::Exact { payer_wallet, .. } => payer_wallet,
        }
    }

    pub fn scheme(&self) -> &'static str {
        match self {
            Self::Stake { .. } => "stake",
            Self::Exact { .. } => "exact",
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Stake { session_id, .. } => Some(session_id),
            Self::Exact { .. } => None,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let authorization = match self {
            Self::Stake {
                session_id,
                authorization,
                ..
            } => {
                if session_id.trim().is_empty() || session_id.len() > 128 {
                    anyhow::bail!("session_id must contain 1-128 characters")
                }
                authorization
            }
            Self::Exact { authorization, .. } => authorization,
        };
        if authorization.is_null() {
            anyhow::bail!("payment authorization must not be null")
        }
        if serde_json::to_vec(authorization)?.len() > 16 * 1024 {
            anyhow::bail!("payment authorization exceeds 16 KiB")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleRequest {
    pub binding: OperationBinding,
    pub payment: PaymentAuthorization,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentReceipt {
    pub operation_id: String,
    pub scheme: String,
    pub status: String,
    pub binding_digest: String,
    pub payer_wallet: String,
    pub amount: String,
    pub asset: String,
    pub network: String,
    pub pay_to: String,
    #[serde(default)]
    pub settlement_tx: Option<String>,
    pub settled_at: String,
    /// Provider-defined signed receipt. Mnemonic stores it byte-for-byte.
    pub receipt: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPaymentStatus {
    pub operation_id: String,
    pub status: String,
    #[serde(default)]
    pub receipt: Option<PaymentReceipt>,
}

pub fn binding_digest(binding: &OperationBinding) -> anyhow::Result<String> {
    // Struct field order is stable under serde and forms the V1 canonical
    // fixture. Both sides additionally compare every receipt field below, so
    // the digest is never the sole validation boundary.
    Ok(blake3::hash(&serde_json::to_vec(binding)?)
        .to_hex()
        .to_string())
}

fn validate_receipt(
    binding: &OperationBinding,
    payment: &PaymentAuthorization,
    receipt: &PaymentReceipt,
) -> anyhow::Result<()> {
    let expected_digest = binding_digest(binding)?;
    if receipt.operation_id != binding.operation_id
        || receipt.status != "settled"
        || receipt.scheme != payment.scheme()
        || receipt.binding_digest != expected_digest
        || !receipt
            .payer_wallet
            .eq_ignore_ascii_case(&binding.payer_wallet)
        || receipt.amount != binding.amount
        || !receipt.asset.eq_ignore_ascii_case(&binding.asset)
        || receipt.network != binding.network
        || !receipt.pay_to.eq_ignore_ascii_case(&binding.pay_to)
        || receipt.receipt.is_null()
    {
        anyhow::bail!("payment receipt does not match the immutable operation binding")
    }
    Ok(())
}

#[derive(Debug)]
pub enum ProviderError {
    Transport(String),
    Rejected { status: u16, message: String },
    InvalidResponse(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(message) => write!(f, "payment provider request failed: {message}"),
            Self::Rejected { status, message } => {
                write!(
                    f,
                    "payment provider rejected operation ({status}): {message}"
                )
            }
            Self::InvalidResponse(message) => {
                write!(f, "invalid payment provider response: {message}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    async fn settle(&self, request: &SettleRequest) -> Result<PaymentReceipt, ProviderError>;
    async fn status(&self, operation_id: &str) -> Result<ProviderPaymentStatus, ProviderError>;
}

#[derive(Debug, Clone, Serialize)]
pub struct PaymentQuoteConfig {
    pub payment_url: String,
    pub network: String,
    pub asset: String,
    pub pay_to: String,
    pub session_cap: String,
    pub session_max_per_anchor: String,
    pub session_valid_for_seconds: u64,
}

pub struct PaidAnchoring {
    pub provider: Arc<dyn PaymentProvider>,
    pub quote: PaymentQuoteConfig,
}

#[derive(Clone)]
pub struct UniversalPaywallClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

/// Paid-flow URLs carry operation capabilities and service credentials, so
/// production endpoints must use TLS. Plain HTTP is accepted only for an
/// explicit loopback host used by local tests and development.
pub fn validate_secure_http_url(value: &str) -> anyhow::Result<url::Url> {
    let parsed = url::Url::parse(value)?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("URL credentials are not allowed")
    }
    let loopback = parsed.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
        anyhow::bail!("URL must use https (plain http is allowed only on loopback)")
    }
    Ok(parsed)
}

impl UniversalPaywallClient {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self, ProviderError> {
        let base_url = base_url.trim_end_matches('/').to_string();
        validate_secure_http_url(&base_url)
            .map_err(|e| ProviderError::InvalidResponse(format!("invalid provider URL: {e}")))?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(35))
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(Self {
            http,
            base_url,
            api_key: api_key.to_string(),
        })
    }

    async fn decode<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, ProviderError> {
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !status.is_success() {
            let message = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
            return Err(ProviderError::Rejected {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_slice(&bytes).map_err(|e| {
            ProviderError::InvalidResponse(format!("{e}: {}", String::from_utf8_lossy(&bytes)))
        })
    }
}

#[async_trait]
impl PaymentProvider for UniversalPaywallClient {
    async fn settle(&self, request: &SettleRequest) -> Result<PaymentReceipt, ProviderError> {
        let response = self
            .http
            .post(format!("{}/v1/payments/settle", self.base_url))
            .header("x-api-key", &self.api_key)
            .json(request)
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let receipt: PaymentReceipt = Self::decode(response).await?;
        validate_receipt(&request.binding, &request.payment, &receipt)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        Ok(receipt)
    }

    async fn status(&self, operation_id: &str) -> Result<ProviderPaymentStatus, ProviderError> {
        let encoded: String =
            url::form_urlencoded::byte_serialize(operation_id.as_bytes()).collect();
        let response = self
            .http
            .get(format!("{}/v1/payments/{encoded}", self.base_url))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status: ProviderPaymentStatus = Self::decode(response).await?;
        if status.operation_id != operation_id {
            return Err(ProviderError::InvalidResponse(
                "status operation_id does not match request".into(),
            ));
        }
        Ok(status)
    }
}

#[derive(Debug, Clone)]
pub struct NewPaidOperation {
    pub binding: OperationBinding,
    pub signer_pubkey: String,
    pub cose_signed_bytes: Vec<u8>,
    pub content: String,
    pub tags: Vec<String>,
    pub embedding: Vec<f32>,
    pub workspace: Option<String>,
    pub visibility: Visibility,
}

#[derive(Debug, Clone)]
pub struct PaidOperation {
    pub binding: OperationBinding,
    pub signer_pubkey: String,
    pub cose_signed_bytes: Vec<u8>,
    pub content: String,
    pub tags: Vec<String>,
    pub embedding: Vec<f32>,
    pub workspace: Option<String>,
    pub visibility: Visibility,
    pub state: String,
    pub payment_scheme: Option<String>,
    pub session_id: Option<String>,
    pub receipt: Option<PaymentReceipt>,
    pub attestation_id: Option<String>,
    pub solana_tx: Option<String>,
    pub arweave_tx: Option<String>,
    pub last_error: Option<String>,
}

fn embedding_to_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn embedding_from_bytes(bytes: &[u8]) -> anyhow::Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        anyhow::bail!("stored paid-operation embedding has invalid length")
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

pub fn migrate(store: &SqliteStore) -> anyhow::Result<()> {
    store.conn().execute_batch(MIGRATION_SQL)?;
    ensure_column(store, "workspace", "TEXT")?;
    ensure_column(store, "workspace_hash", "TEXT")?;
    ensure_column(store, "visibility", "TEXT NOT NULL DEFAULT 'private'")?;
    ensure_column(store, "action", "TEXT NOT NULL DEFAULT 'manual'")?;
    // Crash recovery uses leases rather than leaving operations permanently
    // wedged. Provider settlement is idempotent by operation_id, so retrying
    // an uncertain payment asks the provider to return its existing receipt.
    store.conn().execute_batch(
        "UPDATE paid_operations
         SET state='payment_failed',last_error='stale payment claim recovered',updated_at=datetime('now')
         WHERE state='payment_authorizing'
           AND datetime(updated_at) < datetime('now','-2 minutes');
         UPDATE paid_operations
         SET state='delivery_retryable',last_error='stale anchoring claim recovered',updated_at=datetime('now')
         WHERE state IN ('anchoring','verifying_delivery')
           AND datetime(updated_at) < datetime('now','-5 minutes');",
    )?;
    Ok(())
}

fn ensure_column(store: &SqliteStore, column: &str, definition: &str) -> anyhow::Result<()> {
    let mut statement = store.conn().prepare("PRAGMA table_info(paid_operations)")?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !names.iter().any(|name| name == column) {
        store.conn().execute_batch(&format!(
            "ALTER TABLE paid_operations ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

pub fn create_or_read(
    store: &SqliteStore,
    operation: &NewPaidOperation,
) -> anyhow::Result<PaidOperation> {
    migrate(store)?;
    let expected_scope = OperationScope::new(
        operation.workspace.as_deref(),
        operation.visibility,
        &operation.binding.scope.action,
    );
    if operation.binding.scope != expected_scope {
        anyhow::bail!("operation scope does not match workspace/visibility")
    }
    let now = Utc::now().to_rfc3339();
    let tags = serde_json::to_string(&operation.tags)?;
    let embedding = embedding_to_bytes(&operation.embedding);
    store.conn().execute(
        "INSERT OR IGNORE INTO paid_operations (
            operation_id, payer_subject, payer_wallet, artifact_hash, amount,
            asset, network, pay_to, expires_at, nonce, workspace,workspace_hash,
            visibility,action,signer_pubkey,cose_signed_bytes, content, tags_json, embedding, state,
            created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,
                   ?16,?17,?18,?19,'awaiting_payment',?20,?20)",
        params![
            operation.binding.operation_id,
            operation.binding.payer_subject,
            operation.binding.payer_wallet,
            operation.binding.artifact_hash,
            operation.binding.amount,
            operation.binding.asset,
            operation.binding.network,
            operation.binding.pay_to,
            operation.binding.expires_at,
            operation.binding.nonce,
            operation.workspace,
            operation.binding.scope.workspace_hash,
            operation.binding.scope.visibility,
            operation.binding.scope.action,
            operation.signer_pubkey,
            operation.cose_signed_bytes,
            operation.content,
            tags,
            embedding,
            now,
        ],
    )?;
    let existing = read(store, &operation.binding.operation_id)?
        .ok_or_else(|| anyhow::anyhow!("paid operation disappeared after insert"))?;
    if existing.binding != operation.binding
        || existing.signer_pubkey != operation.signer_pubkey
        || existing.cose_signed_bytes != operation.cose_signed_bytes
        || existing.content != operation.content
        || existing.tags != operation.tags
        || existing.embedding != operation.embedding
        || existing.workspace != operation.workspace
        || existing.visibility != operation.visibility
    {
        anyhow::bail!("operation binding mismatch for existing operation_id")
    }
    Ok(existing)
}

pub fn read(store: &SqliteStore, operation_id: &str) -> anyhow::Result<Option<PaidOperation>> {
    migrate(store)?;
    let row = store
        .conn()
        .query_row(
            "SELECT payer_subject,payer_wallet,artifact_hash,amount,asset,network,
                    pay_to,expires_at,nonce,workspace,workspace_hash,visibility,action,
                    signer_pubkey,cose_signed_bytes,content,tags_json,embedding,state,payment_scheme,session_id,receipt_json,
                    attestation_id,solana_tx,arweave_tx,last_error
             FROM paid_operations WHERE operation_id=?1",
            params![operation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, Vec<u8>>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, Vec<u8>>(17)?,
                    row.get::<_, String>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                    row.get::<_, Option<String>>(23)?,
                    row.get::<_, Option<String>>(24)?,
                    row.get::<_, Option<String>>(25)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(PaidOperation {
        binding: OperationBinding {
            version: 1,
            operation_id: operation_id.to_string(),
            payer_subject: row.0,
            payer_wallet: row.1,
            artifact_hash: row.2,
            amount: row.3,
            asset: row.4,
            network: row.5,
            pay_to: row.6,
            expires_at: row.7,
            nonce: row.8,
            scope: OperationScope {
                workspace_hash: row.10,
                visibility: row.11.clone(),
                action: row.12,
            },
        },
        workspace: row.9,
        visibility: Visibility::from_str_strict(&row.11)
            .ok_or_else(|| anyhow::anyhow!("invalid paid-operation visibility"))?,
        signer_pubkey: row.13,
        cose_signed_bytes: row.14,
        content: row.15,
        tags: serde_json::from_str(&row.16)?,
        embedding: embedding_from_bytes(&row.17)?,
        state: row.18,
        payment_scheme: row.19,
        session_id: row.20,
        receipt: row.21.map(|v| serde_json::from_str(&v)).transpose()?,
        attestation_id: row.22,
        solana_tx: row.23,
        arweave_tx: row.24,
        last_error: row.25,
    }))
}

fn transition(
    store: &SqliteStore,
    operation_id: &str,
    from: &[&str],
    to: &str,
) -> anyhow::Result<bool> {
    migrate(store)?;
    let placeholders = (0..from.len()).map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE paid_operations SET state=?,updated_at=? WHERE operation_id=? AND state IN ({placeholders})"
    );
    let now = Utc::now().to_rfc3339();
    let mut values: Vec<&dyn rusqlite::ToSql> = vec![&to, &now, &operation_id];
    values.extend(from.iter().map(|v| v as &dyn rusqlite::ToSql));
    Ok(store.conn().execute(&sql, values.as_slice())? == 1)
}

pub fn claim_payment(
    store: &SqliteStore,
    operation_id: &str,
    payment: &PaymentAuthorization,
) -> anyhow::Result<bool> {
    migrate(store)?;
    let now = Utc::now().to_rfc3339();
    Ok(store.conn().execute(
        "UPDATE paid_operations
         SET state='payment_authorizing',payment_scheme=?2,session_id=?3,
             last_error=NULL,updated_at=?4
         WHERE operation_id=?1 AND state IN ('awaiting_payment','payment_failed')",
        params![operation_id, payment.scheme(), payment.session_id(), now],
    )? == 1)
}

/// Bind the wallet selected in the payment UI to a signed-operation draft.
/// The first non-empty wallet wins; retries with the same wallet are
/// idempotent and attempts to substitute another wallet are rejected.
pub fn bind_payer_wallet(
    store: &SqliteStore,
    operation_id: &str,
    payer_wallet: &str,
) -> anyhow::Result<PaidOperation> {
    if payer_wallet.len() != 42
        || !payer_wallet.starts_with("0x")
        || !payer_wallet[2..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        anyhow::bail!("payer_wallet must be a 20-byte 0x-prefixed EVM address")
    }
    migrate(store)?;
    let now = Utc::now().to_rfc3339();
    store.conn().execute(
        "UPDATE paid_operations SET payer_wallet=?2,updated_at=?3
         WHERE operation_id=?1 AND payer_wallet='' AND state IN ('awaiting_payment','payment_failed')",
        params![operation_id, payer_wallet, now],
    )?;
    let operation =
        read(store, operation_id)?.ok_or_else(|| anyhow::anyhow!("paid operation not found"))?;
    if !operation
        .binding
        .payer_wallet
        .eq_ignore_ascii_case(payer_wallet)
    {
        anyhow::bail!("payer wallet conflicts with existing operation binding")
    }
    Ok(operation)
}

pub fn mark_payment_failed(
    store: &SqliteStore,
    operation_id: &str,
    error: &str,
) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    store.conn().execute(
        "UPDATE paid_operations SET state='payment_failed',last_error=?2,updated_at=?3
         WHERE operation_id=?1 AND state='payment_authorizing'",
        params![operation_id, error, now],
    )?;
    Ok(())
}

pub fn record_receipt(
    store: &SqliteStore,
    operation_id: &str,
    payment: &PaymentAuthorization,
    receipt: &PaymentReceipt,
) -> anyhow::Result<()> {
    let operation =
        read(store, operation_id)?.ok_or_else(|| anyhow::anyhow!("paid operation not found"))?;
    validate_receipt(&operation.binding, payment, receipt)?;
    let now = Utc::now().to_rfc3339();
    let receipt_json = serde_json::to_string(receipt)?;
    let changed = store.conn().execute(
        "UPDATE paid_operations
         SET state='payment_ready',payment_scheme=?2,session_id=?3,
             receipt_json=?4,last_error=NULL,updated_at=?5
         WHERE operation_id=?1 AND state='payment_authorizing'",
        params![
            operation_id,
            payment.scheme(),
            payment.session_id(),
            receipt_json,
            now,
        ],
    )?;
    if changed == 0 {
        let existing = read(store, operation_id)?
            .ok_or_else(|| anyhow::anyhow!("paid operation not found"))?;
        if existing.receipt.as_ref() != Some(receipt) {
            anyhow::bail!("payment receipt conflicts with existing operation state")
        }
    }
    Ok(())
}

pub fn claim_anchoring(store: &SqliteStore, operation_id: &str) -> anyhow::Result<bool> {
    transition(
        store,
        operation_id,
        &["payment_ready", "delivery_retryable"],
        "anchoring",
    )
}

/// Replace an expired, unpaid quote without changing the signed artifact or
/// operation ID. The old nonce becomes unusable; a client must obtain a fresh
/// wallet/session authorization against the returned binding.
pub fn refresh_unpaid_quote(
    store: &SqliteStore,
    operation_id: &str,
    amount_micro_usdc: i64,
) -> anyhow::Result<PaidOperation> {
    if amount_micro_usdc <= 0 {
        anyhow::bail!("anchoring quote amount must be positive")
    }
    let now = Utc::now().to_rfc3339();
    let expires = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
    let nonce = uuid::Uuid::new_v4().to_string();
    let changed = store.conn().execute(
        "UPDATE paid_operations
         SET payer_wallet='',amount=?2,expires_at=?3,nonce=?4,
             payment_scheme=NULL,session_id=NULL,last_error=NULL,updated_at=?5
         WHERE operation_id=?1 AND receipt_json IS NULL
           AND state IN ('awaiting_payment','payment_failed')",
        params![
            operation_id,
            amount_micro_usdc.to_string(),
            expires,
            nonce,
            now
        ],
    )?;
    if changed != 1 {
        anyhow::bail!("only an unpaid, inactive quote can be refreshed")
    }
    read(store, operation_id)?.ok_or_else(|| anyhow::anyhow!("paid operation not found"))
}

/// Persist partial chain progress before moving to the next paid side effect.
/// A retry reuses the same Irys item and Solana transaction instead of
/// spending the already-settled payment twice.
pub fn record_arweave(
    store: &SqliteStore,
    operation_id: &str,
    arweave_tx: &str,
) -> anyhow::Result<()> {
    record_chain_reference(store, operation_id, "arweave_tx", arweave_tx, false)
}

pub fn record_solana(
    store: &SqliteStore,
    operation_id: &str,
    solana_tx: &str,
) -> anyhow::Result<()> {
    record_chain_reference(store, operation_id, "solana_tx", solana_tx, true)
}

fn record_chain_reference(
    store: &SqliteStore,
    operation_id: &str,
    column: &str,
    value: &str,
    require_arweave: bool,
) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("chain reference must not be empty")
    }
    let arweave_guard = if require_arweave {
        " AND arweave_tx IS NOT NULL"
    } else {
        ""
    };
    let sql = format!(
        "UPDATE paid_operations SET {column}=?2,updated_at=?3
         WHERE operation_id=?1 AND state='anchoring'
           AND ({column} IS NULL OR {column}=?2){arweave_guard}"
    );
    let now = Utc::now().to_rfc3339();
    if store
        .conn()
        .execute(&sql, params![operation_id, value, now])?
        != 1
    {
        anyhow::bail!("paid operation rejected conflicting chain progress")
    }
    Ok(())
}

pub fn mark_delivery_retryable(
    store: &SqliteStore,
    operation_id: &str,
    error: &str,
) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    store.conn().execute(
        "UPDATE paid_operations SET state='delivery_retryable',last_error=?2,updated_at=?3
         WHERE operation_id=?1 AND state IN ('anchoring','verifying_delivery')",
        params![operation_id, error, now],
    )?;
    Ok(())
}

pub fn mark_verifying(store: &SqliteStore, operation_id: &str) -> anyhow::Result<()> {
    let _ = transition(store, operation_id, &["anchoring"], "verifying_delivery")?;
    Ok(())
}

pub fn mark_anchored(
    store: &SqliteStore,
    operation_id: &str,
    attestation_id: &str,
    solana_tx: &str,
    arweave_tx: &str,
) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    let changed = store.conn().execute(
        "UPDATE paid_operations
         SET state='anchored',attestation_id=?2,solana_tx=?3,arweave_tx=?4,
             last_error=NULL,updated_at=?5
         WHERE operation_id=?1 AND state IN ('anchoring','verifying_delivery')
           AND (solana_tx IS NULL OR solana_tx=?3)
           AND (arweave_tx IS NULL OR arweave_tx=?4)",
        params![operation_id, attestation_id, solana_tx, arweave_tx, now],
    )?;
    if changed != 1 {
        let existing = read(store, operation_id)?
            .ok_or_else(|| anyhow::anyhow!("paid operation not found"))?;
        if existing.state != "anchored"
            || existing.attestation_id.as_deref() != Some(attestation_id)
            || existing.solana_tx.as_deref() != Some(solana_tx)
            || existing.arweave_tx.as_deref() != Some(arweave_tx)
        {
            anyhow::bail!("paid operation was not in a compatible anchorable state")
        }
    }
    Ok(())
}

pub fn payer_subject(pubkey: &str) -> String {
    blake3::hash(pubkey.as_bytes()).to_hex().to_string()
}

pub fn new_binding(
    operation_id: &str,
    signer_pubkey: &str,
    payer_wallet: &str,
    artifact_hash: &str,
    amount_micro_usdc: i64,
    quote: &PaymentQuoteConfig,
    scope: OperationScope,
) -> OperationBinding {
    let expires = Utc::now() + chrono::Duration::minutes(5);
    OperationBinding {
        version: 1,
        operation_id: operation_id.to_string(),
        payer_subject: payer_subject(signer_pubkey),
        payer_wallet: payer_wallet.to_string(),
        artifact_hash: artifact_hash.to_string(),
        amount: amount_micro_usdc.to_string(),
        asset: quote.asset.clone(),
        network: quote.network.clone(),
        pay_to: quote.pay_to.clone(),
        expires_at: expires.to_rfc3339(),
        nonce: uuid::Uuid::new_v4().to_string(),
        scope,
    }
}

pub fn quote_expired(binding: &OperationBinding) -> bool {
    DateTime::parse_from_rfc3339(&binding.expires_at)
        .map(|v| v.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote() -> PaymentQuoteConfig {
        PaymentQuoteConfig {
            payment_url: "https://pay.example/session".into(),
            network: "eip155:5042002".into(),
            asset: "0x0000000000000000000000000000000000000001".into(),
            pay_to: "0x0000000000000000000000000000000000000002".into(),
            session_cap: "5000000".into(),
            session_max_per_anchor: "50000".into(),
            session_valid_for_seconds: 604800,
        }
    }

    fn operation(wallet: &str) -> NewPaidOperation {
        let visibility = Visibility::Private;
        NewPaidOperation {
            binding: new_binding(
                "op-1",
                "signer",
                wallet,
                "abc",
                1000,
                &quote(),
                OperationScope::new(Some("workspace-a"), visibility, "manual"),
            ),
            signer_pubkey: "signer".into(),
            cose_signed_bytes: vec![1, 2, 3],
            content: "hello".into(),
            tags: vec!["checkpoint".into()],
            embedding: vec![0.25, -0.5],
            workspace: Some("workspace-a".into()),
            visibility,
        }
    }

    fn receipt_for(operation: &NewPaidOperation, scheme: &str) -> PaymentReceipt {
        PaymentReceipt {
            operation_id: operation.binding.operation_id.clone(),
            scheme: scheme.into(),
            status: "settled".into(),
            binding_digest: binding_digest(&operation.binding).unwrap(),
            payer_wallet: operation.binding.payer_wallet.clone(),
            amount: operation.binding.amount.clone(),
            asset: operation.binding.asset.clone(),
            network: operation.binding.network.clone(),
            pay_to: operation.binding.pay_to.clone(),
            settlement_tx: Some("0x123".into()),
            settled_at: Utc::now().to_rfc3339(),
            receipt: serde_json::json!({"signature":"provider-signature"}),
        }
    }

    #[test]
    fn operation_is_restart_safe_and_idempotent() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("0x0000000000000000000000000000000000000003");
        let first = create_or_read(&store, &op).unwrap();
        let second = create_or_read(&store, &op).unwrap();
        assert_eq!(first.binding, second.binding);
        assert_eq!(second.embedding, vec![0.25, -0.5]);
        assert_eq!(second.state, "awaiting_payment");
    }

    #[test]
    fn provider_urls_require_tls_except_on_loopback() {
        assert!(validate_secure_http_url("https://pay.example/api").is_ok());
        assert!(validate_secure_http_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_secure_http_url("http://localhost:8080").is_ok());
        assert!(validate_secure_http_url("http://pay.example/api").is_err());
        assert!(validate_secure_http_url("https://user:secret@pay.example").is_err());
    }

    #[test]
    fn signed_operation_and_scope_survive_database_reopen() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let op = operation("0x0000000000000000000000000000000000000003");
        {
            let store = SqliteStore::open(file.path()).unwrap();
            create_or_read(&store, &op).unwrap();
        }
        let reopened = SqliteStore::open(file.path()).unwrap();
        let loaded = read(&reopened, "op-1").unwrap().unwrap();
        assert_eq!(loaded.cose_signed_bytes, op.cose_signed_bytes);
        assert_eq!(loaded.binding.scope, op.binding.scope);
        assert_eq!(loaded.workspace, op.workspace);
        assert_eq!(loaded.embedding, op.embedding);
    }

    #[test]
    fn immutable_binding_rejects_operation_id_reuse() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("0x0000000000000000000000000000000000000003");
        create_or_read(&store, &op).unwrap();
        let mut changed = op.clone();
        changed.binding.artifact_hash = "different".into();
        assert!(create_or_read(&store, &changed)
            .unwrap_err()
            .to_string()
            .contains("binding mismatch"));
    }

    #[test]
    fn settlement_and_anchor_claims_are_single_winner() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("0x0000000000000000000000000000000000000003");
        create_or_read(&store, &op).unwrap();
        let payment = PaymentAuthorization::Stake {
            session_id: "s-1".into(),
            payer_wallet: op.binding.payer_wallet.clone(),
            authorization: serde_json::json!({"signature":"0xabc"}),
        };
        assert!(claim_payment(&store, "op-1", &payment).unwrap());
        assert!(!claim_payment(&store, "op-1", &payment).unwrap());
        let receipt = receipt_for(&op, "stake");
        record_receipt(&store, "op-1", &payment, &receipt).unwrap();
        record_receipt(&store, "op-1", &payment, &receipt).unwrap();
        assert!(claim_anchoring(&store, "op-1").unwrap());
        assert!(!claim_anchoring(&store, "op-1").unwrap());
        mark_verifying(&store, "op-1").unwrap();
        mark_anchored(&store, "op-1", "a-1", "sol", "ar").unwrap();
        let loaded = read(&store, "op-1").unwrap().unwrap();
        assert_eq!(loaded.state, "anchored");
        assert_eq!(loaded.receipt, Some(receipt));
    }

    #[test]
    fn payer_wallet_is_bound_once() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("");
        create_or_read(&store, &op).unwrap();
        let wallet = "0x0000000000000000000000000000000000000003";
        let bound = bind_payer_wallet(&store, "op-1", wallet).unwrap();
        assert_eq!(bound.binding.payer_wallet, wallet);
        bind_payer_wallet(&store, "op-1", wallet).unwrap();
        assert!(
            bind_payer_wallet(&store, "op-1", "0x0000000000000000000000000000000000000004")
                .is_err()
        );
    }

    #[test]
    fn expired_unpaid_quote_refreshes_without_changing_artifact_scope() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("");
        create_or_read(&store, &op).unwrap();
        store
            .conn()
            .execute(
                "UPDATE paid_operations SET expires_at='2000-01-01T00:00:00Z' WHERE operation_id='op-1'",
                [],
            )
            .unwrap();
        let old = read(&store, "op-1").unwrap().unwrap();
        let refreshed = refresh_unpaid_quote(&store, "op-1", 2500).unwrap();
        assert_ne!(old.binding.nonce, refreshed.binding.nonce);
        assert_eq!(refreshed.binding.amount, "2500");
        assert_eq!(refreshed.binding.artifact_hash, old.binding.artifact_hash);
        assert_eq!(refreshed.binding.scope, old.binding.scope);
        assert!(!quote_expired(&refreshed.binding));
    }

    #[test]
    fn partial_chain_progress_survives_retry_and_rejects_conflicts() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("0x0000000000000000000000000000000000000003");
        create_or_read(&store, &op).unwrap();
        let payment = PaymentAuthorization::Stake {
            session_id: "s-1".into(),
            payer_wallet: op.binding.payer_wallet.clone(),
            authorization: serde_json::json!({"signature":"0xabc"}),
        };
        claim_payment(&store, "op-1", &payment).unwrap();
        record_receipt(&store, "op-1", &payment, &receipt_for(&op, "stake")).unwrap();
        assert!(claim_anchoring(&store, "op-1").unwrap());
        record_arweave(&store, "op-1", "ar-1").unwrap();
        mark_delivery_retryable(&store, "op-1", "solana unavailable").unwrap();
        assert!(claim_anchoring(&store, "op-1").unwrap());
        let recovered = read(&store, "op-1").unwrap().unwrap();
        assert_eq!(recovered.arweave_tx.as_deref(), Some("ar-1"));
        record_arweave(&store, "op-1", "ar-1").unwrap();
        assert!(record_arweave(&store, "op-1", "ar-2").is_err());
        record_solana(&store, "op-1", "sol-1").unwrap();
        mark_verifying(&store, "op-1").unwrap();
        mark_anchored(&store, "op-1", "op-1", "sol-1", "ar-1").unwrap();
    }

    #[test]
    fn stale_claims_recover_to_retryable_states() {
        let store = SqliteStore::in_memory().unwrap();
        let op = operation("0x0000000000000000000000000000000000000003");
        create_or_read(&store, &op).unwrap();
        store
            .conn()
            .execute(
                "UPDATE paid_operations SET state='payment_authorizing',updated_at=datetime('now','-10 minutes') WHERE operation_id='op-1'",
                [],
            )
            .unwrap();
        assert_eq!(
            read(&store, "op-1").unwrap().unwrap().state,
            "payment_failed"
        );
        store
            .conn()
            .execute(
                "UPDATE paid_operations SET state='anchoring',updated_at=datetime('now','-10 minutes') WHERE operation_id='op-1'",
                [],
            )
            .unwrap();
        assert_eq!(
            read(&store, "op-1").unwrap().unwrap().state,
            "delivery_retryable"
        );
    }

    #[tokio::test]
    async fn universal_client_uses_versioned_settle_and_status_contract() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let op = operation("0x0000000000000000000000000000000000000003");
        let payment = PaymentAuthorization::Stake {
            session_id: "session-1".into(),
            payer_wallet: op.binding.payer_wallet.clone(),
            authorization: serde_json::json!({"signature":"0xabc"}),
        };
        let expected = receipt_for(&op, "stake");
        let settle = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/payments/settle")
                .header("x-api-key", "service-secret");
            then.status(200).json_body_obj(&expected);
        });
        let status_receipt = expected.clone();
        let status = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/payments/op-1")
                .header("x-api-key", "service-secret");
            then.status(200).json_body_obj(&ProviderPaymentStatus {
                operation_id: "op-1".into(),
                status: "settled".into(),
                receipt: Some(status_receipt),
            });
        });
        let client = UniversalPaywallClient::new(&server.base_url(), "service-secret").unwrap();
        let request = SettleRequest {
            binding: op.binding,
            payment,
        };
        assert_eq!(client.settle(&request).await.unwrap(), expected);
        assert_eq!(client.status("op-1").await.unwrap().status, "settled");
        settle.assert();
        status.assert();
    }
}

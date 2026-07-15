//! Durable paid-anchoring operation metadata.
//!
//! This table deliberately stores correlation, binding, and receipt metadata
//! only. Canonical artifact bytes remain in the existing signing/delivery
//! pipeline and are never copied into payment state.

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// Idempotent server-side migration. The paid-operation table is independent
/// of attestations so existing anchored-memory recall never depends on it.
pub const MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS paid_operations (
    operation_id TEXT PRIMARY KEY,
    subject_hash TEXT NOT NULL,
    payer_wallet TEXT,
    artifact_hash TEXT NOT NULL,
    binding_digest TEXT,
    quote_id TEXT,
    quote_expires_at TEXT,
    state TEXT NOT NULL,
    provider_receipt_json TEXT,
    delivery_receipt_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_paid_operations_subject ON paid_operations(subject_hash);
CREATE INDEX IF NOT EXISTS idx_paid_operations_state ON paid_operations(state);";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaidOperationState {
    AwaitingSignature,
    AwaitingPayment,
    PaymentAuthorizing,
    PaymentReady,
    Anchoring,
    VerifyingDelivery,
    Anchored,
    PaymentRejected,
    QuoteExpired,
    PaymentFailed,
    DeliveryRetryable,
    RefundPending,
    Abandoned,
}

impl PaidOperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingSignature => "awaiting_signature",
            Self::AwaitingPayment => "awaiting_payment",
            Self::PaymentAuthorizing => "payment_authorizing",
            Self::PaymentReady => "payment_ready",
            Self::Anchoring => "anchoring",
            Self::VerifyingDelivery => "verifying_delivery",
            Self::Anchored => "anchored",
            Self::PaymentRejected => "payment_rejected",
            Self::QuoteExpired => "quote_expired",
            Self::PaymentFailed => "payment_failed",
            Self::DeliveryRetryable => "delivery_retryable",
            Self::RefundPending => "refund_pending",
            Self::Abandoned => "abandoned",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "awaiting_signature" => Ok(Self::AwaitingSignature),
            "awaiting_payment" => Ok(Self::AwaitingPayment),
            "payment_authorizing" => Ok(Self::PaymentAuthorizing),
            "payment_ready" => Ok(Self::PaymentReady),
            "anchoring" => Ok(Self::Anchoring),
            "verifying_delivery" => Ok(Self::VerifyingDelivery),
            "anchored" => Ok(Self::Anchored),
            "payment_rejected" => Ok(Self::PaymentRejected),
            "quote_expired" => Ok(Self::QuoteExpired),
            "payment_failed" => Ok(Self::PaymentFailed),
            "delivery_retryable" => Ok(Self::DeliveryRetryable),
            "refund_pending" => Ok(Self::RefundPending),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(anyhow!("invalid paid operation state in database")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaidOperation {
    pub operation_id: String,
    pub subject_hash: String,
    pub payer_wallet: Option<String>,
    pub artifact_hash: String,
    pub binding_digest: Option<String>,
    pub quote_id: Option<String>,
    pub quote_expires_at: Option<String>,
    pub state: PaidOperationState,
    pub provider_receipt_json: Option<String>,
    pub delivery_receipt_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct NewPaidOperation<'a> {
    pub operation_id: &'a str,
    pub subject_hash: &'a str,
    pub artifact_hash: &'a str,
    pub created_at: &'a str,
}

pub fn migrate_paid_operations(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_SQL)
        .context("create paid_operations table")
}

pub fn create_or_get(conn: &Connection, input: NewPaidOperation<'_>) -> Result<PaidOperation> {
    if input.operation_id.is_empty()
        || input.subject_hash.is_empty()
        || input.artifact_hash.is_empty()
    {
        return Err(anyhow!(
            "paid operation requires operation_id, subject_hash, and artifact_hash"
        ));
    }

    conn.execute(
        "INSERT INTO paid_operations \
         (operation_id, subject_hash, artifact_hash, state, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
         ON CONFLICT(operation_id) DO NOTHING",
        params![
            input.operation_id,
            input.subject_hash,
            input.artifact_hash,
            PaidOperationState::AwaitingSignature.as_str(),
            input.created_at,
        ],
    )
    .context("create paid operation")?;

    let operation =
        get(conn, input.operation_id)?.ok_or_else(|| anyhow!("paid operation disappeared"))?;
    if operation.subject_hash != input.subject_hash
        || operation.artifact_hash != input.artifact_hash
    {
        return Err(anyhow!("operation_id_conflict"));
    }
    Ok(operation)
}

pub fn get(conn: &Connection, operation_id: &str) -> Result<Option<PaidOperation>> {
    conn.query_row(
        "SELECT operation_id, subject_hash, payer_wallet, artifact_hash, binding_digest, quote_id, \
                quote_expires_at, state, provider_receipt_json, delivery_receipt_json, created_at, updated_at \
         FROM paid_operations WHERE operation_id = ?1",
        params![operation_id],
        |row| {
            let state: String = row.get(7)?;
            Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
                row.get(6)?, state, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?,
            ))
        },
    )
    .optional()
    .context("read paid operation")?
    .map(|row: (String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String, Option<String>, Option<String>, String, String)| {
        Ok(PaidOperation {
            operation_id: row.0,
            subject_hash: row.1,
            payer_wallet: row.2,
            artifact_hash: row.3,
            binding_digest: row.4,
            quote_id: row.5,
            quote_expires_at: row.6,
            state: PaidOperationState::parse(&row.7)?,
            provider_receipt_json: row.8,
            delivery_receipt_json: row.9,
            created_at: row.10,
            updated_at: row.11,
        })
    })
    .transpose()
}

/// Move an operation only from an expected state. This makes a duplicate
/// browser callback or recovery worker observe a conflict instead of silently
/// overwriting a newer state.
pub fn transition(
    conn: &Connection,
    operation_id: &str,
    expected: PaidOperationState,
    next: PaidOperationState,
    updated_at: &str,
) -> Result<PaidOperation> {
    let changed = conn
        .execute(
            "UPDATE paid_operations SET state = ?1, updated_at = ?2 \
             WHERE operation_id = ?3 AND state = ?4",
            params![next.as_str(), updated_at, operation_id, expected.as_str()],
        )
        .context("transition paid operation")?;
    if changed != 1 {
        return Err(anyhow!("paid_operation_state_conflict"));
    }
    get(conn, operation_id)?.ok_or_else(|| anyhow!("paid operation disappeared"))
}

pub fn record_quote(
    conn: &Connection,
    operation_id: &str,
    payer_wallet: &str,
    binding_digest: &str,
    quote_id: &str,
    quote_expires_at: &str,
    updated_at: &str,
) -> Result<PaidOperation> {
    let changed = conn
        .execute(
            "UPDATE paid_operations SET payer_wallet = ?1, binding_digest = ?2, quote_id = ?3, \
             quote_expires_at = ?4, state = ?5, updated_at = ?6 \
             WHERE operation_id = ?7 AND state IN ('awaiting_signature', 'awaiting_payment', 'quote_expired')",
            params![
                payer_wallet,
                binding_digest,
                quote_id,
                quote_expires_at,
                PaidOperationState::AwaitingPayment.as_str(),
                updated_at,
                operation_id,
            ],
        )
        .context("record paid operation quote")?;
    if changed != 1 {
        return Err(anyhow!("paid_operation_state_conflict"));
    }
    get(conn, operation_id)?.ok_or_else(|| anyhow!("paid operation disappeared"))
}

pub fn mark_payment_authorizing(
    conn: &Connection,
    operation_id: &str,
    updated_at: &str,
) -> Result<PaidOperation> {
    let changed = conn
        .execute(
            "UPDATE paid_operations SET state = ?1, updated_at = ?2 \
             WHERE operation_id = ?3 AND state IN ('awaiting_payment', 'payment_authorizing')",
            params![
                PaidOperationState::PaymentAuthorizing.as_str(),
                updated_at,
                operation_id,
            ],
        )
        .context("mark payment authorizing")?;
    if changed != 1 {
        return Err(anyhow!("paid_operation_state_conflict"));
    }
    get(conn, operation_id)?.ok_or_else(|| anyhow!("paid operation disappeared"))
}

pub fn record_provider_receipt(
    conn: &Connection,
    operation_id: &str,
    receipt_json: &str,
    updated_at: &str,
) -> Result<PaidOperation> {
    let changed = conn
        .execute(
            "UPDATE paid_operations SET provider_receipt_json = ?1, state = ?2, updated_at = ?3 \
             WHERE operation_id = ?4 AND state IN ('awaiting_payment', 'payment_authorizing')",
            params![
                receipt_json,
                PaidOperationState::PaymentReady.as_str(),
                updated_at,
                operation_id,
            ],
        )
        .context("record provider receipt")?;
    if changed != 1 {
        return Err(anyhow!("paid_operation_state_conflict"));
    }
    get(conn, operation_id)?.ok_or_else(|| anyhow!("paid operation disappeared"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn migration_and_idempotent_create_preserve_private_artifacts_outside_payment_state() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_operations(&conn).unwrap();
        migrate_paid_operations(&conn).unwrap();

        let first = create_or_get(
            &conn,
            NewPaidOperation {
                operation_id: "op-1",
                subject_hash: "subject-hash",
                artifact_hash: "blake3-signed-artifact",
                created_at: "2026-07-15T00:00:00Z",
            },
        )
        .unwrap();
        let second = create_or_get(
            &conn,
            NewPaidOperation {
                operation_id: "op-1",
                subject_hash: "subject-hash",
                artifact_hash: "blake3-signed-artifact",
                created_at: "2026-07-15T00:00:01Z",
            },
        )
        .unwrap();

        assert_eq!(first, second);
        assert_eq!(first.state, PaidOperationState::AwaitingSignature);
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'paid_operations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!sql.contains("artifact_content"));
        assert!(!sql.contains("canonical_cbor"));
    }

    #[test]
    fn operation_id_cannot_be_rebound_and_transitions_are_compare_and_set() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_operations(&conn).unwrap();
        create_or_get(
            &conn,
            NewPaidOperation {
                operation_id: "op-1",
                subject_hash: "subject-hash",
                artifact_hash: "hash-a",
                created_at: "2026-07-15T00:00:00Z",
            },
        )
        .unwrap();

        assert!(create_or_get(
            &conn,
            NewPaidOperation {
                operation_id: "op-1",
                subject_hash: "subject-hash",
                artifact_hash: "hash-b",
                created_at: "2026-07-15T00:00:00Z",
            },
        )
        .unwrap_err()
        .to_string()
        .contains("operation_id_conflict"));

        let transitioned = transition(
            &conn,
            "op-1",
            PaidOperationState::AwaitingSignature,
            PaidOperationState::AwaitingPayment,
            "2026-07-15T00:01:00Z",
        )
        .unwrap();
        assert_eq!(transitioned.state, PaidOperationState::AwaitingPayment);
        assert!(transition(
            &conn,
            "op-1",
            PaidOperationState::AwaitingSignature,
            PaidOperationState::PaymentReady,
            "2026-07-15T00:02:00Z",
        )
        .unwrap_err()
        .to_string()
        .contains("paid_operation_state_conflict"));
    }

    #[test]
    fn quote_and_receipt_are_durable_operation_metadata() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_operations(&conn).unwrap();
        create_or_get(
            &conn,
            NewPaidOperation {
                operation_id: "op-1",
                subject_hash: "subject-hash",
                artifact_hash: "hash-a",
                created_at: "2026-07-15T00:00:00Z",
            },
        )
        .unwrap();
        record_quote(
            &conn,
            "op-1",
            "0x1111111111111111111111111111111111111111",
            "0xdigest",
            "q_1",
            "2026-07-15T00:05:00Z",
            "2026-07-15T00:00:01Z",
        )
        .unwrap();
        mark_payment_authorizing(&conn, "op-1", "2026-07-15T00:00:02Z").unwrap();
        let settled = record_provider_receipt(
            &conn,
            "op-1",
            r#"{\"status\":\"settled\"}"#,
            "2026-07-15T00:00:03Z",
        )
        .unwrap();
        assert_eq!(settled.state, PaidOperationState::PaymentReady);
        assert_eq!(settled.quote_id.as_deref(), Some("q_1"));
        assert_eq!(
            settled.provider_receipt_json.as_deref(),
            Some(r#"{\"status\":\"settled\"}"#)
        );
        assert!(record_provider_receipt(
            &conn,
            "op-1",
            r#"{\"status\":\"different\"}"#,
            "2026-07-15T00:00:04Z",
        )
        .unwrap_err()
        .to_string()
        .contains("paid_operation_state_conflict"));
    }
}

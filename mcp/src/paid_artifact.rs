//! Versioned binding and durable staging for a paid, client-signed artifact.
//!
//! A payment must commit to the exact COSE_Sign1 envelope that the client
//! produced, rather than to editor content or to an unsigned CBOR payload.
//! The domain separator makes this hash unambiguous and leaves room for a
//! future envelope format without changing the meaning of existing receipts.
//!
//! Staging is deliberately separate from `paid_operations`: payment metadata
//! must not acquire artifact bytes. The enclosing SQLite store already holds
//! private attestation content under Mnemonic's at-rest access model; staging
//! uses that same local trust boundary until the artifact is anchored or
//! abandoned.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use mnemonic_core::storage::WriteMode;
use rusqlite::{params, Connection, OptionalExtension};

use crate::pending::PendingEntry;

/// Current paid-artifact binding format.
pub const PAID_ARTIFACT_BINDING_VERSION: u8 = 1;

const DOMAIN_SEPARATOR: &[u8] = b"mnemonic:paid-artifact:v1\0";

const MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS paid_artifact_staging (
    correlation_id TEXT PRIMARY KEY,
    signer_pubkey TEXT NOT NULL,
    artifact_hash TEXT NOT NULL,
    cose_sign1 BLOB NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_paid_artifact_staging_hash
    ON paid_artifact_staging(artifact_hash);
CREATE TABLE IF NOT EXISTS paid_artifact_delivery_context (
    correlation_id TEXT PRIMARY KEY,
    signer_pubkey TEXT NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,
    content_hash TEXT NOT NULL,
    canonical_cbor BLOB NOT NULL,
    tags_json TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    write_mode TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(correlation_id) REFERENCES paid_artifact_staging(correlation_id)
);
CREATE TABLE IF NOT EXISTS paid_artifact_delivery_claims (
    correlation_id TEXT PRIMARY KEY,
    claimed_at TEXT NOT NULL,
    FOREIGN KEY(correlation_id) REFERENCES paid_artifact_delivery_context(correlation_id)
);
CREATE TABLE IF NOT EXISTS paid_artifact_delivery_attempts (
    correlation_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    arweave_tx TEXT,
    solana_tx TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    lease_id TEXT,
    lease_expires_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(correlation_id) REFERENCES paid_artifact_delivery_context(correlation_id)
);";

/// A verified signed envelope held while payment completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPaidArtifact {
    pub correlation_id: String,
    pub signer_pubkey: String,
    pub artifact_hash: String,
    pub cose_sign1: Vec<u8>,
    pub created_at: String,
    pub updated_at: String,
}

/// The private local context required to perform delivery after payment.
/// This is separate from payment metadata and retained only under Mnemonic's
/// existing SQLite at-rest trust boundary.
#[derive(Debug, Clone)]
pub struct StagedDeliveryContext {
    pub correlation_id: String,
    pub signer_pubkey: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub content_hash: String,
    pub canonical_cbor: Vec<u8>,
    pub tags: Vec<String>,
    pub metadata: serde_json::Value,
    pub write_mode: WriteMode,
    pub exp: DateTime<Utc>,
}

impl StagedDeliveryContext {
    pub fn into_pending_entry(self) -> PendingEntry {
        PendingEntry {
            jwt_sub: self.signer_pubkey,
            content: self.content,
            embedding: self.embedding,
            content_hash: self.content_hash,
            canonical_cbor: self.canonical_cbor,
            tags: self.tags,
            metadata: self.metadata,
            write_mode: self.write_mode,
            exp: self.exp,
        }
    }
}

/// Create the independent artifact-staging table.
pub fn migrate_paid_artifact_staging(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_SQL)
        .context("create paid artifact staging table")
}

/// Persist a verified client-signed envelope exactly once.
///
/// A reused correlation id is allowed only when it refers to the identical
/// signer and signed envelope. This prevents a second callback from swapping
/// the artifact after a quote has been created.
pub fn stage_verified_cose(
    conn: &Connection,
    correlation_id: &str,
    signer_pubkey: &str,
    cose_sign1: &[u8],
    now: &str,
) -> Result<StagedPaidArtifact> {
    if correlation_id.is_empty() || signer_pubkey.is_empty() || cose_sign1.is_empty() {
        return Err(anyhow!(
            "staged paid artifact requires correlation_id, signer_pubkey, and COSE bytes"
        ));
    }
    let artifact_hash = hash_client_signed_cose(cose_sign1);
    conn.execute(
        "INSERT INTO paid_artifact_staging \
         (correlation_id, signer_pubkey, artifact_hash, cose_sign1, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
         ON CONFLICT(correlation_id) DO NOTHING",
        params![
            correlation_id,
            signer_pubkey,
            artifact_hash,
            cose_sign1,
            now
        ],
    )
    .context("stage verified paid artifact")?;

    let staged = get_staged_cose(conn, correlation_id)?
        .ok_or_else(|| anyhow!("staged paid artifact disappeared"))?;
    if staged.signer_pubkey != signer_pubkey
        || staged.artifact_hash != artifact_hash
        || staged.cose_sign1 != cose_sign1
    {
        return Err(anyhow!("paid_artifact_correlation_conflict"));
    }
    Ok(staged)
}

pub fn get_staged_cose(
    conn: &Connection,
    correlation_id: &str,
) -> Result<Option<StagedPaidArtifact>> {
    conn.query_row(
        "SELECT correlation_id, signer_pubkey, artifact_hash, cose_sign1, created_at, updated_at \
         FROM paid_artifact_staging WHERE correlation_id = ?1",
        params![correlation_id],
        |row| {
            Ok(StagedPaidArtifact {
                correlation_id: row.get(0)?,
                signer_pubkey: row.get(1)?,
                artifact_hash: row.get(2)?,
                cose_sign1: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .optional()
    .context("read staged paid artifact")
}

/// Persist the entire delivery context at the same point as the verified
/// signature. This allows a restarted MCP to resume from the same signed
/// artifact rather than requesting a new quote or recomputing an embedding.
pub fn stage_delivery_context(
    conn: &Connection,
    correlation_id: &str,
    entry: &PendingEntry,
    now: &str,
) -> Result<()> {
    let tags_json = serde_json::to_string(&entry.tags).context("serialize staged tags")?;
    let metadata_json =
        serde_json::to_string(&entry.metadata).context("serialize staged metadata")?;
    let embedding = encode_embedding(&entry.embedding);
    conn.execute(
        "INSERT INTO paid_artifact_delivery_context \
         (correlation_id, signer_pubkey, content, embedding, content_hash, canonical_cbor, tags_json, metadata_json, write_mode, expires_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11) \
         ON CONFLICT(correlation_id) DO NOTHING",
        params![
            correlation_id,
            entry.jwt_sub,
            entry.content,
            embedding,
            entry.content_hash,
            entry.canonical_cbor,
            tags_json,
            metadata_json,
            entry.write_mode.as_str(),
            entry.exp.to_rfc3339(),
            now,
        ],
    )
    .context("stage paid artifact delivery context")?;

    let staged = get_staged_delivery_context(conn, correlation_id)?
        .ok_or_else(|| anyhow!("staged delivery context disappeared"))?;
    if staged.signer_pubkey != entry.jwt_sub
        || staged.content_hash != entry.content_hash
        || staged.canonical_cbor != entry.canonical_cbor
        || staged.write_mode != entry.write_mode
    {
        return Err(anyhow!("paid_artifact_context_conflict"));
    }
    Ok(())
}

pub fn get_staged_delivery_context(
    conn: &Connection,
    correlation_id: &str,
) -> Result<Option<StagedDeliveryContext>> {
    conn.query_row(
        "SELECT correlation_id, signer_pubkey, content, embedding, content_hash, canonical_cbor, tags_json, metadata_json, write_mode, expires_at \
         FROM paid_artifact_delivery_context WHERE correlation_id = ?1",
        params![correlation_id],
        |row| {
            let embedding: Vec<u8> = row.get(3)?;
            let tags_json: String = row.get(6)?;
            let metadata_json: String = row.get(7)?;
            let write_mode: String = row.get(8)?;
            let exp: String = row.get(9)?;
            Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, embedding, row.get(4)?, row.get(5)?,
                tags_json, metadata_json, write_mode, exp,
            ))
        },
    )
    .optional()
    .context("read staged delivery context")?
    .map(|row: (String, String, String, Vec<u8>, String, Vec<u8>, String, String, String, String)| {
        Ok(StagedDeliveryContext {
            correlation_id: row.0,
            signer_pubkey: row.1,
            content: row.2,
            embedding: decode_embedding(&row.3)?,
            content_hash: row.4,
            canonical_cbor: row.5,
            tags: serde_json::from_str(&row.6).context("parse staged tags")?,
            metadata: serde_json::from_str(&row.7).context("parse staged metadata")?,
            write_mode: WriteMode::from_str_strict(&row.8)
                .ok_or_else(|| anyhow!("invalid staged write mode"))?,
            exp: DateTime::parse_from_rfc3339(&row.9)
                .context("parse staged expiry")?
                .with_timezone(&Utc),
        })
    })
    .transpose()
}

/// Atomically claim a staged context for its single anchoring attempt.
/// A second callback (including one after a restart) must not create a second
/// Arweave/Solana delivery for the same paid operation.
pub fn claim_delivery_context(conn: &Connection, correlation_id: &str, now: &str) -> Result<bool> {
    let changed = conn
        .execute(
            "INSERT INTO paid_artifact_delivery_claims (correlation_id, claimed_at) VALUES (?1, ?2) \
             ON CONFLICT(correlation_id) DO NOTHING",
            params![correlation_id, now],
        )
        .context("claim staged paid delivery context")?;
    Ok(changed == 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAttempt {
    pub correlation_id: String,
    pub state: String,
    pub arweave_tx: Option<String>,
    pub solana_tx: Option<String>,
    pub attempts: u32,
    pub lease_id: String,
}

/// Acquire a short lease for one paid delivery. The durable attempt records
/// partial progress, so a retry continues from a stored Arweave id rather
/// than uploading or charging for the same artifact again.
pub fn acquire_delivery_attempt(
    conn: &Connection,
    correlation_id: &str,
    lease_id: &str,
    now: &str,
    lease_expires_at: &str,
) -> Result<Option<DeliveryAttempt>> {
    let existing: Option<(String, Option<String>, Option<String>, u32, Option<String>)> = conn
        .query_row(
            "SELECT state, arweave_tx, solana_tx, attempts, lease_expires_at \
             FROM paid_artifact_delivery_attempts WHERE correlation_id = ?1",
            params![correlation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .context("read paid delivery attempt")?;
    if let Some((state, arweave_tx, solana_tx, attempts, existing_lease)) = existing {
        if state == "completed" || existing_lease.as_deref().is_some_and(|until| until > now) {
            return Ok(None);
        }
        conn.execute(
            "UPDATE paid_artifact_delivery_attempts SET state = ?1, attempts = ?2, lease_id = ?3, \
             lease_expires_at = ?4, updated_at = ?5 WHERE correlation_id = ?6",
            params![
                "anchoring",
                attempts + 1,
                lease_id,
                lease_expires_at,
                now,
                correlation_id
            ],
        )
        .context("reacquire paid delivery attempt")?;
        return Ok(Some(DeliveryAttempt {
            correlation_id: correlation_id.into(),
            state,
            arweave_tx,
            solana_tx,
            attempts: attempts + 1,
            lease_id: lease_id.into(),
        }));
    }
    conn.execute(
        "INSERT INTO paid_artifact_delivery_attempts \
         (correlation_id, state, attempts, lease_id, lease_expires_at, created_at, updated_at) \
         VALUES (?1, 'anchoring', 1, ?2, ?3, ?4, ?4)",
        params![correlation_id, lease_id, lease_expires_at, now],
    )
    .context("create paid delivery attempt")?;
    Ok(Some(DeliveryAttempt {
        correlation_id: correlation_id.into(),
        state: "anchoring".into(),
        arweave_tx: None,
        solana_tx: None,
        attempts: 1,
        lease_id: lease_id.into(),
    }))
}

pub fn record_arweave_uploaded(
    conn: &Connection,
    attempt: &DeliveryAttempt,
    arweave_tx: &str,
    now: &str,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE paid_artifact_delivery_attempts SET state = 'arweave_uploaded', arweave_tx = ?1, updated_at = ?2 \
         WHERE correlation_id = ?3 AND lease_id = ?4 AND arweave_tx IS NULL",
        params![arweave_tx, now, attempt.correlation_id, attempt.lease_id],
    ).context("record paid Arweave delivery")?;
    if changed != 1 {
        return Err(anyhow!("paid_delivery_lease_conflict"));
    }
    Ok(())
}

pub fn mark_delivery_retryable(
    conn: &Connection,
    attempt: &DeliveryAttempt,
    error: &str,
    now: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE paid_artifact_delivery_attempts SET state = 'delivery_retryable', lease_id = NULL, lease_expires_at = NULL, last_error = ?1, updated_at = ?2 \
         WHERE correlation_id = ?3 AND lease_id = ?4",
        params![error, now, attempt.correlation_id, attempt.lease_id],
    ).context("mark paid delivery retryable")?;
    Ok(())
}

pub fn mark_delivery_completed(
    conn: &Connection,
    attempt: &DeliveryAttempt,
    solana_tx: &str,
    now: &str,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE paid_artifact_delivery_attempts SET state = 'completed', solana_tx = ?1, lease_id = NULL, lease_expires_at = NULL, updated_at = ?2 \
         WHERE correlation_id = ?3 AND lease_id = ?4",
        params![solana_tx, now, attempt.correlation_id, attempt.lease_id],
    ).context("mark paid delivery completed")?;
    if changed != 1 {
        return Err(anyhow!("paid_delivery_lease_conflict"));
    }
    Ok(())
}

fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return Err(anyhow!("invalid staged embedding"));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte chunk")))
        .collect())
}

/// Return the canonical `artifact_hash` for an exact-payment binding.
///
/// `cose_sign1` must be the exact envelope returned by the client's signing
/// key. Hashing the envelope (rather than just its payload) commits to the
/// signature, protected headers, and signer key identifier as well as the
/// canonical artifact bytes.
pub fn hash_client_signed_cose(cose_sign1: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_SEPARATOR);
    hasher.update(cose_sign1);
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn binding_is_deterministic_and_domain_separated() {
        let cose = b"a client-signed cose envelope";
        assert_eq!(hash_client_signed_cose(cose), hash_client_signed_cose(cose));
        assert_ne!(
            hash_client_signed_cose(cose),
            blake3::hash(cose).to_hex().to_string()
        );
    }

    #[test]
    fn any_signed_envelope_change_invalidates_the_binding() {
        assert_ne!(
            hash_client_signed_cose(b"cose-envelope-a"),
            hash_client_signed_cose(b"cose-envelope-b")
        );
    }

    #[test]
    fn staged_envelope_is_immutable_for_a_correlation_id() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_artifact_staging(&conn).unwrap();
        let staged = stage_verified_cose(&conn, "correlation", "signer", b"cose-a", "now").unwrap();
        assert_eq!(staged.artifact_hash, hash_client_signed_cose(b"cose-a"));
        assert_eq!(
            stage_verified_cose(&conn, "correlation", "signer", b"cose-a", "later").unwrap(),
            staged
        );
        assert!(stage_verified_cose(&conn, "correlation", "signer", b"cose-b", "later").is_err());
    }

    #[test]
    fn delivery_context_round_trips_without_reembedding() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_artifact_staging(&conn).unwrap();
        let entry = PendingEntry {
            jwt_sub: "signer".into(),
            content: "private memory".into(),
            embedding: vec![0.5, -1.25],
            content_hash: "canonical-hash".into(),
            canonical_cbor: vec![1, 2, 3],
            tags: vec!["tag".into()],
            metadata: serde_json::json!({"turbo_bits": 4}),
            write_mode: WriteMode::Participate,
            exp: Utc::now(),
        };
        stage_verified_cose(&conn, "correlation", "signer", b"cose", "now").unwrap();
        stage_delivery_context(&conn, "correlation", &entry, "now").unwrap();
        let recovered = get_staged_delivery_context(&conn, "correlation")
            .unwrap()
            .unwrap()
            .into_pending_entry();
        assert_eq!(recovered.content, entry.content);
        assert_eq!(recovered.embedding, entry.embedding);
        assert_eq!(recovered.canonical_cbor, entry.canonical_cbor);
        assert_eq!(recovered.write_mode, WriteMode::Participate);
        assert!(claim_delivery_context(&conn, "correlation", "later").unwrap());
        assert!(!claim_delivery_context(&conn, "correlation", "again").unwrap());
    }

    #[test]
    fn delivery_retry_reuses_recorded_arweave_progress_without_a_new_claim() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_artifact_staging(&conn).unwrap();
        let entry = PendingEntry {
            jwt_sub: "signer".into(),
            content: "private memory".into(),
            embedding: vec![],
            content_hash: "hash".into(),
            canonical_cbor: vec![1],
            tags: vec![],
            metadata: serde_json::json!({}),
            write_mode: WriteMode::Participate,
            exp: Utc::now(),
        };
        stage_verified_cose(&conn, "correlation", "signer", b"cose", "now").unwrap();
        stage_delivery_context(&conn, "correlation", &entry, "now").unwrap();
        let first = acquire_delivery_attempt(
            &conn,
            "correlation",
            "lease-1",
            "2026-07-15T00:00:00Z",
            "2026-07-15T00:10:00Z",
        )
        .unwrap()
        .unwrap();
        record_arweave_uploaded(&conn, &first, "arweave-1", "2026-07-15T00:01:00Z").unwrap();
        mark_delivery_retryable(&conn, &first, "solana unavailable", "2026-07-15T00:02:00Z")
            .unwrap();
        let retry = acquire_delivery_attempt(
            &conn,
            "correlation",
            "lease-2",
            "2026-07-15T00:03:00Z",
            "2026-07-15T00:13:00Z",
        )
        .unwrap()
        .unwrap();
        assert_eq!(retry.arweave_tx.as_deref(), Some("arweave-1"));
        assert_eq!(retry.attempts, 2);
        mark_delivery_completed(&conn, &retry, "solana-1", "2026-07-15T00:04:00Z").unwrap();
        assert!(acquire_delivery_attempt(
            &conn,
            "correlation",
            "lease-3",
            "2026-07-15T00:05:00Z",
            "2026-07-15T00:15:00Z",
        )
        .unwrap()
        .is_none());
    }
}

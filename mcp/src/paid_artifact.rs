//! Versioned binding and byte-free retry metadata for paid client artifacts.
//!
//! A payment must commit to the exact COSE_Sign1 envelope that the client
//! produced, rather than to editor content or to an unsigned CBOR payload.
//! The domain separator makes this hash unambiguous and leaves room for a
//! future envelope format without changing the meaning of existing receipts.
//!
//! New records retain author and digest, never source bytes. Historical staging
//! remains untouched on boot until identical client resubmission drains it.
//! The client must retain the original envelope; autonomous upload retries stop.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use mnemonic_core::storage::WriteMode;
use rusqlite::{params, Connection, OptionalExtension};

use crate::pending::PendingEntry;

/// Column tuple for the staged-delivery SELECT. Named so the query and the
/// struct construction below stay legible.
type StagedDeliveryRow = (
    String,
    String,
    String,
    Vec<u8>,
    String,
    Vec<u8>,
    String,
    String,
    String,
    String,
);

/// Column tuple for the delivery-attempt SELECT.
type DeliveryAttemptRow = (String, Option<String>, Option<String>, u32, Option<String>);

/// Current paid-artifact binding format.
// Reserved for the in-flight paid-anchoring work; exercised by tests but not
// yet reached from the binary. Reworked by M3 (work/x402-v2-conformance).
#[allow(dead_code)]
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
    next_retry_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(correlation_id) REFERENCES paid_artifact_delivery_context(correlation_id)
);
CREATE TABLE IF NOT EXISTS paid_delivery_review_cases (
    correlation_id TEXT PRIMARY KEY,
    reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT NOT NULL,
    FOREIGN KEY(correlation_id) REFERENCES paid_artifact_delivery_attempts(correlation_id)
);";

/// Metadata binding; cose_sign1 is empty for all new records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPaidArtifact {
    pub correlation_id: String,
    pub signer_pubkey: String,
    pub artifact_hash: String,
    pub cose_sign1: Vec<u8>,
    pub created_at: String,
    pub updated_at: String,
}

/// Legacy-compatible metadata row. Payload fields are empty for new writes.
#[derive(Debug, Clone)]
pub struct StagedDeliveryContext {
    // Reserved for the in-flight paid-anchoring work; not yet read from the
    // binary. Reworked by M3 (work/x402-v2-conformance).
    #[allow(dead_code)]
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
            visibility: mnemonic_core::storage::Visibility::Private,
            is_sealed: false,
            free_quota: false,
            requester_ip: None,
            exp: self.exp,
        }
    }
}

/// Create the independent artifact-staging table.
pub fn migrate_paid_artifact_staging(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_SQL)
        .context("create paid artifact staging table")?;
    let mut statement = conn.prepare("PRAGMA table_info(paid_artifact_delivery_attempts)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == "next_retry_at") {
        conn.execute(
            "ALTER TABLE paid_artifact_delivery_attempts ADD COLUMN next_retry_at TEXT",
            [],
        )?;
    }
    Ok(())
}

/// Persist its verified author and envelope digest, never its source bytes.
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
            Vec::<u8>::new(),
            now
        ],
    )
    .context("stage verified paid artifact")?;

    let staged = get_staged_cose(conn, correlation_id)?
        .ok_or_else(|| anyhow!("staged paid artifact disappeared"))?;
    if staged.signer_pubkey != signer_pubkey || staged.artifact_hash != artifact_hash {
        return Err(anyhow!("paid_artifact_correlation_conflict"));
    }
    // Old deployments retained bytes. Drain only after the client resubmits
    // the identical verified envelope; boot migrations never discard them.
    if !staged.cose_sign1.is_empty() {
        conn.execute(
            "UPDATE paid_artifact_staging SET cose_sign1=X'' WHERE correlation_id=?1",
            [correlation_id],
        )?;
    }
    Ok(StagedPaidArtifact {
        cose_sign1: Vec::new(),
        ..staged
    })
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

/// Persist only the content hash, owner, write mode and expiry. Restarted
/// callbacks reconstruct transient context from identical client resubmission.
pub fn stage_delivery_context(
    conn: &Connection,
    correlation_id: &str,
    entry: &PendingEntry,
    now: &str,
) -> Result<()> {
    let tags_json = "[]";
    let metadata_json = "{}";
    let embedding: Vec<u8> = vec![];
    conn.execute(
        "INSERT INTO paid_artifact_delivery_context \
         (correlation_id, signer_pubkey, content, embedding, content_hash, canonical_cbor, tags_json, metadata_json, write_mode, expires_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11) \
         ON CONFLICT(correlation_id) DO NOTHING",
        params![
            correlation_id,
            entry.jwt_sub,
            "",
            embedding,
            entry.content_hash,
            Vec::<u8>::new(),
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
        || staged.write_mode != entry.write_mode
    {
        return Err(anyhow!("paid_artifact_context_conflict"));
    }
    conn.execute("UPDATE paid_artifact_delivery_context SET content='',embedding=X'',canonical_cbor=X'',tags_json='[]',metadata_json='{}' WHERE correlation_id=?1", [correlation_id])?;
    Ok(())
}

/// Metadata-only visibility into upgrade retention. No source bytes leave this API.
pub fn legacy_retained_count(conn: &Connection) -> Result<u64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM paid_artifact_staging s LEFT JOIN paid_artifact_delivery_context c USING(correlation_id) WHERE length(s.cose_sign1)>0 OR length(c.content)>0 OR length(c.embedding)>0 OR length(c.canonical_cbor)>0", [], |r| r.get(0))?)
}

/// Reconstruct transient context only from the client's identical signed bytes.
pub fn resubmitted_context(
    conn: &Connection,
    correlation_id: &str,
    bytes: &[u8],
    author: &str,
) -> Result<Option<PendingEntry>> {
    let Some(staged) = get_staged_cose(conn, correlation_id)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        staged.signer_pubkey == author && staged.artifact_hash == hash_client_signed_cose(bytes),
        "resubmission binding mismatch"
    );
    let Some(context) = get_staged_delivery_context(conn, correlation_id)? else {
        return Ok(None);
    };
    let verified = mnemonic_core::codec::sign::verify_artifact(bytes, Some(&context.content_hash))
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        verified.valid && verified.signer == author,
        "invalid resubmission signature"
    );
    let payload = mnemonic_core::codec::canonical::from_canonical_cbor(&verified.payload)
        .map_err(anyhow::Error::msg)?;
    let mut entry = context.into_pending_entry();
    entry.canonical_cbor = verified.payload;
    entry.content = payload["content"].as_str().unwrap_or("").into();
    entry.embedding = vec![];
    entry.tags = payload["tags"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    entry.metadata = payload["metadata"].clone();
    entry.is_sealed = payload["type"] == "sealed";
    entry.visibility = if payload["visibility"] == "public" {
        mnemonic_core::storage::Visibility::Public
    } else {
        mnemonic_core::storage::Visibility::Private
    };
    Ok(Some(entry))
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
    .map(|row: StagedDeliveryRow| {
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
// Reserved for the in-flight paid-anchoring work; exercised by tests but not
// yet reached from the binary. Reworked by M3 (work/x402-v2-conformance).
#[allow(dead_code)]
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
    let existing: Option<DeliveryAttemptRow> = conn
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
        // An abandoned delivery is terminal: only an explicit, audited operator
        // action may reopen it. In particular, a late browser callback must not
        // silently start another delivery after the bounded retry budget ends.
        if state == "completed"
            || state == "abandoned"
            || existing_lease.as_deref().is_some_and(|until| until > now)
        {
            return Ok(None);
        }
        let changed=conn.execute(
            "UPDATE paid_artifact_delivery_attempts SET state = ?1, attempts = ?2, lease_id = ?3, \
             lease_expires_at = ?4, updated_at = ?5 WHERE correlation_id = ?6 AND attempts = ?7 AND state NOT IN ('completed','abandoned') AND (lease_expires_at IS NULL OR lease_expires_at <= ?5)",
            params![
                "anchoring",
                attempts + 1,
                lease_id,
                lease_expires_at,
                now,
                correlation_id,
                attempts
            ],
        )
        .context("reacquire paid delivery attempt")?;
        if changed != 1 {
            return Ok(None);
        }
        return Ok(Some(DeliveryAttempt {
            correlation_id: correlation_id.into(),
            state,
            arweave_tx,
            solana_tx,
            attempts: attempts + 1,
            lease_id: lease_id.into(),
        }));
    }
    let changed = conn
        .execute(
            "INSERT INTO paid_artifact_delivery_attempts \
         (correlation_id, state, attempts, lease_id, lease_expires_at, created_at, updated_at) \
         VALUES (?1, 'anchoring', 1, ?2, ?3, ?4, ?4) ON CONFLICT(correlation_id) DO NOTHING",
            params![correlation_id, lease_id, lease_expires_at, now],
        )
        .context("create paid delivery attempt")?;
    if changed != 1 {
        return Ok(None);
    }
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
        "UPDATE paid_artifact_delivery_attempts SET state = 'verification_pending', arweave_tx = ?1, updated_at = ?2 \
         WHERE correlation_id = ?3 AND lease_id = ?4 AND (arweave_tx IS NULL OR arweave_tx = ?1)",
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
    let delay_secs = match attempt.attempts {
        1 => 60,
        2 => 120,
        3 => 240,
        4 => 480,
        5 => 960,
        6 => 1920,
        _ => 3600,
    };
    let retry_at = chrono::DateTime::parse_from_rfc3339(now)
        .context("parse delivery retry time")?
        .with_timezone(&Utc)
        .checked_add_signed(chrono::TimeDelta::seconds(delay_secs))
        .ok_or_else(|| anyhow!("delivery retry time overflow"))?
        .to_rfc3339();
    let terminal = attempt.attempts >= 8;
    conn.execute(
        "UPDATE paid_artifact_delivery_attempts SET state = ?1, lease_id = NULL, lease_expires_at = NULL, next_retry_at = ?2, last_error = ?3, updated_at = ?4 \
         WHERE correlation_id = ?5 AND lease_id = ?6",
        params![if terminal { "abandoned" } else { "delivery_retryable" }, if terminal { None } else { Some(retry_at) }, error, now, attempt.correlation_id, attempt.lease_id],
    ).context("mark paid delivery retryable")?;
    if terminal {
        conn.execute(
            "INSERT INTO paid_delivery_review_cases (correlation_id, reason, created_at) VALUES (?1, ?2, ?3) ON CONFLICT(correlation_id) DO NOTHING",
            params![attempt.correlation_id, error, now],
        ).context("open paid delivery review case")?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn due_delivery_retries(conn: &Connection, now: &str, limit: usize) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT correlation_id FROM paid_artifact_delivery_attempts \
         WHERE state = 'delivery_retryable' AND next_retry_at <= ?1 ORDER BY next_retry_at LIMIT ?2",
    )?;
    let retries = statement
        .query_map(params![now, limit as i64], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("list due delivery retries")?;
    Ok(retries)
}

/// Record that the Solana memo for a paid anchored delivery has been submitted
/// but not yet confirmed. Stage 2 (chain-agnostic/decisions.md) stopped writing
/// SPL Memos, so this function is no longer called on the new write path. It is
/// kept for forward compatibility: a future re-introduction of an anchor writer
/// can call it again without an ABI change. `allow(dead_code)` instead of
/// deletion because removing it would silently break any operator running a
/// mixed-version deployment that still has `solana_submitted` rows in flight.
#[allow(dead_code)]
pub fn record_solana_submitted(
    conn: &Connection,
    attempt: &DeliveryAttempt,
    solana_tx: &str,
    now: &str,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE paid_artifact_delivery_attempts SET state = 'solana_submitted', solana_tx = ?1, updated_at = ?2 \
         WHERE correlation_id = ?3 AND lease_id = ?4 AND solana_tx IS NULL",
        params![solana_tx, now, attempt.correlation_id, attempt.lease_id],
    ).context("record paid Solana submission")?;
    if changed != 1 {
        return Err(anyhow!("paid_delivery_lease_conflict"));
    }
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

#[allow(dead_code)]
fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return Err(anyhow!("invalid staged embedding"));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
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
    fn delivery_context_retains_only_identity_without_source_bytes() {
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
            write_mode: WriteMode::Anchored,
            visibility: mnemonic_core::storage::Visibility::Private,
            is_sealed: false,
            free_quota: false,
            requester_ip: None,
            exp: Utc::now(),
        };
        stage_verified_cose(&conn, "correlation", "signer", b"cose", "now").unwrap();
        stage_delivery_context(&conn, "correlation", &entry, "now").unwrap();
        let recovered = get_staged_delivery_context(&conn, "correlation")
            .unwrap()
            .unwrap()
            .into_pending_entry();
        assert!(recovered.content.is_empty());
        assert!(recovered.embedding.is_empty());
        assert!(recovered.canonical_cbor.is_empty());
        assert_eq!(recovered.write_mode, WriteMode::Anchored);
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
            write_mode: WriteMode::Anchored,
            visibility: mnemonic_core::storage::Visibility::Private,
            is_sealed: false,
            free_quota: false,
            requester_ip: None,
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

    #[test]
    fn retry_schedule_is_due_only_after_backoff_and_eighth_failure_is_abandoned() {
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
            write_mode: WriteMode::Anchored,
            visibility: mnemonic_core::storage::Visibility::Private,
            is_sealed: false,
            free_quota: false,
            requester_ip: None,
            exp: Utc::now(),
        };
        stage_verified_cose(&conn, "retry", "signer", b"cose", "now").unwrap();
        stage_delivery_context(&conn, "retry", &entry, "now").unwrap();
        let first = acquire_delivery_attempt(
            &conn,
            "retry",
            "lease-1",
            "2026-07-15T00:00:00Z",
            "2026-07-15T00:10:00Z",
        )
        .unwrap()
        .unwrap();
        mark_delivery_retryable(&conn, &first, "temporary", "2026-07-15T00:00:00Z").unwrap();
        assert!(due_delivery_retries(&conn, "2026-07-15T00:00:59Z", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            due_delivery_retries(&conn, "2026-07-15T00:01:00Z", 10).unwrap(),
            vec!["retry"]
        );
        for attempt_number in 2..=8 {
            let attempt = acquire_delivery_attempt(
                &conn,
                "retry",
                &format!("lease-{attempt_number}"),
                "2026-07-15T02:00:00Z",
                "2026-07-15T02:10:00Z",
            )
            .unwrap()
            .unwrap();
            mark_delivery_retryable(&conn, &attempt, "temporary", "2026-07-15T02:00:00Z").unwrap();
        }
        assert!(due_delivery_retries(&conn, "2026-07-15T23:00:00Z", 10)
            .unwrap()
            .is_empty());
        let state: String = conn
            .query_row(
                "SELECT state FROM paid_artifact_delivery_attempts WHERE correlation_id = 'retry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "abandoned");
        let review_reason: String = conn
            .query_row(
                "SELECT reason FROM paid_delivery_review_cases WHERE correlation_id = 'retry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(review_reason, "temporary");
        assert!(acquire_delivery_attempt(
            &conn,
            "retry",
            "late-callback",
            "2026-07-16T00:00:00Z",
            "2026-07-16T00:10:00Z",
        )
        .unwrap()
        .is_none());
    }

    /// T02: the `artifact_hash` written to `paid_operations` must equal the
    /// hash derived from the staged COSE envelope. Any change to the envelope
    /// (payload, signature, protected headers, key id) produces a different
    /// hash, so a quote is always bound to the exact client-signed artifact.
    #[test]
    fn staged_cose_hash_matches_paid_operation_artifact_hash() {
        use crate::paid_operation;

        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_artifact_staging(&conn).unwrap();
        paid_operation::migrate_paid_operations(&conn).unwrap();

        let cose_bytes = b"exact-client-signed-cose-envelope";
        let expected_hash = hash_client_signed_cose(cose_bytes);

        // Stage the COSE envelope.
        let staged = stage_verified_cose(
            &conn,
            "corr-1",
            "signer-1",
            cose_bytes,
            "2026-07-15T00:00:00Z",
        )
        .unwrap();
        assert_eq!(
            staged.artifact_hash, expected_hash,
            "staged artifact_hash must equal hash_client_signed_cose result"
        );

        // The paid_operation is created with the same artifact_hash, binding
        // the quote to the exact signed artifact.
        paid_operation::create_or_get(
            &conn,
            paid_operation::NewPaidOperation {
                operation_id: "corr-1",
                subject_hash: "subject-hash",
                artifact_hash: &staged.artifact_hash,
                created_at: "2026-07-15T00:00:00Z",
            },
        )
        .unwrap();

        let op = paid_operation::get(&conn, "corr-1").unwrap().unwrap();
        assert_eq!(
            op.artifact_hash, expected_hash,
            "paid_operation.artifact_hash must match staged COSE hash"
        );

        // A different COSE envelope produces a different hash; the operation
        // cannot be rebound to a different artifact.
        let other_cose = b"a-different-cose-envelope";
        let other_hash = hash_client_signed_cose(other_cose);
        assert_ne!(
            other_hash, expected_hash,
            "a different COSE envelope must produce a different artifact_hash"
        );
        assert!(
            paid_operation::create_or_get(
                &conn,
                paid_operation::NewPaidOperation {
                    operation_id: "corr-1",
                    subject_hash: "subject-hash",
                    artifact_hash: &other_hash,
                    created_at: "2026-07-15T00:00:01Z",
                },
            )
            .unwrap_err()
            .to_string()
            .contains("operation_id_conflict"),
            "a settled receipt cannot be reused for a different artifact"
        );
    }
}

#[cfg(test)]
mod resubmission_tests {
    use super::*;
    use solana_sdk::signature::{Keypair, Signer};
    #[test]
    fn restart_rebuilds_only_from_identical_client_bytes_and_drains_legacy_on_resubmission() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_paid_artifact_staging(&conn).unwrap();
        let kp = Keypair::new();
        let author = kp.pubkey().to_string();
        let payload = serde_json::json!({"artifact_id":"one","type":"memory","schema_version":1,"producer":format!("did:sol:{author}"),"content":"private payload","created_at":"2026-10-02T00:00:00Z","tags":["private-tag"]});
        let signed = mnemonic_core::codec::sign::sign_artifact(
            &payload,
            &mnemonic_core::codec::schema::MEMORY_V1,
            &kp,
        )
        .unwrap();
        let entry = PendingEntry {
            jwt_sub: author.clone(),
            content: "private payload".into(),
            embedding: vec![1.0],
            content_hash: signed.content_hash.clone(),
            canonical_cbor: signed.canonical_cbor.clone(),
            tags: vec!["private-tag".into()],
            metadata: serde_json::json!({}),
            write_mode: WriteMode::Anchored,
            visibility: mnemonic_core::storage::Visibility::Private,
            is_sealed: false,
            free_quota: false,
            requester_ip: None,
            exp: Utc::now(),
        };
        stage_verified_cose(&conn, "op", &author, &signed.cose_bytes, "now").unwrap();
        stage_delivery_context(&conn, "op", &entry, "now").unwrap();
        let retained:i64=conn.query_row("SELECT length(s.cose_sign1)+length(c.content)+length(c.embedding)+length(c.canonical_cbor) FROM paid_artifact_staging s JOIN paid_artifact_delivery_context c USING(correlation_id)",[],|r|r.get(0)).unwrap();
        assert_eq!(retained, 0);
        let recovered = resubmitted_context(&conn, "op", &signed.cose_bytes, &author)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.content, "private payload");
        assert_eq!(recovered.canonical_cbor, signed.canonical_cbor);
        assert!(resubmitted_context(&conn, "op", b"changed", &author).is_err());
        assert!(resubmitted_context(&conn, "op", &signed.cose_bytes, "other").is_err());
        // Simulate an old deployment. Boot preserves original pending bytes.
        conn.execute(
            "UPDATE paid_artifact_staging SET cose_sign1=?1",
            [&signed.cose_bytes],
        )
        .unwrap();
        conn.execute("UPDATE paid_artifact_delivery_context SET content='legacy private payload',canonical_cbor=?1",[&signed.canonical_cbor]).unwrap();
        migrate_paid_artifact_staging(&conn).unwrap();
        assert_eq!(legacy_retained_count(&conn).unwrap(), 1);
        assert!(stage_verified_cose(&conn, "op", &author, b"different", "later").is_err());
        assert_eq!(legacy_retained_count(&conn).unwrap(), 1);
        stage_verified_cose(&conn, "op", &author, &signed.cose_bytes, "later").unwrap();
        stage_delivery_context(&conn, "op", &entry, "later").unwrap();
        assert_eq!(legacy_retained_count(&conn).unwrap(), 0);
    }
}

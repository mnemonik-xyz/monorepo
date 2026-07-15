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
use rusqlite::{params, Connection, OptionalExtension};

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
    ON paid_artifact_staging(artifact_hash);";

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
}

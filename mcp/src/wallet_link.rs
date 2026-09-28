//! Fresh EIP-191 wallet links for paid Mnemonic operations.
//!
//! A quote may use a wallet only after it signs a short-lived challenge bound
//! to the authenticated Mnemonic subject and the exact operation id.

use std::str::FromStr;

use alloy_primitives::Signature;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "mnemonic-paywall-wallet-link/v1";

const MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS paid_wallet_links (
    operation_id TEXT PRIMARY KEY,
    subject_hash TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    nonce TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    wallet_address TEXT,
    signature TEXT,
    verified_at TEXT,
    created_at TEXT NOT NULL
);";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletLinkChallenge {
    pub service: String,
    pub subject_hash: String,
    pub operation_id: String,
    pub chain_id: u64,
    pub nonce: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedWalletLink {
    // Reserved for the in-flight paid-anchoring work; not yet read from the
    // binary. Reworked by M3 (work/x402-v2-conformance).
    #[allow(dead_code)]
    pub operation_id: String,
    pub subject_hash: String,
    pub chain_id: u64,
    pub wallet_address: String,
    // Reserved for the in-flight paid-anchoring work; not yet read from the
    // binary. Reworked by M3 (work/x402-v2-conformance).
    #[allow(dead_code)]
    pub verified_at: String,
}

pub fn migrate_wallet_links(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_SQL)
        .context("create paid wallet links table")
}

pub fn create_or_get_challenge(
    conn: &Connection,
    operation_id: &str,
    subject_hash: &str,
    chain_id: u64,
    now: DateTime<Utc>,
) -> Result<WalletLinkChallenge> {
    if operation_id.is_empty() || subject_hash.is_empty() || chain_id == 0 {
        return Err(anyhow!(
            "wallet link requires operation, subject, and chain"
        ));
    }
    if let Some(challenge) = get_challenge(conn, operation_id)? {
        if challenge.subject_hash != subject_hash || challenge.chain_id != chain_id {
            return Err(anyhow!("wallet_link_operation_conflict"));
        }
        return Ok(challenge);
    }
    let mut nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let challenge = WalletLinkChallenge {
        service: SERVICE.into(),
        subject_hash: subject_hash.into(),
        operation_id: operation_id.into(),
        chain_id,
        nonce: format!("0x{}", hex::encode(nonce)),
        expires_at: (now + chrono::TimeDelta::minutes(5)).to_rfc3339(),
    };
    conn.execute(
        "INSERT INTO paid_wallet_links (operation_id, subject_hash, chain_id, nonce, expires_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![operation_id, subject_hash, chain_id, challenge.nonce, challenge.expires_at, now.to_rfc3339()],
    )
    .context("create wallet link challenge")?;
    Ok(challenge)
}

pub fn challenge_message(challenge: &WalletLinkChallenge) -> String {
    format!(
        "Mnemonic wallet link\nservice: {}\nsubject_hash: {}\noperation_id: {}\nchain_id: {}\nnonce: {}\nexpires_at: {}",
        challenge.service,
        challenge.subject_hash,
        challenge.operation_id,
        challenge.chain_id,
        challenge.nonce,
        challenge.expires_at,
    )
}

pub fn verify_and_record(
    conn: &Connection,
    challenge: &WalletLinkChallenge,
    signature: &str,
    now: DateTime<Utc>,
) -> Result<VerifiedWalletLink> {
    if DateTime::parse_from_rfc3339(&challenge.expires_at)
        .context("parse wallet link expiry")?
        .with_timezone(&Utc)
        <= now
    {
        return Err(anyhow!("wallet_link_expired"));
    }
    let signature = Signature::from_str(signature).context("parse wallet link signature")?;
    let address = signature
        .recover_address_from_msg(challenge_message(challenge))
        .context("recover wallet link signer")?
        .to_string()
        .to_lowercase();
    let changed = conn.execute(
        "UPDATE paid_wallet_links SET wallet_address = ?1, signature = ?2, verified_at = ?3 \
         WHERE operation_id = ?4 AND subject_hash = ?5 AND chain_id = ?6 AND nonce = ?7 \
         AND expires_at = ?8 AND wallet_address IS NULL",
        params![
            address,
            signature.to_string(),
            now.to_rfc3339(),
            challenge.operation_id,
            challenge.subject_hash,
            challenge.chain_id,
            challenge.nonce,
            challenge.expires_at
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("wallet_link_already_used_or_mismatched"));
    }
    get_verified(conn, &challenge.operation_id)?
        .ok_or_else(|| anyhow!("verified wallet link disappeared"))
}

pub fn get_verified(conn: &Connection, operation_id: &str) -> Result<Option<VerifiedWalletLink>> {
    conn.query_row(
        "SELECT operation_id, subject_hash, chain_id, wallet_address, verified_at FROM paid_wallet_links \
         WHERE operation_id = ?1 AND wallet_address IS NOT NULL AND verified_at IS NOT NULL",
        params![operation_id],
        |row| Ok(VerifiedWalletLink { operation_id: row.get(0)?, subject_hash: row.get(1)?, chain_id: row.get(2)?, wallet_address: row.get(3)?, verified_at: row.get(4)? }),
    ).optional().context("read verified wallet link")
}

fn get_challenge(conn: &Connection, operation_id: &str) -> Result<Option<WalletLinkChallenge>> {
    conn.query_row(
        "SELECT subject_hash, operation_id, chain_id, nonce, expires_at FROM paid_wallet_links WHERE operation_id = ?1",
        params![operation_id],
        |row| Ok(WalletLinkChallenge { service: SERVICE.into(), subject_hash: row.get(0)?, operation_id: row.get(1)?, chain_id: row.get(2)?, nonce: row.get(3)?, expires_at: row.get(4)? }),
    ).optional().context("read wallet link challenge")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_stable_and_operation_bound() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_wallet_links(&conn).unwrap();
        let now = Utc::now();
        let first = create_or_get_challenge(&conn, "operation", "subject", 31_337, now).unwrap();
        let repeated = create_or_get_challenge(&conn, "operation", "subject", 31_337, now).unwrap();
        assert_eq!(first.nonce, repeated.nonce);
        assert!(challenge_message(&first).contains("operation_id: operation"));
        assert!(create_or_get_challenge(&conn, "operation", "other-subject", 31_337, now).is_err());
    }

    /// T03: A wallet proof cannot be replayed for another subject, operation,
    /// or chain. The challenge is single-use: once `wallet_address` is set the
    /// row is locked against a second verification.
    #[test]
    fn wallet_proof_cannot_be_replayed_for_different_context() {
        use alloy_primitives::hex;
        use alloy_signer::Signer;
        use alloy_signer_local::PrivateKeySigner;

        let conn = Connection::open_in_memory().unwrap();
        migrate_wallet_links(&conn).unwrap();
        let now = Utc::now();

        // Create a challenge for operation-A with chain 31337.
        let challenge_a =
            create_or_get_challenge(&conn, "op-a", "subject-a", 31_337, now).unwrap();
        assert!(challenge_message(&challenge_a).contains("operation_id: op-a"));
        assert!(challenge_message(&challenge_a).contains("chain_id: 31337"));

        // A different subject or chain is rejected at challenge creation time.
        assert!(create_or_get_challenge(&conn, "op-a", "subject-b", 31_337, now).is_err());
        assert!(create_or_get_challenge(&conn, "op-a", "subject-a", 1, now).is_err());

        // Sign the challenge with a deterministic test key (Anvil key #0).
        let key_bytes =
            hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        let signer = PrivateKeySigner::from_slice(&key_bytes).unwrap();
        let msg = challenge_message(&challenge_a);
        let signature = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(signer.sign_message(msg.as_bytes()))
            .unwrap();
        let sig_hex = format!("0x{}", hex::encode(signature.as_bytes()));

        // Verify and record the link.
        let link = verify_and_record(&conn, &challenge_a, &sig_hex, now).unwrap();
        assert!(!link.wallet_address.is_empty());
        assert_eq!(link.subject_hash, "subject-a");
        assert_eq!(link.chain_id, 31_337);

        // The same challenge cannot be verified a second time (single-use).
        assert!(verify_and_record(&conn, &challenge_a, &sig_hex, now)
            .unwrap_err()
            .to_string()
            .contains("wallet_link_already_used_or_mismatched"));

        // A different operation cannot reuse this link (get_verified is op-scoped).
        assert!(get_verified(&conn, "op-b").unwrap().is_none());

        // The verified link is retrievable for the original operation.
        let recovered = get_verified(&conn, "op-a").unwrap().unwrap();
        assert_eq!(recovered.wallet_address, link.wallet_address);
    }

    /// T03: An expired challenge is rejected. The five-minute expiry is
    /// enforced by `verify_and_record`, not just by a wall-clock check.
    #[test]
    fn expired_wallet_link_challenge_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_wallet_links(&conn).unwrap();

        // Create a challenge that was issued five minutes + one second ago.
        let issued_at = Utc::now() - chrono::TimeDelta::seconds(301);
        let challenge =
            create_or_get_challenge(&conn, "exp-op", "exp-subject", 31_337, issued_at).unwrap();

        // Attempting to verify with `now` must fail because the challenge expired.
        let now = Utc::now();
        let error = verify_and_record(&conn, &challenge, "0x1234", now)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("wallet_link_expired"),
            "expected wallet_link_expired, got: {error}"
        );
    }
}

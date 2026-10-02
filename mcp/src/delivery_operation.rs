//! Byte-free durable delivery state. Financial receipts remain in paid_operations.
//! An operation's locator is fixed before any upload, so crashes never lose it.
use anyhow::{anyhow, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub const MIGRATION: &str = "CREATE TABLE IF NOT EXISTS delivery_operations (
 operation_id TEXT PRIMARY KEY, author TEXT NOT NULL, envelope_digest TEXT NOT NULL,
 byte_length INTEGER NOT NULL, backend TEXT NOT NULL, locator TEXT NOT NULL,
 payment_status TEXT NOT NULL, delivery_status TEXT NOT NULL,
 lease_id TEXT, lease_until INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
 error_code TEXT, remedy_reference TEXT, updated_at INTEGER NOT NULL
);";
#[derive(Debug, Clone, Serialize)]
pub struct DeliveryOperation {
    pub operation_id: String,
    pub author: String,
    pub envelope_digest: String,
    pub byte_length: usize,
    pub backend: String,
    pub locator: String,
    pub payment_status: String,
    pub delivery_status: String,
    pub attempts: u32,
    pub error_code: Option<String>,
    pub remedy_reference: Option<String>,
}
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION)?;
    Ok(())
}
pub fn get(conn: &Connection, id: &str) -> Result<Option<DeliveryOperation>> {
    Ok(conn.query_row("SELECT operation_id,author,envelope_digest,byte_length,backend,locator,payment_status,delivery_status,attempts,error_code,remedy_reference FROM delivery_operations WHERE operation_id=?1",[id],|r|Ok(DeliveryOperation {operation_id:r.get(0)?,author:r.get(1)?,envelope_digest:r.get(2)?,byte_length:r.get(3)?,backend:r.get(4)?,locator:r.get(5)?,payment_status:r.get(6)?,delivery_status:r.get(7)?,attempts:r.get(8)?,error_code:r.get(9)?,remedy_reference:r.get(10)?})).optional()?)
}
#[allow(clippy::too_many_arguments)]
pub fn bind(
    conn: &Connection,
    id: &str,
    author: &str,
    digest: &str,
    size: usize,
    backend: &str,
    locator: &str,
    paid: bool,
    now: i64,
) -> Result<DeliveryOperation> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b)),
        "invalid operation ID"
    );
    ensure!(
        !author.is_empty()
            && digest.len() == 64
            && size > 0
            && size <= 1048576
            && !backend.is_empty()
            && !locator.is_empty(),
        "invalid operation binding"
    );
    conn.execute("INSERT INTO delivery_operations(operation_id,author,envelope_digest,byte_length,backend,locator,payment_status,delivery_status,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'validated',?8) ON CONFLICT(operation_id) DO NOTHING",params![id,author,digest,size,backend,locator,if paid{"required"}else{"not_required"},now])?;
    let row = get(conn, id)?.ok_or_else(|| anyhow!("operation disappeared"))?;
    ensure!(
        row.author == author
            && row.envelope_digest == digest
            && row.byte_length == size
            && row.backend == backend
            && row.locator == locator,
        "operation binding conflict"
    );
    Ok(row)
}
/// One lease spans payment reconciliation, settlement, upload and verification.
/// Expired leases resume the SAME financial operation and deterministic locator.
pub fn acquire(conn: &Connection, id: &str, lease: &str, now: i64) -> Result<bool> {
    ensure!(!lease.is_empty(), "missing lease");
    Ok(conn.execute("UPDATE delivery_operations SET lease_id=?1,lease_until=?2,updated_at=?3 WHERE operation_id=?4 AND delivery_status!='failed_terminal' AND (lease_id IS NULL OR lease_until<=?3)",params![lease,now+600,now,id])?==1)
}
pub fn payment_settled(conn: &Connection, id: &str, lease: &str, now: i64) -> Result<()> {
    let changed=conn.execute("UPDATE delivery_operations SET payment_status='settled',updated_at=?1 WHERE operation_id=?2 AND lease_id=?3 AND payment_status IN ('required','settled')",params![now,id,lease])?;
    ensure!(changed == 1, "delivery lease/payment conflict");
    Ok(())
}
pub fn payment_free(conn: &Connection, id: &str, lease: &str, now: i64) -> Result<()> {
    let changed=conn.execute("UPDATE delivery_operations SET payment_status='not_required',updated_at=?1 WHERE operation_id=?2 AND lease_id=?3 AND payment_status IN ('required','not_required')",params![now,id,lease])?;
    ensure!(changed == 1, "delivery lease/payment conflict");
    Ok(())
}
pub fn quota_refunded(conn: &Connection, id: &str, lease: &str) -> Result<()> {
    ensure!(conn.execute("UPDATE delivery_operations SET payment_status='required' WHERE operation_id=?1 AND (lease_id=?2 OR lease_id IS NULL) AND payment_status='not_required' AND delivery_status!='verified'",params![id,lease])?==1,"quota state conflict");
    Ok(())
}
pub fn uploading(conn: &Connection, id: &str, lease: &str, now: i64) -> Result<()> {
    let changed=conn.execute("UPDATE delivery_operations SET delivery_status='uploading',attempts=attempts+1,updated_at=?1 WHERE operation_id=?2 AND lease_id=?3 AND payment_status IN ('settled','not_required')",params![now,id,lease])?;
    ensure!(changed == 1, "delivery lease/payment conflict");
    Ok(())
}
pub fn release(
    conn: &Connection,
    id: &str,
    lease: &str,
    status: &str,
    error: Option<&str>,
    now: i64,
) -> Result<()> {
    ensure!(
        [
            "validated",
            "verification_pending",
            "verified",
            "failed_retryable",
            "failed_terminal"
        ]
        .contains(&status),
        "invalid delivery status"
    );
    let changed=conn.execute("UPDATE delivery_operations SET delivery_status=?1,error_code=?2,lease_id=NULL,lease_until=NULL,updated_at=?3,payment_status=CASE WHEN ?1='failed_terminal' AND payment_status='settled' THEN 'remedy_pending' ELSE payment_status END WHERE operation_id=?4 AND lease_id=?5",params![status,error,now,id,lease])?;
    ensure!(changed == 1, "delivery lease conflict");
    Ok(())
}
/// Manual remedy evidence only; this does not move funds or grant credit.
#[allow(dead_code)]
pub fn record_remedy(conn: &Connection, id: &str, reference: &str, now: i64) -> Result<()> {
    ensure!(
        !reference.is_empty() && reference.len() <= 1024,
        "remedy evidence required"
    );
    ensure!(conn.execute("UPDATE delivery_operations SET payment_status='remedied',remedy_reference=?1,updated_at=?2 WHERE operation_id=?3 AND payment_status='remedy_pending'",params![reference,now,id])?==1,"remedy state conflict");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        migrate(&c).unwrap();
        bind(
            &c,
            "op",
            "owner",
            &"a".repeat(64),
            12,
            "arweave",
            "known-before-upload",
            true,
            0,
        )
        .unwrap();
        c
    }
    #[test]
    fn crash_after_settlement_reuses_binding_and_locator() {
        let c = setup();
        assert!(acquire(&c, "op", "first", 1).unwrap());
        payment_settled(&c, "op", "first", 2).unwrap();
        assert!(!acquire(&c, "op", "concurrent", 3).unwrap());
        assert!(acquire(&c, "op", "restart", 602).unwrap());
        let row = get(&c, "op").unwrap().unwrap();
        assert_eq!(row.payment_status, "settled");
        assert_eq!(row.locator, "known-before-upload");
        assert!(bind(
            &c,
            "op",
            "owner",
            &"b".repeat(64),
            12,
            "arweave",
            "known-before-upload",
            true,
            603
        )
        .is_err());
        assert!(release(&c, "op", "first", "verified", None, 603).is_err());
    }
    #[test]
    fn crash_after_upload_and_receipt_failure_preserves_progress() {
        let c = setup();
        acquire(&c, "op", "one", 0).unwrap();
        payment_settled(&c, "op", "one", 1).unwrap();
        uploading(&c, "op", "one", 2).unwrap();
        assert!(acquire(&c, "op", "two", 601).unwrap());
        release(&c, "op", "two", "verified", None, 602).unwrap();
        let row = get(&c, "op").unwrap().unwrap();
        assert_eq!(row.payment_status, "settled");
        assert_eq!(row.delivery_status, "verified");
        assert_eq!(row.locator, "known-before-upload");
    }
    #[test]
    fn terminal_paid_failure_requires_visible_remedy_evidence() {
        let c = setup();
        acquire(&c, "op", "one", 0).unwrap();
        payment_settled(&c, "op", "one", 1).unwrap();
        release(&c, "op", "one", "failed_terminal", Some("retry_limit"), 2).unwrap();
        assert_eq!(
            get(&c, "op").unwrap().unwrap().payment_status,
            "remedy_pending"
        );
        assert!(!acquire(&c, "op", "two", 1000).unwrap());
        assert!(record_remedy(&c, "op", "", 3).is_err());
        record_remedy(&c, "op", "refund:approved-reference", 3).unwrap();
        assert_eq!(get(&c, "op").unwrap().unwrap().payment_status, "remedied");
    }
    #[test]
    fn concurrent_connections_allow_one_lease() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let c = Connection::open(f.path()).unwrap();
        migrate(&c).unwrap();
        bind(
            &c,
            "op",
            "owner",
            &"a".repeat(64),
            12,
            "arweave",
            "locator",
            true,
            0,
        )
        .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let path = f.path().to_owned();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    let c = Connection::open(path).unwrap();
                    b.wait();
                    acquire(&c, "op", &format!("lease-{i}"), 1).unwrap()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .filter(|h| h.thread().id() != std::thread::current().id())
                .map(|h| h.join().unwrap() as usize)
                .sum::<usize>(),
            1
        );
    }
}

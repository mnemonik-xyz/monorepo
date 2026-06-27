//! SQLite local-cache backend for verifiable trajectories.
//!
//! Per `work/verifiable-trajectories/decisions.md` the *canonical* store is
//! Arweave ANS-104 bundles and the MCP server is stateless. This SQLite store is
//! the sanctioned **optional local/offline cache** (the `local` write mode):
//! free, offline, prunable, never the canonical source of truth. It implements
//! the same `TrajectoryStore` trait the verifier consumes, so the identical
//! chain/coverage checks run against it.
//!
//! Owns its own `Connection` and is `!Send`; in async contexts wrap in a
//! `std::sync::Mutex` and never hold the lock across an `.await` (same
//! discipline as `SqliteStore`).

use anyhow::Context;
use rusqlite::{params, Connection};
use std::path::Path;

use crate::trajectory::{StepRecord, TrajectoryStore, VerdictRecord, VerdictStatus};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS trajectory_steps (
    content_hash   TEXT PRIMARY KEY,
    trajectory_id  TEXT NOT NULL,
    seq            INTEGER NOT NULL,
    prev_hash      TEXT,
    producer       TEXT NOT NULL,
    cose_bytes     BLOB NOT NULL,
    canonical_cbor BLOB NOT NULL,
    created_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_traj_steps ON trajectory_steps(trajectory_id, seq);
CREATE TABLE IF NOT EXISTS trajectory_verdicts (
    content_hash TEXT PRIMARY KEY,
    step_hash    TEXT NOT NULL,
    status       TEXT NOT NULL,
    judge        TEXT NOT NULL,
    cose_bytes   BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_traj_verdicts ON trajectory_verdicts(step_hash);
"#;

pub struct SqliteTrajectoryStore {
    conn: Connection,
}

impl SqliteTrajectoryStore {
    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(SCHEMA)
            .context("create trajectory tables")?;
        Ok(Self { conn })
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Self::init(Connection::open(path).context("open trajectory db")?)
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory().context("open in-memory trajectory db")?)
    }

    /// Persist a signed step. Idempotent on `content_hash` (re-attesting the
    /// same step is a no-op).
    pub fn insert_step(&self, s: &StepRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO trajectory_steps \
             (content_hash, trajectory_id, seq, prev_hash, producer, cose_bytes, canonical_cbor, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                s.content_hash,
                s.trajectory_id,
                s.seq as i64,
                s.prev_hash,
                s.producer,
                s.cose_bytes,
                s.canonical_cbor,
                ""
            ],
        )?;
        Ok(())
    }

    /// Producer (signer pubkey) of a step by its `content_hash`, if present.
    /// Used to enforce judge ≠ producer when attaching a verdict.
    pub fn step_producer(&self, step_hash: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT producer FROM trajectory_steps WHERE content_hash = ?1")?;
        let mut rows = stmt.query(params![step_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Persist a signed verdict. Idempotent on the verdict's `content_hash`.
    pub fn insert_verdict(&self, v: &VerdictRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO trajectory_verdicts \
             (content_hash, step_hash, status, judge, cose_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                v.content_hash,
                v.step_hash,
                v.status.as_str(),
                v.judge,
                v.cose_bytes
            ],
        )?;
        Ok(())
    }
}

impl TrajectoryStore for SqliteTrajectoryStore {
    fn steps_for_trajectory(&self, trajectory_id: &str) -> anyhow::Result<Vec<StepRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, seq, prev_hash, producer, cose_bytes, canonical_cbor \
             FROM trajectory_steps WHERE trajectory_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![trajectory_id], |row| {
            Ok(StepRecord {
                trajectory_id: trajectory_id.to_string(),
                content_hash: row.get(0)?,
                seq: row.get::<_, i64>(1)? as u64,
                prev_hash: row.get(2)?,
                producer: row.get(3)?,
                cose_bytes: row.get(4)?,
                canonical_cbor: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn verdicts_for_step(&self, step_hash: &str) -> anyhow::Result<Vec<VerdictRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, status, judge, cose_bytes \
             FROM trajectory_verdicts WHERE step_hash = ?1",
        )?;
        let rows = stmt.query_map(params![step_hash], |row| {
            let status: String = row.get(1)?;
            Ok(VerdictRecord {
                step_hash: step_hash.to_string(),
                content_hash: row.get(0)?,
                status: VerdictStatus::from_str(&status).unwrap_or(VerdictStatus::Concern),
                judge: row.get(2)?,
                cose_bytes: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::{build_step, verify_chain, StepInput};
    use solana_sdk::signature::{Keypair, Signer};

    fn rec(tid: &str, seq: u64, prev: Option<&str>, kp: &Keypair) -> StepRecord {
        let s = build_step(
            &StepInput {
                trajectory_id: tid,
                seq,
                content: "step",
                prev_hash: prev,
                created_at: "2026-06-27T00:00:00Z",
            },
            kp,
        )
        .unwrap();
        StepRecord {
            trajectory_id: tid.to_string(),
            seq,
            content_hash: s.content_hash,
            prev_hash: prev.map(str::to_string),
            producer: kp.pubkey().to_string(),
            cose_bytes: s.cose_bytes,
            canonical_cbor: s.canonical_cbor,
        }
    }

    #[test]
    fn insert_and_order_steps_then_verify() {
        let kp = Keypair::new();
        let store = SqliteTrajectoryStore::in_memory().unwrap();
        let s0 = rec("t", 0, None, &kp);
        let s1 = rec("t", 1, Some(&s0.content_hash), &kp);
        // Insert out of order — read back must be seq-ordered.
        store.insert_step(&s1).unwrap();
        store.insert_step(&s0).unwrap();
        let steps = store.steps_for_trajectory("t").unwrap();
        assert_eq!(steps.iter().map(|s| s.seq).collect::<Vec<_>>(), vec![0, 1]);
        // The verifier runs against the SQLite cache unchanged.
        assert!(verify_chain(&store, "t").unwrap().chain_valid);
    }

    #[test]
    fn insert_step_is_idempotent() {
        let kp = Keypair::new();
        let store = SqliteTrajectoryStore::in_memory().unwrap();
        let s0 = rec("t", 0, None, &kp);
        store.insert_step(&s0).unwrap();
        store.insert_step(&s0).unwrap();
        assert_eq!(store.steps_for_trajectory("t").unwrap().len(), 1);
    }

    #[test]
    fn trajectory_head_tracks_max_seq() {
        let kp = Keypair::new();
        let store = SqliteTrajectoryStore::in_memory().unwrap();
        let s0 = rec("t", 0, None, &kp);
        let s1 = rec("t", 1, Some(&s0.content_hash), &kp);
        store.insert_step(&s0).unwrap();
        store.insert_step(&s1).unwrap();
        let head = store.trajectory_head("t").unwrap().unwrap();
        assert_eq!(head.seq, 1);
        assert_eq!(head.content_hash, s1.content_hash);
    }
}

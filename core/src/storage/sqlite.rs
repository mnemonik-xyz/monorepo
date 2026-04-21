//! SQLite implementation of storage traits.
//!
//! `SqliteStore` wraps a `rusqlite::Connection`. It is `!Send` -- in async contexts,
//! callers must wrap it in `std::sync::Mutex` and never hold the lock across an `.await` point.

use anyhow::Context;
use rusqlite::{Connection, params};
use std::path::Path;

use super::traits::{AttestationRow, AttestationStore, LineageStore, SearchResult};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS attestations (
    attestation_id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    solana_tx TEXT NOT NULL,
    arweave_tx TEXT NOT NULL,
    signer_pubkey TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attestation_embeddings (
    attestation_id TEXT PRIMARY KEY,
    embedding_dim INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    FOREIGN KEY (attestation_id) REFERENCES attestations(attestation_id)
);
CREATE INDEX IF NOT EXISTS idx_attestations_signer ON attestations(signer_pubkey);

CREATE TABLE IF NOT EXISTS api_keys (
    api_key TEXT PRIMARY KEY,
    owner_pubkey TEXT NOT NULL DEFAULT '',
    balance_micro_usdc INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_used_at TEXT
);

CREATE TABLE IF NOT EXISTS payment_events (
    event_id TEXT PRIMARY KEY,
    api_key TEXT NOT NULL,
    amount_micro_usdc INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    tx_sig TEXT,
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_payment_events_key ON payment_events(api_key);

CREATE TABLE IF NOT EXISTS x402_nonces (
    tx_sig TEXT PRIMARY KEY,
    used_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS attestation_costs (
    attestation_id TEXT PRIMARY KEY,
    irys_cost_lamports INTEGER NOT NULL,
    sol_tx_fee_lamports INTEGER NOT NULL,
    sol_price_usdc REAL NOT NULL,
    earned_micro_usdc INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (attestation_id) REFERENCES attestations(attestation_id)
);

CREATE TABLE IF NOT EXISTS lineage_edges (
    parent_id TEXT NOT NULL,
    child_id TEXT NOT NULL,
    depth INTEGER NOT NULL,
    PRIMARY KEY (parent_id, child_id)
);
CREATE INDEX IF NOT EXISTS idx_lineage_edges_child ON lineage_edges(child_id);
"#;

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating db directory")?;
        }
        let conn = Connection::open(path).context("opening SQLite")?;
        conn.execute_batch(SCHEMA).context("initializing schema")?;
        Ok(Self { conn })
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Direct access to the underlying connection for payment methods in mcp/.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

impl AttestationStore for SqliteStore {
    fn save_attestation(
        &self,
        attestation_id: &str,
        content: &str,
        content_hash: &str,
        tags: &[String],
        solana_tx: &str,
        arweave_tx: &str,
        signer_pubkey: &str,
        created_at: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO attestations VALUES (?,?,?,?,?,?,?,?)",
            params![attestation_id, content, content_hash, tags_json,
                    solana_tx, arweave_tx, signer_pubkey, created_at],
        )?;
        let emb_bytes = floats_to_bytes(embedding);
        self.conn.execute(
            "INSERT OR REPLACE INTO attestation_embeddings VALUES (?,?,?)",
            params![attestation_id, embedding.len() as i32, emb_bytes],
        )?;
        Ok(())
    }

    fn find_by_tx(&self, tx_id: &str) -> anyhow::Result<Option<AttestationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT attestation_id, content, content_hash, solana_tx, arweave_tx, signer_pubkey
             FROM attestations WHERE solana_tx = ?1 OR arweave_tx = ?1 LIMIT 1"
        )?;
        let mut rows = stmt.query(params![tx_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(AttestationRow {
                attestation_id: row.get(0)?,
                content: row.get(1)?,
                content_hash: row.get(2)?,
                solana_tx: row.get(3)?,
                arweave_tx: row.get(4)?,
                signer_pubkey: row.get(5)?,
            })),
            None => Ok(None),
        }
    }

    fn count(&self, signer: &str) -> anyhow::Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM attestations WHERE signer_pubkey = ?",
            params![signer], |row| row.get(0),
        )?;
        Ok(count)
    }

    fn search(
        &self,
        query_embedding: &[f32],
        signer: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
                    a.solana_tx, a.arweave_tx, a.created_at, ae.embedding
             FROM attestations a
             JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
             WHERE a.signer_pubkey = ?"
        )?;

        let q_norm = l2_norm(query_embedding);
        let q_normalized: Vec<f32> = if q_norm > 0.0 {
            query_embedding.iter().map(|x| x / q_norm).collect()
        } else {
            query_embedding.to_vec()
        };

        let mut results: Vec<SearchResult> = stmt
            .query_map(params![signer], |row| {
                let emb_blob: Vec<u8> = row.get(7)?;
                let emb = bytes_to_floats(&emb_blob);
                let e_norm = l2_norm(&emb);
                let score = if e_norm > 0.0 {
                    q_normalized.iter().zip(emb.iter()).map(|(a, b)| a * b / e_norm).sum::<f32>()
                } else {
                    0.0
                };
                let tags_str: String = row.get(3)?;
                Ok(SearchResult {
                    attestation_id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get(2)?,
                    tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                    solana_tx: row.get(4)?,
                    arweave_tx: row.get(5)?,
                    created_at: row.get(6)?,
                    relevance_score: score,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());
        results.truncate(limit);
        Ok(results)
    }
}

impl LineageStore for SqliteStore {
    fn save_edge(&self, parent_id: &str, child_id: &str, depth: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO lineage_edges (parent_id, child_id, depth) VALUES (?,?,?)",
            params![parent_id, child_id, depth],
        )?;
        Ok(())
    }

    fn get_edges(&self, child_id: &str) -> anyhow::Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_id, depth FROM lineage_edges WHERE child_id = ?"
        )?;
        let rows = stmt.query_map(params![child_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn clear_edges(&self, artifact_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM lineage_edges WHERE parent_id = ? OR child_id = ?",
            params![artifact_id, artifact_id],
        )?;
        Ok(())
    }
}

fn floats_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_floats(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_find_by_tx() {
        let store = SqliteStore::in_memory().unwrap();
        store.save_attestation(
            "att-1", "content", "hash1", &["tag".into()],
            "sol_tx_1", "ar_tx_1", "signer1", "2026-04-13T00:00:00Z",
            &[1.0, 0.0],
        ).unwrap();

        let found = store.find_by_tx("sol_tx_1").unwrap();
        assert!(found.is_some());
        let row = found.unwrap();
        assert_eq!(row.attestation_id, "att-1");
        assert_eq!(row.content, "content");
        assert_eq!(row.content_hash, "hash1");
    }

    #[test]
    fn test_count_by_signer() {
        let store = SqliteStore::in_memory().unwrap();
        for i in 0..2 {
            store.save_attestation(
                &format!("att-{i}"), "c", "h", &[], "sol", "ar",
                "signer_a", "2026-01-01", &[1.0, 0.0],
            ).unwrap();
        }
        store.save_attestation(
            "att-other", "c", "h", &[], "sol2", "ar2",
            "signer_b", "2026-01-01", &[1.0, 0.0],
        ).unwrap();

        assert_eq!(store.count("signer_a").unwrap(), 2);
        assert_eq!(store.count("signer_b").unwrap(), 1);
    }

    #[test]
    fn test_search_ranking() {
        let store = SqliteStore::in_memory().unwrap();
        // Two attestations with distinct embeddings
        store.save_attestation(
            "att-0", "topic zero", "h0", &[], "s0", "a0",
            "agent", "2026-01-01", &[1.0, 0.0],
        ).unwrap();
        store.save_attestation(
            "att-1", "topic one", "h1", &[], "s1", "a1",
            "agent", "2026-01-01", &[0.0, 1.0],
        ).unwrap();

        // Query closer to att-0's embedding
        let results = store.search(&[1.0, 0.0], "agent", 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].attestation_id, "att-0");
    }

    #[test]
    fn test_find_by_tx_not_found() {
        let store = SqliteStore::in_memory().unwrap();
        let found = store.find_by_tx("nonexistent").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_duplicate_attestation_id() {
        let store = SqliteStore::in_memory().unwrap();
        store.save_attestation(
            "att-dup", "c1", "h1", &[], "sol1", "ar1",
            "s1", "2026-01-01", &[1.0],
        ).unwrap();
        // INSERT OR REPLACE so this succeeds (replaces)
        let result = store.save_attestation(
            "att-dup", "c2", "h2", &[], "sol2", "ar2",
            "s1", "2026-01-01", &[1.0],
        );
        assert!(result.is_ok());
        // Content should be updated
        let row = store.find_by_tx("sol2").unwrap().unwrap();
        assert_eq!(row.content, "c2");
    }

    #[test]
    fn test_lineage_save_and_get() {
        let store = SqliteStore::in_memory().unwrap();
        store.save_edge("parent-1", "child-1", 1).unwrap();
        store.save_edge("parent-2", "child-1", 1).unwrap();

        let edges = store.get_edges("child-1").unwrap();
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_lineage_clear() {
        let store = SqliteStore::in_memory().unwrap();
        store.save_edge("p1", "c1", 1).unwrap();
        store.save_edge("c1", "c2", 2).unwrap();
        store.clear_edges("c1").unwrap();

        let edges_c1 = store.get_edges("c1").unwrap();
        assert!(edges_c1.is_empty());
        let edges_c2 = store.get_edges("c2").unwrap();
        assert!(edges_c2.is_empty());
    }
}

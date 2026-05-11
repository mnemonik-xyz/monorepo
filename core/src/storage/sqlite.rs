//! SQLite implementation of storage traits.
//!
//! `SqliteStore` wraps a `rusqlite::Connection`. It is `!Send` -- in async contexts,
//! callers must wrap it in `std::sync::Mutex` and never hold the lock across an `.await` point.

use anyhow::Context;
use rusqlite::{params, Connection};
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
    -- owner_pubkey TEXT is added by migrate_owner_pubkey_columns().
    -- It is intentionally NOT in this CREATE so the migration helper is the
    -- single source of truth for both fresh DBs (where it ALTERs immediately)
    -- and legacy DBs (where it backfills). PRAGMA table_info gates the ALTER
    -- so it runs at most once per DB.
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
-- Note: the partial UNIQUE index on payment_events(tx_sig) is created
-- separately in `SqliteStore::open` / `SqliteStore::in_memory` AFTER a
-- one-shot dedup pass. Creating it here would fail on legacy databases
-- that accumulated duplicate tx_sigs under the old TOCTOU-vulnerable
-- code path. See `open()` for the migration sequence.

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

/// Aggregate counters returned by `SqliteStore::public_stats`.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PublicStats {
    pub unique_users: i64,
    pub saved_on_node: i64,
    pub saved_onchain: i64,
}

/// Shared initialization step run after `SCHEMA` for every backing store
/// (file-backed via `open` and in-memory via `in_memory`).
///
/// Dedups any pre-existing duplicate non-NULL `tx_sig` rows, then creates
/// the partial UNIQUE index on `payment_events(tx_sig)`. The dedup keeps
/// the earliest row per `tx_sig` (lowest `rowid`) and deletes the rest —
/// it is a no-op on fresh or already-clean databases, and cleans up
/// legacy rows produced by the old TOCTOU-vulnerable `credit_deposit`.
///
/// Wrapped in `BEGIN IMMEDIATE` / `COMMIT` so that a concurrent opener
/// cannot see a half-migrated state. `CREATE UNIQUE INDEX IF NOT EXISTS`
/// is idempotent, so repeated opens of a clean DB execute cheaply.
fn migrate_payment_events_unique_index(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         DELETE FROM payment_events
          WHERE tx_sig IS NOT NULL
            AND rowid NOT IN (
                SELECT MIN(rowid) FROM payment_events
                 WHERE tx_sig IS NOT NULL
                 GROUP BY tx_sig
            );
         CREATE UNIQUE INDEX IF NOT EXISTS uq_payment_events_tx_sig
             ON payment_events(tx_sig) WHERE tx_sig IS NOT NULL;
         COMMIT;",
    )
    .context("migrating payment_events UNIQUE(tx_sig) index")?;
    Ok(())
}

/// Idempotent ADD-COLUMN migration for OAuth ownership scope (Decision 9 + 11).
///
/// Adds two columns:
///   - `attestations.owner_pubkey TEXT` — the OAuth-resolved tenant scope
///     used by `AttestationStore::search`. Distinct from the existing
///     `signer_pubkey` column (the COSE_Sign1 signer identity).
///   - `api_keys.oauth_pubkey TEXT` — links an API key to the OAuth user
///     pubkey. Distinct from the existing `api_keys.owner_pubkey` (deposit
///     owner — wallet that funds the key). Both can coexist.
///
/// After ensuring the columns exist, backfills any pre-OAuth rows where
/// `owner_pubkey` is NULL or empty by copying `signer_pubkey` into it. Pre-
/// OAuth attestations were single-tenant by construction (server signs and
/// implicitly owns), and without this backfill the mandatory owner filter in
/// `search` hides them — including the RAG seed corpus that powers `/chat`.
/// The backfill runs unconditionally and is idempotent (UPDATE only touches
/// matching rows).
///
/// SQLite `ALTER TABLE ... ADD COLUMN` does not support `IF NOT EXISTS`, so
/// presence is checked via `PRAGMA table_info(...)` and the ALTER runs only
/// when absent — making the function idempotent across deploys. Wrapped in
/// `BEGIN IMMEDIATE` to serialize against any concurrent opener.
fn migrate_owner_pubkey_columns(conn: &Connection) -> anyhow::Result<()> {
    fn has_column(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .with_context(|| format!("preparing PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    let need_attestations_col = !has_column(conn, "attestations", "owner_pubkey")?;
    let need_api_keys_col = !has_column(conn, "api_keys", "oauth_pubkey")?;

    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening owner_pubkey migration transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        if need_attestations_col {
            conn.execute("ALTER TABLE attestations ADD COLUMN owner_pubkey TEXT", [])
                .context("adding attestations.owner_pubkey")?;
            // Index speeds up the per-owner search query (mandatory filter).
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_attestations_owner ON attestations(owner_pubkey)",
                [],
            )
            .context("creating idx_attestations_owner")?;
        }
        if need_api_keys_col {
            conn.execute("ALTER TABLE api_keys ADD COLUMN oauth_pubkey TEXT", [])
                .context("adding api_keys.oauth_pubkey")?;
        }
        // Backfill legacy rows. Runs every open: cheap on clean DBs (zero
        // matches), correct on DBs that had the column added without a
        // backfill in an earlier release.
        conn.execute(
            "UPDATE attestations
                SET owner_pubkey = signer_pubkey
              WHERE owner_pubkey IS NULL OR owner_pubkey = ''",
            [],
        )
        .context("backfilling attestations.owner_pubkey from signer_pubkey")?;
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing owner_pubkey migration")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

/// Idempotent ADD-COLUMN migration for the deferred-sign correlation routing.
///
/// Adds `attestations.correlation_id TEXT` (nullable) — the routing token the
/// HTTP/JWT browser-mediated flow uses between `mnemonic_sign_memory` (server
/// returns `awaiting_signature` + correlation_id) and `/api/sign-callback`
/// (webapp posts the COSE envelope back). Storing it on the persisted row
/// lets `mnemonic_check_pending(correlation_id)` resolve the final
/// `solana_tx` + `arweave_tx` after the user approves in the browser.
///
/// Partial index `WHERE correlation_id IS NOT NULL` keeps lookups O(log n)
/// without bloating against legacy rows that pre-date the column. SQLite
/// `ALTER TABLE ... ADD COLUMN` lacks `IF NOT EXISTS`, so presence is gated
/// by `PRAGMA table_info`.
fn migrate_correlation_id_column(conn: &Connection) -> anyhow::Result<()> {
    fn has_column(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .with_context(|| format!("preparing PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    if !has_column(conn, "attestations", "correlation_id")? {
        conn.execute(
            "ALTER TABLE attestations ADD COLUMN correlation_id TEXT",
            [],
        )
        .context("adding attestations.correlation_id")?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_attestations_correlation_id
                 ON attestations(correlation_id) WHERE correlation_id IS NOT NULL",
            [],
        )
        .context("creating idx_attestations_correlation_id")?;
    }
    Ok(())
}

impl SqliteStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating db directory")?;
        }
        let conn = Connection::open(path).context("opening SQLite")?;
        // WAL lets concurrent readers (e.g. the MCP server's pricing-refresh
        // thread) overlap with payment writers instead of blocking on the
        // database-level lock. busy_timeout=5000 ensures a BEGIN IMMEDIATE
        // that loses the lock race queues for up to 5s rather than
        // returning SQLITE_BUSY immediately (rusqlite default is 0ms),
        // which also stabilizes the payment race tests.
        // `PRAGMA foreign_keys=ON` is per-connection and must be set
        // explicitly; SQLite defaults to OFF, which would silently demote
        // `FOREIGN KEY ... ON DELETE CASCADE` clauses (such as the one on
        // `key_escrow_blobs(google_sub) REFERENCES google_identity_links`)
        // to no-ops. We enable it here so cascade-delete and FK-violation
        // rejection actually fire.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;",
        )
        .context("setting WAL + busy_timeout + foreign_keys pragmas")?;
        conn.execute_batch(SCHEMA).context("initializing schema")?;
        migrate_payment_events_unique_index(&conn)?;
        migrate_owner_pubkey_columns(&conn)?;
        migrate_correlation_id_column(&conn)?;
        Ok(Self { conn })
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        // busy_timeout is harmless on in-memory DBs and keeps behavior
        // consistent with file-backed stores. WAL mode is meaningless in
        // memory and not requested. `foreign_keys=ON` mirrors `open()` so
        // FK CASCADE clauses (e.g. `key_escrow_blobs` → `google_identity_links`)
        // are enforced in tests using `in_memory()` too.
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        migrate_payment_events_unique_index(&conn)?;
        migrate_owner_pubkey_columns(&conn)?;
        migrate_correlation_id_column(&conn)?;
        Ok(Self { conn })
    }

    /// Direct access to the underlying connection for payment methods in mcp/.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Stamp `correlation_id` onto an already-persisted attestation row.
    /// Used by `/api/sign-callback` after `save_attestation` succeeds, so
    /// `mnemonic_check_pending(correlation_id)` can later resolve the row.
    /// No-op (zero rows updated) if `attestation_id` is unknown.
    pub fn set_correlation_id(
        &self,
        attestation_id: &str,
        correlation_id: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE attestations SET correlation_id = ?1 WHERE attestation_id = ?2",
            params![correlation_id, attestation_id],
        )?;
        Ok(())
    }

    /// Aggregate counters intended for a public traction widget.
    ///
    /// - `unique_users`: distinct non-empty `owner_pubkey` (OAuth-resolved
    ///   tenant scope). One human-or-agent identity counts once regardless of
    ///   how many memories it saved.
    /// - `saved_on_node`: total rows in `attestations` (everything the node
    ///   has persisted, including local-mode synthetic txs).
    /// - `saved_onchain`: rows whose `solana_tx` is a real Solana signature
    ///   (i.e. not the `local:<...>` synthetic prefix used in
    ///   `STORAGE_MODE=local` and not empty).
    ///
    /// All three are single indexed/scanned aggregates over `attestations`;
    /// safe to call on the request path but the caller may still cache to
    /// avoid hammering SQLite.
    pub fn public_stats(&self) -> anyhow::Result<PublicStats> {
        let unique_users: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT owner_pubkey) FROM attestations
             WHERE owner_pubkey IS NOT NULL AND owner_pubkey <> ''",
            [],
            |row| row.get(0),
        )?;
        let saved_on_node: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM attestations", [], |row| row.get(0))?;
        let saved_onchain: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM attestations
             WHERE solana_tx <> '' AND solana_tx NOT LIKE 'local:%'",
            [],
            |row| row.get(0),
        )?;
        Ok(PublicStats {
            unique_users,
            saved_on_node,
            saved_onchain,
        })
    }

    /// Resolve a persisted attestation by the deferred-sign correlation_id.
    /// Returns `(attestation_id, content_hash, solana_tx, arweave_tx,
    /// signer_pubkey, created_at)` or `None`.
    ///
    /// Bounded by the partial index `idx_attestations_correlation_id`.
    #[allow(clippy::type_complexity)]
    pub fn find_by_correlation_id(
        &self,
        correlation_id: &str,
    ) -> anyhow::Result<Option<(String, String, String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT attestation_id, content_hash, solana_tx, arweave_tx,
                    signer_pubkey, created_at
             FROM attestations
             WHERE correlation_id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![correlation_id])?;
        match rows.next()? {
            Some(row) => Ok(Some((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))),
            None => Ok(None),
        }
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
        owner_pubkey: &str,
        created_at: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        // Explicit column list — `owner_pubkey` was added by
        // `migrate_owner_pubkey_columns` after the original table CREATE,
        // so the column count and order in `INSERT OR REPLACE INTO ...
        // VALUES (...)` would otherwise drift between fresh and migrated DBs.
        self.conn.execute(
            "INSERT OR REPLACE INTO attestations
                 (attestation_id, content, content_hash, tags,
                  solana_tx, arweave_tx, signer_pubkey, created_at, owner_pubkey)
             VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                attestation_id,
                content,
                content_hash,
                tags_json,
                solana_tx,
                arweave_tx,
                signer_pubkey,
                created_at,
                owner_pubkey,
            ],
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
             FROM attestations WHERE solana_tx = ?1 OR arweave_tx = ?1 LIMIT 1",
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
            params![signer],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    fn search(
        &self,
        query_embedding: &[f32],
        owner_pubkey: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        // Mandatory ownership filter (Decision 9). No carve-out; legacy rows
        // with NULL `owner_pubkey` will not match any caller.
        let mut stmt = self.conn.prepare(
            "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
                    a.solana_tx, a.arweave_tx, a.created_at, ae.embedding
             FROM attestations a
             JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
             WHERE a.owner_pubkey = ?",
        )?;

        let q_norm = l2_norm(query_embedding);
        let q_normalized: Vec<f32> = if q_norm > 0.0 {
            query_embedding.iter().map(|x| x / q_norm).collect()
        } else {
            query_embedding.to_vec()
        };

        let mut results: Vec<SearchResult> = stmt
            .query_map(params![owner_pubkey], |row| {
                let emb_blob: Vec<u8> = row.get(7)?;
                let emb = bytes_to_floats(&emb_blob);
                let e_norm = l2_norm(&emb);
                let score = if e_norm > 0.0 {
                    q_normalized
                        .iter()
                        .zip(emb.iter())
                        .map(|(a, b)| a * b / e_norm)
                        .sum::<f32>()
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
        let mut stmt = self
            .conn
            .prepare("SELECT parent_id, depth FROM lineage_edges WHERE child_id = ?")?;
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

    /// Test owner pubkey for in-file unit tests. After Task 4's signature
    /// change, every `save_attestation` callsite must pass an owner; using a
    /// constant here keeps the assertions identical to pre-migration semantics
    /// (single tenant, single owner) while exercising the new column.
    const TEST_OWNER: &str = "test-owner-pubkey";

    #[test]
    fn test_save_and_find_by_tx() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .save_attestation(
                "att-1",
                "content",
                "hash1",
                &["tag".into()],
                "sol_tx_1",
                "ar_tx_1",
                "signer1",
                TEST_OWNER,
                "2026-04-13T00:00:00Z",
                &[1.0, 0.0],
            )
            .unwrap();

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
            store
                .save_attestation(
                    &format!("att-{i}"),
                    "c",
                    "h",
                    &[],
                    "sol",
                    "ar",
                    "signer_a",
                    TEST_OWNER,
                    "2026-01-01",
                    &[1.0, 0.0],
                )
                .unwrap();
        }
        store
            .save_attestation(
                "att-other",
                "c",
                "h",
                &[],
                "sol2",
                "ar2",
                "signer_b",
                TEST_OWNER,
                "2026-01-01",
                &[1.0, 0.0],
            )
            .unwrap();

        // count() is unchanged — still by signer_pubkey, not owner.
        assert_eq!(store.count("signer_a").unwrap(), 2);
        assert_eq!(store.count("signer_b").unwrap(), 1);
    }

    #[test]
    fn test_search_ranking() {
        let store = SqliteStore::in_memory().unwrap();
        // Two attestations with distinct embeddings, both owned by the same
        // tenant so the post-Decision-9 owner filter does not drop them.
        store
            .save_attestation(
                "att-0",
                "topic zero",
                "h0",
                &[],
                "s0",
                "a0",
                "agent",
                "owner_agent",
                "2026-01-01",
                &[1.0, 0.0],
            )
            .unwrap();
        store
            .save_attestation(
                "att-1",
                "topic one",
                "h1",
                &[],
                "s1",
                "a1",
                "agent",
                "owner_agent",
                "2026-01-01",
                &[0.0, 1.0],
            )
            .unwrap();

        // Query closer to att-0's embedding; search is now scoped by
        // `owner_pubkey`, not by `signer_pubkey`.
        let results = store.search(&[1.0, 0.0], "owner_agent", 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].attestation_id, "att-0");
    }

    #[test]
    fn test_search_owner_isolation() {
        // Decision 9 — search must filter by owner, never leak across tenants.
        let store = SqliteStore::in_memory().unwrap();
        store
            .save_attestation(
                "att-alice",
                "alice's note",
                "h-a",
                &[],
                "s-a",
                "a-a",
                "signer_x",
                "owner_alice",
                "2026-01-01",
                &[1.0, 0.0],
            )
            .unwrap();
        store
            .save_attestation(
                "att-bob",
                "bob's note",
                "h-b",
                &[],
                "s-b",
                "a-b",
                "signer_x",
                "owner_bob",
                "2026-01-01",
                &[1.0, 0.0],
            )
            .unwrap();

        let alice_results = store.search(&[1.0, 0.0], "owner_alice", 10).unwrap();
        assert_eq!(alice_results.len(), 1);
        assert_eq!(alice_results[0].attestation_id, "att-alice");

        let bob_results = store.search(&[1.0, 0.0], "owner_bob", 10).unwrap();
        assert_eq!(bob_results.len(), 1);
        assert_eq!(bob_results[0].attestation_id, "att-bob");

        // Owner with no rows must return empty, even though same signer
        // produced rows under other owners.
        let unknown = store.search(&[1.0, 0.0], "owner_carol", 10).unwrap();
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_migrate_owner_pubkey_columns_idempotent() {
        // Decision 9 — migration must run cleanly twice in a row.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migration.db");
        let store = SqliteStore::open(&path).unwrap();
        // Re-running the migration on an already-migrated DB must be a no-op.
        super::migrate_owner_pubkey_columns(store.conn()).unwrap();
        super::migrate_owner_pubkey_columns(store.conn()).unwrap();

        // Both columns should be present.
        let attestations_cols: Vec<String> = store
            .conn()
            .prepare("PRAGMA table_info(attestations)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(attestations_cols.contains(&"owner_pubkey".to_string()));

        let api_keys_cols: Vec<String> = store
            .conn()
            .prepare("PRAGMA table_info(api_keys)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(api_keys_cols.contains(&"oauth_pubkey".to_string()));
    }

    #[test]
    fn test_migrate_backfills_legacy_null_owner_pubkey() {
        // Regression: legacy seed rows had NULL `owner_pubkey` and were hidden
        // from /chat's owner-scoped search, surfacing as "No relevant context
        // found." for every question. The migration must backfill them with
        // signer_pubkey so single-tenant data stays visible after upgrade.
        let store = SqliteStore::in_memory().unwrap();
        let server = "server_keypair";

        // Insert a row, then null out owner_pubkey to simulate a pre-OAuth row
        // (column existed but no value was written by the legacy save path).
        store
            .save_attestation(
                "att-legacy",
                "seeded chunk",
                "h-legacy",
                &[],
                "s-legacy",
                "a-legacy",
                server,
                server,
                "2026-01-01",
                &[1.0, 0.0],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE attestations SET owner_pubkey = NULL WHERE attestation_id = ?",
                params!["att-legacy"],
            )
            .unwrap();

        // Pre-migration: search by owner returns nothing.
        let before = store.search(&[1.0, 0.0], server, 10).unwrap();
        assert!(before.is_empty(), "NULL owner_pubkey must hide the row");

        // Re-run the migration on the live connection.
        super::migrate_owner_pubkey_columns(store.conn()).unwrap();

        // Post-migration: row is visible to its signer-as-owner.
        let after = store.search(&[1.0, 0.0], server, 10).unwrap();
        assert_eq!(after.len(), 1, "backfill must restore visibility");
        assert_eq!(after[0].attestation_id, "att-legacy");
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
        store
            .save_attestation(
                "att-dup",
                "c1",
                "h1",
                &[],
                "sol1",
                "ar1",
                "s1",
                TEST_OWNER,
                "2026-01-01",
                &[1.0],
            )
            .unwrap();
        // INSERT OR REPLACE so this succeeds (replaces)
        let result = store.save_attestation(
            "att-dup",
            "c2",
            "h2",
            &[],
            "sol2",
            "ar2",
            "s1",
            TEST_OWNER,
            "2026-01-01",
            &[1.0],
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

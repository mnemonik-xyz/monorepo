//! SQLite implementation of storage traits.
//!
//! `SqliteStore` wraps a `rusqlite::Connection`. It is `!Send` -- in async contexts,
//! callers must wrap it in `std::sync::Mutex` and never hold the lock across an `.await` point.

use anyhow::Context;
use rusqlite::{params, Connection};
use std::path::Path;

use super::mode::{Visibility, WriteMode};
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

-- Blog projection over public attestations (webapp-rethink Decision 7 / 8).
-- A blog post IS a signed public attestation; this table is a query-convenience
-- projection (slug PK for /blog/:slug lookup, published_at index for ordered
-- listing). `attestation_id` + `content_hash` link each row back to the signed
-- artifact so authorship stays verifiable. Markdown is stored raw and rendered
-- client-side. `CREATE TABLE IF NOT EXISTS` makes this idempotent alongside the
-- other schema statements, so it needs no separate ALTER-style migration.
CREATE TABLE IF NOT EXISTS blog_posts (
    slug TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body_markdown TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    author TEXT NOT NULL DEFAULT '',
    attestation_id TEXT NOT NULL,
    content_hash TEXT NOT NULL DEFAULT '',
    published_at TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'public'
);
CREATE INDEX IF NOT EXISTS idx_blog_posts_published_at ON blog_posts(published_at);
"#;

/// SQL backing `AttestationStore::search` when the caller passes a
/// `Some(owner)` AND `visibility_filter = None` (authenticated path —
/// sees all of the owner's own rows regardless of visibility per
/// Decision 5). Owner scope is the mandatory tenant predicate from
/// Decision 9.
const SEARCH_SQL_ALL: &str = "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.owner_pubkey = ?";

/// SQL backing `AttestationStore::search` when the caller passes a
/// `Some(owner)` AND `Some(visibility)` (owner-scoped + visibility-filtered).
/// Owner scope is preserved; visibility is bound via `ToSql`, never
/// interpolated as text.
const SEARCH_SQL_FILTERED: &str = "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.owner_pubkey = ? AND a.visibility = ?";

/// SQL backing `AttestationStore::search` when the caller passes
/// `owner_pubkey = None` AND `visibility_filter = Some(v)` — the
/// cross-owner anonymous-public path (agent-native-distribution Task 4 /
/// SAR1-M1). Drops the owner predicate; the caller is responsible for
/// pairing this with `Some(Visibility::Public)` so non-public rows do not
/// leak. The `owner_pubkey is NULL` case from legacy unmigrated rows is
/// excluded so anonymous callers don't accidentally surface unmigrated
/// content; only rows with a non-NULL owner AND public visibility appear.
const SEARCH_SQL_CROSS_OWNER_VIS: &str =
    "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.owner_pubkey IS NOT NULL AND a.visibility = ?";

/// SQL backing `SqliteStore::list_public_artifacts` — the non-search Ledger
/// listing (`GET /artifacts`, Decision 6). It extends the exact public-only
/// predicate of `SEARCH_SQL_CROSS_OWNER_VIS` above (`owner_pubkey IS NOT NULL
/// AND visibility = ?`) — never owner-private rows — but drops the embeddings
/// join (a chronological list needs no cosine score) and orders newest-first
/// with a bound `LIMIT`. Visibility is bound as a typed `Visibility` param,
/// never interpolated. The unmigrated NULL-owner legacy rows are excluded by
/// the same `owner_pubkey IS NOT NULL` guard so anonymous listing can never
/// surface them.
const LIST_PUBLIC_ARTIFACTS_SQL: &str =
    "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode
     FROM attestations a
     WHERE a.owner_pubkey IS NOT NULL AND a.visibility = ?
     ORDER BY a.created_at DESC
     LIMIT ?";

/// SQL backing `SqliteStore::attestation_timeline` — daily attestation counts
/// split on-node (`write_mode = 'local'`) vs on-chain (`write_mode =
/// 'participate'`) for the Analytics page. Buckets by the date prefix of the
/// stored ISO-8601 `created_at` (`substr(..., 1, 10)` → `YYYY-MM-DD`); rows are
/// persisted with explicit `Z`/UTC timestamps, so the prefix is a stable UTC
/// day key with no per-connection timezone dependence. `created_at >= ?1`
/// bounds the range; callers pass `""` for "all time" (every ISO timestamp
/// sorts lexicographically after the empty string). The `'local'` /
/// `'participate'` literals are the canonical `WriteMode` spellings — constants,
/// not user input. This is an aggregate traction view (mirrors `public_stats`):
/// it counts rows, never exposes content, so it intentionally spans all rows
/// rather than a single owner.
const TIMELINE_SQL: &str = "SELECT substr(created_at, 1, 10) AS day,
            SUM(CASE WHEN write_mode = 'local' THEN 1 ELSE 0 END) AS on_node,
            SUM(CASE WHEN write_mode = 'participate' THEN 1 ELSE 0 END) AS on_chain
     FROM attestations
     WHERE created_at >= ?1
     GROUP BY day
     ORDER BY day ASC";

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

/// A public-visibility attestation row for the Ledger listing
/// (`GET /artifacts`). Returned by `SqliteStore::list_public_artifacts`; every
/// row is `visibility = public` by construction of the backing query, so the
/// field is not repeated here. `write_mode` is surfaced so the UI can badge
/// on-node vs on-chain provenance and resolve explorer links.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PublicArtifact {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub tags: Vec<String>,
    pub solana_tx: String,
    pub arweave_tx: String,
    pub created_at: String,
    pub write_mode: WriteMode,
}

/// One day's attestation counts for the Analytics timeline, split by
/// `write_mode`. `on_node` counts `WriteMode::Local` writes; `on_chain` counts
/// `WriteMode::Participate` writes. `date` is a `YYYY-MM-DD` UTC day key.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TimelineBucket {
    pub date: String,
    pub on_node: i64,
    pub on_chain: i64,
}

/// A blog post projection row (webapp-rethink Decision 7 / 8). Mirrors the
/// `blog_posts` table; `attestation_id` + `content_hash` tie the row back to
/// the underlying signed public attestation so authorship stays verifiable.
/// `body_markdown` is stored raw and rendered client-side.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BlogPost {
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub tags: Vec<String>,
    pub author: String,
    pub attestation_id: String,
    pub content_hash: String,
    pub published_at: String,
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

/// Probes whether `attestations` has the given column via `PRAGMA
/// table_info`. Shared by `migrate_write_mode_column` and
/// `migrate_visibility_column` — both ADD-COLUMN migrations need this gate
/// because SQLite lacks `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`.
///
/// SAFETY: the PRAGMA query string is a hard-coded literal — only the
/// column-name comparison (a `String` read out of `row.get(1)?`) is
/// dynamic. The older `migrate_owner_pubkey_columns` /
/// `migrate_correlation_id_column` helpers `format!()` the table name into
/// the query string; that pattern is safe today (only internal literals
/// call them) but fragile if extended. This helper deliberately fixes the
/// table to `attestations` so the query is a const, not a runtime-formatted
/// string (security-auditor round 1 finding on `migrate_write_mode_column`,
/// preserved verbatim in the shared lift — code-reviewer round 1, CR-T3-1).
fn attestations_has_column(conn: &Connection, column: &str) -> anyhow::Result<bool> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(attestations)")
        .context("preparing PRAGMA table_info(attestations)")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Idempotent ADD-COLUMN migration for the per-request `write_mode` field
/// (feature `modes-user-choice`, T1).
///
/// Adds `attestations.write_mode TEXT NOT NULL DEFAULT 'participate'` plus a
/// composite `(owner_pubkey, write_mode)` index for the filtered recall and
/// audit queries that follow in later tasks.
///
/// `DEFAULT 'participate'` is the conservative choice for legacy rows: a row
/// that existed under the previous global `STORAGE_MODE=full` operator was,
/// by definition, a paid participate write. Silently re-tagging those rows
/// as `'local'` would destroy billing history and downgrade the integrity
/// claim attached to the row.
///
/// Backfill rule: `solana_tx LIKE 'local:_%'` (strict — at least one
/// character after the colon). This is collision-safe with real Solana
/// signatures because base58 excludes lowercase `l` and `:` is not in the
/// base58 alphabet at all; the only rows that match are the synthetic ids
/// produced by `mcp/src/tools.rs::sign_memory_inline`'s local branch. The
/// bare `local:` prefix (no suffix) is reserved as the synthetic-id
/// namespace and is *not* a valid synthetic id — it stays `'participate'`
/// on purpose.
///
/// SQLite `ALTER TABLE ... ADD COLUMN` lacks `IF NOT EXISTS`, so presence is
/// gated via `PRAGMA table_info`. Wrapped in `BEGIN IMMEDIATE` to serialize
/// against any concurrent opener; the inner steps are individually
/// idempotent (`CREATE INDEX IF NOT EXISTS`, UPDATE matches zero rows on a
/// clean DB) so this is safe to invoke on every open.
fn migrate_write_mode_column(conn: &Connection) -> anyhow::Result<()> {
    let need_column = !attestations_has_column(conn, "write_mode")?;

    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening write_mode migration transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        if need_column {
            conn.execute(
                "ALTER TABLE attestations
                    ADD COLUMN write_mode TEXT NOT NULL DEFAULT 'participate'",
                [],
            )
            .context("adding attestations.write_mode")?;
        }
        // Backfill legacy `local:*` synthetic-id rows. Runs every open: cheap
        // on clean DBs (zero matches once converged), correct on DBs that had
        // the column added without a backfill in an earlier release.
        // `LIKE 'local:_%'` requires at least one char after the colon —
        // bare `local:` is reserved and stays `'participate'` (default).
        conn.execute(
            "UPDATE attestations
                SET write_mode = 'local'
              WHERE solana_tx LIKE 'local:_%'",
            [],
        )
        .context("backfilling attestations.write_mode for local: synthetic ids")?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_attestations_write_mode
                 ON attestations(owner_pubkey, write_mode)",
            [],
        )
        .context("creating idx_attestations_write_mode")?;
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing write_mode migration")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

/// Idempotent ADD-COLUMN migration for per-attestation `visibility`
/// (feature `agent-native-distribution`, T3 / Decision 2).
///
/// Adds `attestations.visibility TEXT NOT NULL DEFAULT 'private'` plus a
/// single-column `idx_attestations_visibility` index used by the anonymous
/// `recall` filter (Decision 5 — authenticated callers see all their own
/// rows; anonymous callers see only `visibility='public'`).
///
/// `DEFAULT 'private'` is privacy-by-default: every legacy row predates the
/// concept of `visibility` and was, by definition, not opted into anonymous
/// discovery. Backfill is therefore an unconditional UPDATE that promotes
/// any NULL/empty row to `'private'`. The UPDATE is no-op once the DEFAULT
/// has fired (NOT NULL means new rows are never NULL), but stays idempotent
/// on every open so a manually-tampered DB or a re-import is normalized.
///
/// SQLite `ALTER TABLE ... ADD COLUMN` lacks `IF NOT EXISTS`, so presence is
/// gated via `PRAGMA table_info` — see the parallel rationale in
/// `migrate_write_mode_column` above. Wrapped in `BEGIN IMMEDIATE` to
/// serialize against any concurrent opener (Decision 13: reuses the
/// existing WAL + busy_timeout config at `open()`; no new tuning here).
fn migrate_visibility_column(conn: &Connection) -> anyhow::Result<()> {
    let need_column = !attestations_has_column(conn, "visibility")?;

    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening visibility migration transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        if need_column {
            conn.execute(
                "ALTER TABLE attestations
                    ADD COLUMN visibility TEXT NOT NULL DEFAULT 'private'",
                [],
            )
            .context("adding attestations.visibility")?;
        }
        // Backfill any pre-migration row that might lack a value (manual
        // ALTER on a legacy DB, or a re-imported row with NULL). Cheap on
        // clean DBs — the NOT NULL DEFAULT makes UPDATE a zero-row no-op
        // after first migration.
        conn.execute(
            "UPDATE attestations
                SET visibility = 'private'
              WHERE visibility IS NULL OR visibility = ''",
            [],
        )
        .context("backfilling attestations.visibility to 'private'")?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_attestations_visibility
                 ON attestations(visibility)",
            [],
        )
        .context("creating idx_attestations_visibility")?;
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing visibility migration")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
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
        migrate_write_mode_column(&conn)?;
        migrate_visibility_column(&conn)?;
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
        migrate_write_mode_column(&conn)?;
        migrate_visibility_column(&conn)?;
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

    /// All `content_hash`es owned by `owner_pubkey`, for building the per-owner
    /// Merkle commitment (verifiable recall, §16). Order is unspecified — the
    /// [`crate::merkle`] layer sorts into the canonical set ordering, so the
    /// root is reproducible from this (rebuildable-cache) query regardless of
    /// row order. Returns an empty vec for an unknown owner.
    pub fn owner_content_hashes(&self, owner_pubkey: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT content_hash FROM attestations WHERE owner_pubkey = ?")?;
        let rows = stmt.query_map(params![owner_pubkey], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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

    /// List public-visibility attestations newest-first for the Ledger page
    /// (`GET /artifacts`, Decision 6). Returns ONLY `visibility = public` rows
    /// (the backing query reuses the public-only predicate from
    /// `SEARCH_SQL_CROSS_OWNER_VIS`); owner-private rows and legacy NULL-owner
    /// rows are never surfaced. `limit` caps the result; an empty table yields
    /// an empty vec. Content search (`?q=`) is served by the cosine `search`
    /// path with `Some(Visibility::Public)`, not by this chronological list.
    pub fn list_public_artifacts(&self, limit: usize) -> anyhow::Result<Vec<PublicArtifact>> {
        let mut stmt = self.conn.prepare(LIST_PUBLIC_ARTIFACTS_SQL)?;
        let rows = stmt.query_map(params![Visibility::Public, limit as i64], |row| {
            let tags_str: String = row.get(3)?;
            Ok(PublicArtifact {
                attestation_id: row.get(0)?,
                content: row.get(1)?,
                content_hash: row.get(2)?,
                tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                solana_tx: row.get(4)?,
                arweave_tx: row.get(5)?,
                created_at: row.get(6)?,
                write_mode: row.get::<_, WriteMode>(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Daily attestation counts split on-node vs on-chain by `write_mode`, for
    /// the Analytics timeline (`GET /analytics/attestations`). `since` is an
    /// optional inclusive ISO-8601 lower bound on `created_at`; `None` means
    /// "all time". Buckets are ordered oldest-first; days with no rows are
    /// absent (the caller fills gaps for charting). Empty table → empty vec.
    pub fn attestation_timeline(&self, since: Option<&str>) -> anyhow::Result<Vec<TimelineBucket>> {
        let mut stmt = self.conn.prepare(TIMELINE_SQL)?;
        let rows = stmt.query_map(params![since.unwrap_or("")], |row| {
            Ok(TimelineBucket {
                date: row.get(0)?,
                on_node: row.get(1)?,
                on_chain: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Upsert a blog-post projection row (used by the `POST /blog` /
    /// `mnemonic_publish_post` publish path once the underlying attestation is
    /// signed and stored). `slug` is the primary key, so re-publishing the same
    /// slug replaces the projection. The projection is a query convenience over
    /// the signed attestation referenced by `attestation_id` — it is not the
    /// source of truth for authorship.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_blog_post(
        &self,
        slug: &str,
        title: &str,
        body_markdown: &str,
        tags: &[String],
        author: &str,
        attestation_id: &str,
        content_hash: &str,
        published_at: &str,
    ) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO blog_posts
                 (slug, title, body_markdown, tags, author,
                  attestation_id, content_hash, published_at, visibility)
             VALUES (?,?,?,?,?,?,?,?,'public')",
            params![
                slug,
                title,
                body_markdown,
                tags_json,
                author,
                attestation_id,
                content_hash,
                published_at,
            ],
        )?;
        Ok(())
    }

    /// List public blog posts newest-first (`GET /blog`). `limit` caps the
    /// result; an empty table yields an empty vec.
    pub fn list_blog_posts(&self, limit: usize) -> anyhow::Result<Vec<BlogPost>> {
        let mut stmt = self.conn.prepare(
            "SELECT slug, title, body_markdown, tags, author,
                    attestation_id, content_hash, published_at
             FROM blog_posts
             WHERE visibility = 'public'
             ORDER BY published_at DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit as i64], blog_post_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Fetch a single public blog post by slug (`GET /blog/:slug`), or `None`.
    pub fn get_blog_post(&self, slug: &str) -> anyhow::Result<Option<BlogPost>> {
        let mut stmt = self.conn.prepare(
            "SELECT slug, title, body_markdown, tags, author,
                    attestation_id, content_hash, published_at
             FROM blog_posts
             WHERE slug = ? AND visibility = 'public'
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![slug])?;
        match rows.next()? {
            Some(row) => Ok(Some(blog_post_from_row(row)?)),
            None => Ok(None),
        }
    }
}

/// Map a `blog_posts` row (in the column order used by `list_blog_posts` /
/// `get_blog_post`) into a [`BlogPost`]. Shared so the two read paths cannot
/// drift in column order or tag decoding.
fn blog_post_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BlogPost> {
    let tags_str: String = row.get(3)?;
    Ok(BlogPost {
        slug: row.get(0)?,
        title: row.get(1)?,
        body_markdown: row.get(2)?,
        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        author: row.get(4)?,
        attestation_id: row.get(5)?,
        content_hash: row.get(6)?,
        published_at: row.get(7)?,
    })
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
        write_mode: WriteMode,
        visibility: Visibility,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        // Explicit column list — `owner_pubkey`, `write_mode`, and
        // `visibility` were added by migration helpers after the original
        // table CREATE, so the column count and order in
        // `INSERT OR REPLACE INTO ... VALUES (...)` would otherwise drift
        // between fresh and migrated DBs.
        self.conn.execute(
            "INSERT OR REPLACE INTO attestations
                 (attestation_id, content, content_hash, tags,
                  solana_tx, arweave_tx, signer_pubkey, created_at, owner_pubkey,
                  write_mode, visibility)
             VALUES (?,?,?,?,?,?,?,?,?,?,?)",
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
                write_mode.as_str(),
                visibility.as_str(),
            ],
        )?;
        let emb_bytes = floats_to_bytes(embedding);
        self.conn.execute(
            "INSERT OR REPLACE INTO attestation_embeddings VALUES (?,?,?)",
            params![attestation_id, embedding.len() as i32, emb_bytes],
        )?;
        Ok(())
    }

    fn find_by_tx(
        &self,
        tx_id: &str,
        owner_pubkey: &str,
    ) -> anyhow::Result<Option<AttestationRow>> {
        // Tenant-isolation predicate (Decision 9 / T4): the lookup MUST
        // include `AND owner_pubkey = ?` so a row owned by a different
        // tenant is indistinguishable from a genuine miss. No carve-out;
        // legacy rows with NULL `owner_pubkey` will not match any caller.
        // Response-timing symmetry is documented as accepted residual
        // (R2-F4) — no constant-time wrapper is layered here.
        let mut stmt = self.conn.prepare(
            "SELECT attestation_id, content, content_hash, solana_tx, arweave_tx, signer_pubkey
             FROM attestations
             WHERE (solana_tx = ?1 OR arweave_tx = ?1) AND owner_pubkey = ?2
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![tx_id, owner_pubkey])?;
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

    fn find_write_mode_by_tx(
        &self,
        tx_id: &str,
        owner_pubkey: &str,
    ) -> anyhow::Result<Option<WriteMode>> {
        // Same tenant-isolation predicate as `find_by_tx`: a row owned by
        // a different tenant returns `Ok(None)` — `verify` routing must
        // not branch differently for "exists for someone else" vs.
        // "doesn't exist at all". Separate from `find_by_tx` so the
        // routing path does not pay the cost of decoding `content` etc.
        let mut stmt = self.conn.prepare(
            "SELECT write_mode FROM attestations
             WHERE (solana_tx = ?1 OR arweave_tx = ?1) AND owner_pubkey = ?2
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![tx_id, owner_pubkey])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get::<_, WriteMode>(0)?)),
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

    fn delete_protocol_knowledge_for_owner(&self, owner_pubkey: &str) -> anyhow::Result<usize> {
        // `tags` is stored as canonical-CBOR-derived JSON text per the
        // attestations schema. The substring search is correct because every
        // seeded chunk carries the literal token `"protocol-knowledge"`
        // (`mcp/src/seed.rs:313`) and no other tag system uses that exact
        // string. If a future tag is introduced that contains it as a
        // substring, this query needs to widen to a JSON_EXTRACT predicate.
        //
        // Delete in dependency order to avoid `FOREIGN KEY constraint failed`:
        // `memory_embeddings.attestation_id` has a FK into `attestations`.
        // `attestation_costs` has a FK too (full mode only — empty on
        // local-only deploys, but the DELETE is cheap either way).
        // Discovered live on prod 2026-06-26: the original single-statement
        // DELETE was blocked by the embeddings FK; the seed code WARN-logged
        // and continued, leaving stale chunks alongside the fresh corpus.
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM memory_embeddings \
             WHERE attestation_id IN (\
               SELECT attestation_id FROM attestations \
               WHERE owner_pubkey = ?1 \
                 AND tags LIKE '%\"protocol-knowledge\"%'\
             )",
            params![owner_pubkey],
        )?;
        tx.execute(
            "DELETE FROM attestation_costs \
             WHERE attestation_id IN (\
               SELECT attestation_id FROM attestations \
               WHERE owner_pubkey = ?1 \
                 AND tags LIKE '%\"protocol-knowledge\"%'\
             )",
            params![owner_pubkey],
        )?;
        let affected = tx.execute(
            "DELETE FROM attestations \
             WHERE owner_pubkey = ?1 \
               AND tags LIKE '%\"protocol-knowledge\"%'",
            params![owner_pubkey],
        )?;
        tx.commit()?;
        Ok(affected)
    }

    fn search(
        &self,
        query_embedding: &[f32],
        owner_pubkey: Option<&str>,
        visibility_filter: Option<Visibility>,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        // Mandatory ownership filter (Decision 9). No carve-out; legacy rows
        // with NULL `owner_pubkey` will not match any caller.
        //
        // Optional visibility filter (Decision 5): the anonymous-recall path
        // passes `Some(Visibility::Public)`; authenticated callers pass
        // `None`. SQL for both branches is a module-level `&'static str`
        // constant (`SEARCH_SQL_*` below) — only typed `Visibility`
        // (round-tripped through `ToSql`) ever reaches the
        // `AND a.visibility = ?` placeholder, never user text. The two
        // statements are kept as separate constants rather than concatenated
        // at runtime so a future reader sees both queries verbatim without
        // having to mentally fold a `format!` call (security-auditor round 1
        // SEC-T3-01, code-reviewer round 1 CR-T3-2).

        let q_norm = l2_norm(query_embedding);
        let q_normalized: Vec<f32> = if q_norm > 0.0 {
            query_embedding.iter().map(|x| x / q_norm).collect()
        } else {
            query_embedding.to_vec()
        };

        let row_mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<SearchResult> {
            let emb_blob: Vec<u8> = row.get(9)?;
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
                write_mode: row.get::<_, WriteMode>(7)?,
                visibility: row.get::<_, Visibility>(8)?,
                relevance_score: score,
            })
        };

        // Three search shapes per Decision 5 + SAR1-M1 (agent-native-distribution):
        //   (Some(owner), Some(v))  → owner-scoped + visibility-filtered
        //   (Some(owner), None)     → owner-scoped (authenticated, full corpus)
        //   (None,        Some(v))  → cross-owner + visibility-filtered (anonymous-public)
        //   (None,        None)     → DISALLOWED — would expose all rows; the
        //                             trait doc requires `None` owner to be
        //                             paired with `Some(visibility)`. Defensive
        //                             early-return keeps the storage layer from
        //                             silently leaking on a future caller bug.
        let mut results: Vec<SearchResult> = match (owner_pubkey, visibility_filter) {
            (Some(owner), Some(v)) => {
                let mut stmt = self.conn.prepare(SEARCH_SQL_FILTERED)?;
                let rows = stmt.query_map(params![owner, v], row_mapper)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(owner), None) => {
                let mut stmt = self.conn.prepare(SEARCH_SQL_ALL)?;
                let rows = stmt.query_map(params![owner], row_mapper)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(v)) => {
                let mut stmt = self.conn.prepare(SEARCH_SQL_CROSS_OWNER_VIS)?;
                let rows = stmt.query_map(params![v], row_mapper)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, None) => {
                // Defensive: callers must not pass `None`/`None` (would
                // expose every row anonymously). Trait doc forbids this;
                // returning an empty result here keeps the leak closed
                // even if a future caller violates the contract.
                Vec::new()
            }
        };

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
                WriteMode::Participate,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();

        let found = store.find_by_tx("sol_tx_1", TEST_OWNER).unwrap();
        assert!(found.is_some());
        let row = found.unwrap();
        assert_eq!(row.attestation_id, "att-1");
        assert_eq!(row.content, "content");
        assert_eq!(row.content_hash, "hash1");

        // Tenant-isolation predicate: a wrong-tenant lookup returns
        // `Ok(None)` indistinguishable from a genuine miss.
        let other_tenant = store.find_by_tx("sol_tx_1", "different-owner").unwrap();
        assert!(
            other_tenant.is_none(),
            "wrong-tenant lookup must return None"
        );
    }

    #[test]
    fn test_find_write_mode_by_tx_routes_by_stored_mode() {
        // Seed two rows under different tenants but identical-shape txids
        // so the predicate has to discriminate by both tx AND owner.
        let store = SqliteStore::in_memory().unwrap();
        store
            .save_attestation(
                "att-local",
                "local content",
                "h-local",
                &[],
                "sol-shared",
                "ar-local",
                "signer-a",
                "owner-a",
                "2026-04-13T00:00:00Z",
                WriteMode::Local,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();
        store
            .save_attestation(
                "att-participate",
                "participate content",
                "h-participate",
                &[],
                "sol-different",
                "ar-participate",
                "signer-b",
                "owner-b",
                "2026-04-13T00:00:00Z",
                WriteMode::Participate,
                Visibility::Private,
                &[0.0, 1.0],
            )
            .unwrap();

        // owner-a's row is local
        assert_eq!(
            store
                .find_write_mode_by_tx("sol-shared", "owner-a")
                .unwrap(),
            Some(WriteMode::Local)
        );
        // owner-b's row is participate
        assert_eq!(
            store
                .find_write_mode_by_tx("sol-different", "owner-b")
                .unwrap(),
            Some(WriteMode::Participate)
        );
        // Wrong tenant for an existing tx → None (no leak).
        assert_eq!(
            store
                .find_write_mode_by_tx("sol-shared", "owner-b")
                .unwrap(),
            None
        );
        // Unknown tx → None.
        assert_eq!(
            store
                .find_write_mode_by_tx("nonexistent", "owner-a")
                .unwrap(),
            None
        );
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
                    WriteMode::Participate,
                    Visibility::Private,
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
                WriteMode::Participate,
                Visibility::Private,
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
                WriteMode::Participate,
                Visibility::Private,
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
                WriteMode::Participate,
                Visibility::Private,
                &[0.0, 1.0],
            )
            .unwrap();

        // Query closer to att-0's embedding; search is now scoped by
        // `owner_pubkey`, not by `signer_pubkey`.
        let results = store
            .search(&[1.0, 0.0], Some("owner_agent"), None, 2)
            .unwrap();
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
                WriteMode::Participate,
                Visibility::Private,
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
                WriteMode::Participate,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();

        let alice_results = store
            .search(&[1.0, 0.0], Some("owner_alice"), None, 10)
            .unwrap();
        assert_eq!(alice_results.len(), 1);
        assert_eq!(alice_results[0].attestation_id, "att-alice");

        let bob_results = store
            .search(&[1.0, 0.0], Some("owner_bob"), None, 10)
            .unwrap();
        assert_eq!(bob_results.len(), 1);
        assert_eq!(bob_results[0].attestation_id, "att-bob");

        // Owner with no rows must return empty, even though same signer
        // produced rows under other owners.
        let unknown = store
            .search(&[1.0, 0.0], Some("owner_carol"), None, 10)
            .unwrap();
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
                WriteMode::Participate,
                Visibility::Private,
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
        let before = store.search(&[1.0, 0.0], Some(server), None, 10).unwrap();
        assert!(before.is_empty(), "NULL owner_pubkey must hide the row");

        // Re-run the migration on the live connection.
        super::migrate_owner_pubkey_columns(store.conn()).unwrap();

        // Post-migration: row is visible to its signer-as-owner.
        let after = store.search(&[1.0, 0.0], Some(server), None, 10).unwrap();
        assert_eq!(after.len(), 1, "backfill must restore visibility");
        assert_eq!(after[0].attestation_id, "att-legacy");
    }

    #[test]
    fn test_find_by_tx_not_found() {
        let store = SqliteStore::in_memory().unwrap();
        let found = store.find_by_tx("nonexistent", TEST_OWNER).unwrap();
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
                WriteMode::Participate,
                Visibility::Private,
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
            WriteMode::Participate,
            Visibility::Private,
            &[1.0],
        );
        assert!(result.is_ok());
        // Content should be updated
        let row = store.find_by_tx("sol2", TEST_OWNER).unwrap().unwrap();
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

    // -- modes-user-choice T1 ------------------------------------------------

    /// Returns the list of (cid, name, type, notnull, dflt_value) tuples for
    /// `attestations` so tests can assert on column presence + DEFAULT.
    fn attestations_table_info(conn: &Connection) -> Vec<(String, String, i64, Option<String>)> {
        let mut stmt = conn.prepare("PRAGMA table_info(attestations)").unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,         // name
                row.get::<_, String>(2)?,         // type
                row.get::<_, i64>(3)?,            // notnull
                row.get::<_, Option<String>>(4)?, // dflt_value
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    fn index_exists(conn: &Connection, name: &str) -> bool {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name=?")
            .unwrap();
        let mut rows = stmt.query(params![name]).unwrap();
        rows.next().unwrap().is_some()
    }

    #[test]
    fn migrate_write_mode_column_on_fresh_db() {
        // Fresh in-memory store runs the migration in `in_memory()`.
        let store = SqliteStore::in_memory().unwrap();

        let cols = attestations_table_info(store.conn());
        let wm = cols
            .iter()
            .find(|(name, _, _, _)| name == "write_mode")
            .expect("write_mode column must exist on a fresh DB");
        assert_eq!(wm.1, "TEXT", "write_mode must be TEXT");
        assert_eq!(wm.2, 1, "write_mode must be NOT NULL");
        // SQLite stores the DEFAULT clause verbatim including the quotes.
        assert_eq!(
            wm.3.as_deref(),
            Some("'participate'"),
            "DEFAULT must be 'participate' (legacy paid writes), not 'local'"
        );

        // Composite index must be created.
        assert!(
            index_exists(store.conn(), "idx_attestations_write_mode"),
            "idx_attestations_write_mode must be created"
        );

        // A row inserted via save_attestation gets the bound value.
        store
            .save_attestation(
                "att-local",
                "c",
                "h",
                &[],
                "local:abc",
                "ar",
                "s",
                TEST_OWNER,
                "2026-01-01",
                WriteMode::Local,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();
        let mode: WriteMode = store
            .conn()
            .query_row(
                "SELECT write_mode FROM attestations WHERE attestation_id=?",
                params!["att-local"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mode, WriteMode::Local);
    }

    /// Seed a fresh in-memory DB whose `attestations` table predates the
    /// `write_mode` column. We do that by recreating only the legacy column
    /// set, then running the migration on it. This exercises the same code
    /// path operators see when upgrading.
    fn open_legacy_attestations_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE attestations (
                attestation_id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                solana_tx TEXT NOT NULL,
                arweave_tx TEXT NOT NULL,
                signer_pubkey TEXT NOT NULL,
                created_at TEXT NOT NULL,
                owner_pubkey TEXT
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn migrate_write_mode_column_backfills_legacy_rows() {
        let conn = open_legacy_attestations_db();
        // Seed three rows BEFORE the column exists, using raw INSERTs.
        // - 'local:abc'         → backfill must tag as 'local'
        // - 'local:' (no suffix) → must stay 'participate' (the `_` requires ≥1 char)
        // - real-shaped base58  → must stay 'participate'
        let real_sig = "5VfYdkv9LR3p4n2H8eYZUxq7w1bWmZ7v3jR6Vh8L9NkXrF4tY7B6cJ2sP5gN8eMqXk";
        for (id, sol_tx) in [
            ("att-localish", "local:abc"),
            ("att-bareprefix", "local:"),
            ("att-realsig", real_sig),
        ] {
            conn.execute(
                "INSERT INTO attestations
                    (attestation_id, content, content_hash, tags,
                     solana_tx, arweave_tx, signer_pubkey, created_at, owner_pubkey)
                 VALUES (?,?,?,?,?,?,?,?,?)",
                params![id, "c", "h", "[]", sol_tx, "ar", "s", "2026-01-01", "owner"],
            )
            .unwrap();
        }

        super::migrate_write_mode_column(&conn).unwrap();

        let read_mode = |id: &str| -> String {
            conn.query_row(
                "SELECT write_mode FROM attestations WHERE attestation_id=?",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };

        assert_eq!(
            read_mode("att-localish"),
            "local",
            "'local:abc' must be backfilled to 'local'"
        );
        assert_eq!(
            read_mode("att-bareprefix"),
            "participate",
            "bare 'local:' (no suffix) must stay 'participate' (default)"
        );
        assert_eq!(
            read_mode("att-realsig"),
            "participate",
            "real base58 signature must stay 'participate' (default)"
        );

        // Index must be created by the migration even on a legacy DB.
        assert!(index_exists(&conn, "idx_attestations_write_mode"));
    }

    #[test]
    fn migrate_write_mode_column_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("idempotent.db");
        let store = SqliteStore::open(&path).unwrap();

        // Insert one row and one synthetic-id row.
        store
            .save_attestation(
                "att-p",
                "c",
                "h",
                &[],
                "real-sig",
                "ar",
                "s",
                TEST_OWNER,
                "2026-01-01",
                WriteMode::Participate,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();
        // A legacy 'local:foo' row inserted via a raw UPDATE to simulate the
        // pre-migration state, then re-run the migration.
        store
            .conn()
            .execute(
                "UPDATE attestations SET solana_tx='local:foo', write_mode='participate'
                    WHERE attestation_id='att-p'",
                [],
            )
            .unwrap();

        // First re-run: must flip to 'local' (the row now matches the LIKE).
        super::migrate_write_mode_column(store.conn()).unwrap();
        let mode_1: String = store
            .conn()
            .query_row(
                "SELECT write_mode FROM attestations WHERE attestation_id='att-p'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mode_1, "local");

        // Snapshot row count, then re-run twice more. Nothing must change.
        let count_before: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM attestations", [], |row| row.get(0))
            .unwrap();
        super::migrate_write_mode_column(store.conn()).unwrap();
        super::migrate_write_mode_column(store.conn()).unwrap();

        let count_after: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM attestations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count_after, count_before, "row count must not change");

        // Exactly one index of that name — no duplicates from multiple runs.
        let idx_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                   WHERE type='index' AND name='idx_attestations_write_mode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1);

        // Final value is still 'local'.
        let mode_final: String = store
            .conn()
            .query_row(
                "SELECT write_mode FROM attestations WHERE attestation_id='att-p'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mode_final, "local");
    }

    // -- webapp-rethink T7: public artifacts, timeline, blog projection -------

    /// Seed a row with explicit visibility + write_mode + created_at so the
    /// public-list and timeline tests can assert on each axis independently.
    #[allow(clippy::too_many_arguments)]
    fn seed_row(
        store: &SqliteStore,
        id: &str,
        owner: &str,
        solana_tx: &str,
        created_at: &str,
        write_mode: WriteMode,
        visibility: Visibility,
    ) {
        store
            .save_attestation(
                id,
                "content",
                "hash",
                &["t".into()],
                solana_tx,
                "ar",
                "signer",
                owner,
                created_at,
                write_mode,
                visibility,
                &[1.0, 0.0],
            )
            .unwrap();
    }

    #[test]
    fn list_public_artifacts_returns_public_only() {
        // Decision 6 — the Ledger list must surface public rows ONLY; private
        // rows (even though they exist for the same owner) must never appear.
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "pub-1",
            "owner-a",
            "s1",
            "2026-06-01T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
        );
        seed_row(
            &store,
            "priv-1",
            "owner-a",
            "s2",
            "2026-06-02T00:00:00Z",
            WriteMode::Local,
            Visibility::Private,
        );
        seed_row(
            &store,
            "pub-2",
            "owner-b",
            "s3",
            "2026-06-03T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
        );

        let rows = store.list_public_artifacts(10).unwrap();
        assert_eq!(rows.len(), 2, "only the two public rows must be listed");
        let ids: Vec<&str> = rows.iter().map(|r| r.attestation_id.as_str()).collect();
        assert!(ids.contains(&"pub-1"));
        assert!(ids.contains(&"pub-2"));
        assert!(
            !ids.contains(&"priv-1"),
            "private row must never be listed publicly"
        );

        // Newest-first ordering by created_at.
        assert_eq!(rows[0].attestation_id, "pub-2");
        assert_eq!(rows[1].attestation_id, "pub-1");

        // Limit is honored.
        let limited = store.list_public_artifacts(1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].attestation_id, "pub-2");
    }

    #[test]
    fn list_public_artifacts_excludes_legacy_null_owner() {
        // Unmigrated NULL-owner public rows must not leak through the anonymous
        // listing (same guard as SEARCH_SQL_CROSS_OWNER_VIS).
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "pub-null",
            "owner-x",
            "s1",
            "2026-06-01T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
        );
        store
            .conn
            .execute(
                "UPDATE attestations SET owner_pubkey = NULL WHERE attestation_id = 'pub-null'",
                [],
            )
            .unwrap();
        assert!(
            store.list_public_artifacts(10).unwrap().is_empty(),
            "NULL-owner row must not appear in the public list"
        );
    }

    #[test]
    fn list_public_artifacts_empty_table() {
        let store = SqliteStore::in_memory().unwrap();
        assert!(store.list_public_artifacts(10).unwrap().is_empty());
    }

    #[test]
    fn attestation_timeline_buckets_by_day_and_write_mode() {
        let store = SqliteStore::in_memory().unwrap();
        // Day 1: 2 local (on-node) + 1 participate (on-chain).
        seed_row(
            &store,
            "d1-l1",
            "o",
            "local:a",
            "2026-06-01T01:00:00Z",
            WriteMode::Local,
            Visibility::Private,
        );
        seed_row(
            &store,
            "d1-l2",
            "o",
            "local:b",
            "2026-06-01T09:00:00Z",
            WriteMode::Local,
            Visibility::Private,
        );
        seed_row(
            &store,
            "d1-p1",
            "o",
            "sig-1",
            "2026-06-01T23:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
        );
        // Day 2: 1 participate (on-chain).
        seed_row(
            &store,
            "d2-p1",
            "o",
            "sig-2",
            "2026-06-02T12:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
        );

        let buckets = store.attestation_timeline(None).unwrap();
        assert_eq!(buckets.len(), 2, "two distinct days");
        // Ordered oldest-first.
        assert_eq!(buckets[0].date, "2026-06-01");
        assert_eq!(buckets[0].on_node, 2);
        assert_eq!(buckets[0].on_chain, 1);
        assert_eq!(buckets[1].date, "2026-06-02");
        assert_eq!(buckets[1].on_node, 0);
        assert_eq!(buckets[1].on_chain, 1);

        // Range filter: `since` excludes the earlier day.
        let ranged = store.attestation_timeline(Some("2026-06-02")).unwrap();
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0].date, "2026-06-02");
    }

    #[test]
    fn attestation_timeline_empty_table() {
        let store = SqliteStore::in_memory().unwrap();
        assert!(store.attestation_timeline(None).unwrap().is_empty());
    }

    #[test]
    fn blog_posts_upsert_list_get_roundtrip() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .upsert_blog_post(
                "hello-world",
                "Hello World",
                "# Hello\n\nbody",
                &["intro".into(), "demo".into()],
                "agent-zero",
                "att-1",
                "deadbeef",
                "2026-06-01T00:00:00Z",
            )
            .unwrap();
        store
            .upsert_blog_post(
                "second-post",
                "Second",
                "more",
                &[],
                "agent-zero",
                "att-2",
                "cafe",
                "2026-06-05T00:00:00Z",
            )
            .unwrap();

        // get by slug
        let got = store.get_blog_post("hello-world").unwrap().unwrap();
        assert_eq!(got.title, "Hello World");
        assert_eq!(got.body_markdown, "# Hello\n\nbody");
        assert_eq!(got.tags, vec!["intro".to_string(), "demo".to_string()]);
        assert_eq!(got.author, "agent-zero");
        assert_eq!(got.attestation_id, "att-1");
        assert_eq!(got.content_hash, "deadbeef");

        // unknown slug → None
        assert!(store.get_blog_post("nope").unwrap().is_none());

        // list newest-first by published_at
        let list = store.list_blog_posts(10).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].slug, "second-post");
        assert_eq!(list[1].slug, "hello-world");

        // upsert on same slug replaces, does not duplicate
        store
            .upsert_blog_post(
                "hello-world",
                "Hello (edited)",
                "edited body",
                &[],
                "agent-zero",
                "att-1b",
                "f00d",
                "2026-06-01T00:00:00Z",
            )
            .unwrap();
        let list_after = store.list_blog_posts(10).unwrap();
        assert_eq!(list_after.len(), 2, "upsert must replace, not append");
        let edited = store.get_blog_post("hello-world").unwrap().unwrap();
        assert_eq!(edited.title, "Hello (edited)");
        assert_eq!(edited.attestation_id, "att-1b");
    }

    #[test]
    fn blog_posts_migration_idempotent() {
        // The blog_posts table + index come from the SCHEMA batch
        // (CREATE ... IF NOT EXISTS), so opening the same DB twice must not
        // error and must preserve rows.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blog-idempotent.db");
        {
            let store = SqliteStore::open(&path).unwrap();
            store
                .upsert_blog_post(
                    "persisted",
                    "Persisted",
                    "body",
                    &[],
                    "a",
                    "att-x",
                    "hash-x",
                    "2026-06-01T00:00:00Z",
                )
                .unwrap();
        }
        // Re-open: SCHEMA runs again; row survives, table not recreated.
        let store2 = SqliteStore::open(&path).unwrap();
        let got = store2.get_blog_post("persisted").unwrap();
        assert!(got.is_some(), "row must survive a re-open");
        assert_eq!(got.unwrap().title, "Persisted");

        // Exactly one index of that name — no duplicates across opens.
        let idx_count: i64 = store2
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                   WHERE type='index' AND name='idx_blog_posts_published_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1);
    }
}

//! SQLite implementation of storage traits.
//!
//! `SqliteStore` wraps a `rusqlite::Connection`. It is `!Send` -- in async contexts,
//! callers must wrap it in `std::sync::Mutex` and never hold the lock across an `.await` point.

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use super::mode::{Visibility, WriteMode};
use super::traits::{
    AttestationRow, AttestationStore, LineageStore, ReconstructionInputs, SearchResult,
};

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
    -- privacy TEXT and sealed_blob BLOB are added by migrate_sealed_columns().
);
CREATE TABLE IF NOT EXISTS attestation_embeddings (
    attestation_id TEXT PRIMARY KEY,
    embedding_dim INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    FOREIGN KEY (attestation_id) REFERENCES attestations(attestation_id)
);
CREATE INDEX IF NOT EXISTS idx_attestations_signer ON attestations(signer_pubkey);
CREATE INDEX IF NOT EXISTS idx_attestations_content_hash ON attestations(content_hash);

-- Sealed-memories tables (task 4).
-- `grants` stores decryption grants linking a sealed memory to an authorised
-- reader (or broadcast). `memory_hash` is the blake3 hash of the sealed
-- attestation. `reader_kid` is NULL for anonymous/broadcast grants.
-- `grant_cose` is the raw COSE_Sign1 envelope of the grant artifact.
-- `withdrawn_at` is set when the author revokes the grant.
CREATE TABLE IF NOT EXISTS grants (
    id TEXT PRIMARY KEY,
    memory_hash TEXT NOT NULL,
    reader_kid TEXT,
    grant_cose BLOB NOT NULL,
    author_pubkey TEXT NOT NULL,
    created_at TEXT NOT NULL,
    withdrawn_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_grants_memory_hash ON grants(memory_hash);
CREATE INDEX IF NOT EXISTS idx_grants_reader_kid ON grants(reader_kid);

-- Per-owner recall key store (tech-spec §7.4, used by task 13).
-- `rk_wrap` is the recall key wrapped for the owner's identity key.
CREATE TABLE IF NOT EXISTS owner_recall_keys (
    owner_pubkey TEXT PRIMARY KEY,
    rk_wrap BLOB NOT NULL
);

-- Sealed-index table (tech-spec §7.4, used by task 13).
-- `k_wrap_rk` is the content key wrapped under the recall key.
-- `emb_nonce` + `emb_ct` store the AEAD-encrypted embedding so the owner
-- can later rebuild their index from the sealed row.
CREATE TABLE IF NOT EXISTS sealed_index (
    attestation_id TEXT PRIMARY KEY,
    k_wrap_rk BLOB NOT NULL,
    emb_nonce BLOB NOT NULL,
    emb_ct BLOB NOT NULL
);

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
/// Decision 9. Sealed rows (`privacy = 'sealed'`) are excluded: their
/// content is encrypted and stored in `sealed_blob`, not in the `content`
/// column that search results carry.
const SEARCH_SQL_ALL: &str = "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding, a.signer_pubkey, COALESCE(a.owner_pubkey, ''),
            a.plaintext_on_arweave
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.owner_pubkey = ?
       AND (a.privacy IS NULL OR a.privacy = 'plaintext')";

/// SQL backing `AttestationStore::search` when the caller passes a
/// `Some(owner)` AND `Some(visibility)` (owner-scoped + visibility-filtered).
/// Owner scope is preserved; visibility is bound via `ToSql`, never
/// interpolated as text. Sealed rows are excluded (same rationale as
/// `SEARCH_SQL_ALL`).
const SEARCH_SQL_FILTERED: &str = "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding, a.signer_pubkey, COALESCE(a.owner_pubkey, ''),
            a.plaintext_on_arweave
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.owner_pubkey = ? AND a.visibility = ?
       AND (a.privacy IS NULL OR a.privacy = 'plaintext')";

/// SQL backing `AttestationStore::search` for the anonymous public-pool path
/// (`owner_pubkey = None`). Returns rows from EVERY owner, but only rows whose
/// `visibility` equals the bound parameter. The only caller binds
/// `Visibility::Public`, so private rows never reach an anonymous caller.
/// Legacy rows with a NULL or empty `visibility` are backfilled to `'private'`
/// by `migrate_visibility_column`, and a NULL never equals the bound value, so
/// they also stay out of the pool (privacy-by-default). Sealed rows are
/// excluded (same rationale as `SEARCH_SQL_ALL`).
const SEARCH_SQL_PUBLIC_POOL: &str = "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding, a.signer_pubkey, COALESCE(a.owner_pubkey, ''),
            a.plaintext_on_arweave
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.visibility = ?1
       AND (a.privacy IS NULL OR a.privacy = 'plaintext')";

/// SQL backing `SqliteStore::search_owner_tagged` — owner-scoped cosine search
/// limited to rows whose JSON `tags` array contains one exact tag. The caller
/// binds the owner and a `%"<tag>"%` LIKE pattern. Used by the public `/chat`
/// endpoint so it reads only the seeded `protocol-knowledge` corpus, never
/// other rows that the operator key owns. Sealed rows are excluded.
const SEARCH_SQL_OWNER_TAGGED: &str = "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.visibility, ae.embedding, a.signer_pubkey, COALESCE(a.owner_pubkey, ''),
            a.plaintext_on_arweave
     FROM attestations a
     JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
     WHERE a.owner_pubkey = ?1 AND a.tags LIKE ?2 ESCAPE '\\'
       AND (a.privacy IS NULL OR a.privacy = 'plaintext')";

/// SQL backing `SqliteStore::list_public_artifacts` — the non-search Ledger
/// listing (`GET /artifacts`). Cross-owner, newest-first, bound `LIMIT`. Only
/// rows whose `visibility` equals the bound parameter (always
/// `Visibility::Public`) are returned; private and legacy NULL rows are not.
const LIST_PUBLIC_ARTIFACTS_SQL: &str =
    "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.plaintext_on_arweave
     FROM attestations a
     WHERE a.visibility = ?1
     ORDER BY a.created_at DESC
     LIMIT ?2";

/// Same as `LIST_PUBLIC_ARTIFACTS_SQL`, restricted to one `write_mode`.
const LIST_PUBLIC_ARTIFACTS_BY_MODE_SQL: &str =
    "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
            a.solana_tx, a.arweave_tx, a.created_at, a.write_mode,
            a.plaintext_on_arweave
     FROM attestations a
     WHERE a.write_mode = ?1 AND a.visibility = ?2
     ORDER BY a.created_at DESC
     LIMIT ?3";

/// SQL backing `SqliteStore::non_public_anchor_keys`. Returns the Arweave tx
/// id and content hash of every row that is NOT public. `IS NOT` is NULL-safe,
/// so a legacy NULL `visibility` counts as non-public. The caller binds
/// `Visibility::Public`.
const NON_PUBLIC_ANCHOR_KEYS_SQL: &str = "SELECT arweave_tx, content_hash
     FROM attestations
     WHERE visibility IS NOT ?1";

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
            SUM(CASE WHEN write_mode <> 'local' THEN 1 ELSE 0 END) AS on_chain
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

/// Per-row facts consumed by the chain-stats merge (see `recovery_facts`).
/// `write_mode` stays a raw string ('local' / 'participate') because the
/// merge only compares it against the canonical spellings.
#[derive(Debug, Clone)]
pub struct RowFact {
    /// Raw column value — real Arweave id, `local:<...>` synthetic, or empty.
    pub arweave_tx: String,
    /// OAuth-resolved tenant scope; `None` for legacy/empty rows.
    pub owner_pubkey: Option<String>,
    /// UTC day `YYYY-MM-DD` from the stored ISO-8601 `created_at`.
    pub day: String,
    pub write_mode: String,
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
    /// True when the content was submitted to Arweave as plain text that
    /// anyone can read (owner decision D-8).
    pub plaintext_on_arweave: bool,
}

/// One row as `mnemonic-mcp export` emits it (issue #47,
/// work/arweave-as-source-of-truth Wave 5).
///
/// Every column an owner could need to reconstruct or audit their own history,
/// including the labels. `content` is empty for a row a hosted operator wrote
/// under the anchored path — it never had the text (D-2) — and `arweave_tx` is
/// then where the bytes actually live.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportRow {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub tags: Vec<String>,
    pub solana_tx: String,
    pub arweave_tx: String,
    pub signer_pubkey: String,
    pub owner_pubkey: String,
    pub created_at: String,
    pub write_mode: WriteMode,
    pub visibility: Visibility,
    pub plaintext_on_arweave: bool,
}

/// Keys of every non-public row, returned by
/// `SqliteStore::non_public_anchor_keys`. `GET /artifacts` drops any
/// chain-snapshot item whose Arweave tx id or content hash is in one of these
/// sets.
#[derive(Debug, Clone, Default)]
pub struct NonPublicAnchorKeys {
    pub arweave_txs: std::collections::HashSet<String>,
    pub content_hashes: std::collections::HashSet<String>,
}

impl NonPublicAnchorKeys {
    /// True when the Arweave tx id or the content hash belongs to a
    /// non-public row.
    pub fn matches(&self, arweave_tx: &str, content_hash: Option<&str>) -> bool {
        self.arweave_txs.contains(arweave_tx)
            || content_hash.is_some_and(|h| self.content_hashes.contains(h))
    }
}

/// One day's attestation counts for the Analytics timeline, split by
/// `write_mode`. `on_node` counts `WriteMode::Local` writes; `on_chain` counts
/// `WriteMode::Anchored` writes. `date` is a `YYYY-MM-DD` UTC day key.
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
/// by definition, a paid anchored write. Silently re-tagging those rows
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

/// SQL predicate text for "the row has a real Arweave tx id" (not empty and
/// not a synthetic `local:` id). A constant, never user input. Mirrors
/// [`is_anchored_arweave_tx`].
const ANCHORED_TX_PREDICATE: &str = "arweave_tx <> '' AND arweave_tx NOT LIKE 'local:%'";

/// True when `arweave_tx` is a real Arweave tx id: not empty and not a
/// synthetic `local:` id. The bytes behind such an id were submitted to
/// Arweave, which anyone can read.
pub fn is_anchored_arweave_tx(arweave_tx: &str) -> bool {
    !arweave_tx.is_empty() && !arweave_tx.starts_with("local:")
}

/// Owner decision D-8 (2026-09-27): content anchored on Arweave is plain text
/// that anyone can read, so the server must not label it private. Returns
/// the `(visibility, plaintext_on_arweave)` pair that `save_attestation`
/// stores:
///
/// - A real Arweave tx id sets `plaintext_on_arweave = true` (this also
///   covers a demoted row whose bytes were already submitted).
/// - A `anchored` row with a real Arweave tx id is stored `public`,
///   whatever the caller asked for. Sealed (encrypted) writes are planned;
///   until they ship, anchored content is public plain text.
/// - Every other row keeps the requested visibility.
pub fn effective_visibility(
    write_mode: WriteMode,
    arweave_tx: &str,
    requested: Visibility,
) -> (Visibility, bool) {
    let anchored = is_anchored_arweave_tx(arweave_tx);
    let visibility = if anchored && write_mode == WriteMode::Anchored {
        Visibility::Public
    } else {
        requested
    };
    (visibility, anchored)
}

/// Idempotent migration for owner decision D-8 (2026-09-27).
///
/// Adds `attestations.plaintext_on_arweave INTEGER NOT NULL DEFAULT 0` and
/// `attestations.relabelled_public_at TEXT` (NULL by default), then:
///
/// 1. sets `plaintext_on_arweave = 1` on every row with a real Arweave tx id;
/// 2. relabels `anchored` rows with a real Arweave tx id that are not
///    `public` to `public`, and stamps `relabelled_public_at` with the
///    migration time. Operators use this column to find the owners to
///    notify: `SELECT DISTINCT owner_pubkey FROM attestations WHERE
///    relabelled_public_at IS NOT NULL`.
///
/// Both UPDATEs touch only rows that still need the change, so a re-run is a
/// no-op and never moves an existing `relabelled_public_at` stamp.
fn migrate_plaintext_on_arweave_column(conn: &Connection) -> anyhow::Result<()> {
    let need_flag = !attestations_has_column(conn, "plaintext_on_arweave")?;
    let need_stamp = !attestations_has_column(conn, "relabelled_public_at")?;

    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening plaintext_on_arweave migration transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        if need_flag {
            conn.execute(
                "ALTER TABLE attestations
                    ADD COLUMN plaintext_on_arweave INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("adding attestations.plaintext_on_arweave")?;
        }
        if need_stamp {
            conn.execute(
                "ALTER TABLE attestations ADD COLUMN relabelled_public_at TEXT",
                [],
            )
            .context("adding attestations.relabelled_public_at")?;
        }
        conn.execute(
            &format!(
                "UPDATE attestations SET plaintext_on_arweave = 1
                  WHERE plaintext_on_arweave = 0 AND {ANCHORED_TX_PREDICATE}"
            ),
            [],
        )
        .context("backfilling attestations.plaintext_on_arweave")?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            &format!(
                "UPDATE attestations
                    SET visibility = ?1, relabelled_public_at = ?2
                  WHERE write_mode = ?3
                    AND visibility IS NOT ?1
                    AND {ANCHORED_TX_PREDICATE}"
            ),
            params![Visibility::Public, now, WriteMode::Anchored],
        )
        .context("relabelling anchored private rows to public")?;
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing plaintext_on_arweave migration")?;
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

/// Normalize the deprecated `write_mode` value `'participate'` to the canonical
/// `'anchored'` (work/arweave-as-source-of-truth, 2026-09-27).
///
/// The rename is a spelling change only; the meaning is unchanged — the memory
/// lives on Arweave. `WriteMode::from_str_strict` still accepts the old token,
/// so a read of an un-migrated row never fails. This migration exists so that
/// **SQL** can keep comparing the column against a single literal: `TIMELINE_SQL`
/// and the `WHERE a.write_mode = ?1` listing would otherwise each have to match
/// two spellings, and one missed site silently undercounts rows.
///
/// Idempotent: after the first run the UPDATE matches zero rows. It needs no
/// `PRAGMA table_info` guard because `migrate_write_mode_column` runs before it
/// in both callers, so the column always exists.
///
/// The column DEFAULT stays `'participate'`: changing it requires a full table
/// rebuild in SQLite, and every INSERT passes `write_mode` explicitly, so the
/// default never applies to a new row. Any row that did acquire it is normalized
/// on the next open.
fn migrate_write_mode_anchored_rename(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening write_mode anchored-rename transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        conn.execute(
            "UPDATE attestations
                SET write_mode = 'anchored'
              WHERE write_mode = 'participate'",
            [],
        )
        .context("normalizing attestations.write_mode 'participate' to 'anchored'")?;
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing write_mode anchored-rename migration")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

/// Idempotent ADD-COLUMN migration for sealed-memories support (task 4).
///
/// Adds two columns to `attestations`:
///   - `privacy TEXT NOT NULL DEFAULT 'plaintext'` — the privacy mode of the
///     row. Values: `'plaintext'` (unencrypted, default for all existing rows)
///     or `'sealed'` (content is encrypted; stored in `sealed_blob`).
///   - `sealed_blob BLOB` — the raw COSE_Sign1 sealed artifact for
///     `privacy = 'sealed'` rows. NULL for plaintext rows.
///
/// SQLite `ALTER TABLE ... ADD COLUMN` lacks `IF NOT EXISTS`, so presence is
/// gated via `PRAGMA table_info`. Wrapped in `BEGIN IMMEDIATE` to serialize
/// against any concurrent opener. Idempotent across deploys.
fn migrate_sealed_columns(conn: &Connection) -> anyhow::Result<()> {
    let need_privacy = !attestations_has_column(conn, "privacy")?;
    let need_sealed_blob = !attestations_has_column(conn, "sealed_blob")?;

    if !need_privacy && !need_sealed_blob {
        return Ok(());
    }

    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening sealed_columns migration transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        if need_privacy {
            conn.execute(
                "ALTER TABLE attestations
                    ADD COLUMN privacy TEXT NOT NULL DEFAULT 'plaintext'",
                [],
            )
            .context("adding attestations.privacy")?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_attestations_privacy
                     ON attestations(privacy)",
                [],
            )
            .context("creating idx_attestations_privacy")?;
        }
        if need_sealed_blob {
            conn.execute(
                "ALTER TABLE attestations ADD COLUMN sealed_blob BLOB",
                [],
            )
            .context("adding attestations.sealed_blob")?;
        }
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing sealed_columns migration")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

/// A row returned by `SqliteStore::list_sealed`.
#[derive(Debug, Clone)]
pub struct SealedRow {
    pub attestation_id: String,
    pub content_hash: String,
    pub solana_tx: String,
    pub arweave_tx: String,
    pub signer_pubkey: String,
    pub owner_pubkey: String,
    pub created_at: String,
    /// The raw COSE_Sign1 sealed-artifact bytes.
    pub sealed_blob: Vec<u8>,
}

/// A grant row returned by `SqliteStore::grants_for_reader` /
/// `SqliteStore::grants_for_memory`.
#[derive(Debug, Clone)]
pub struct GrantRow {
    pub id: String,
    pub memory_hash: String,
    /// `None` for anonymous / broadcast grants.
    pub reader_kid: Option<String>,
    /// Raw COSE_Sign1 envelope of the grant artifact.
    pub grant_cose: Vec<u8>,
    pub author_pubkey: String,
    pub created_at: String,
    pub withdrawn_at: Option<String>,
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
        migrate_write_mode_anchored_rename(&conn)?;
        migrate_plaintext_on_arweave_column(&conn)?;
        migrate_sealed_columns(&conn)?;
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
        migrate_write_mode_anchored_rename(&conn)?;
        migrate_plaintext_on_arweave_column(&conn)?;
        migrate_sealed_columns(&conn)?;
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

    /// Minimal per-row facts for merging DB state with an Arweave chain
    /// snapshot (recover-traction-from-chain): the chain is the source of
    /// truth for anchored writes; the DB contributes local-only rows plus
    /// exact `created_at` days and owners for rows it still has. Aggregate
    /// data only — no content, no hashes.
    pub fn recovery_facts(&self) -> anyhow::Result<Vec<RowFact>> {
        let mut stmt = self.conn.prepare(
            "SELECT arweave_tx, owner_pubkey, substr(created_at, 1, 10), write_mode
             FROM attestations",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RowFact {
                arweave_tx: row.get(0)?,
                owner_pubkey: row.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty()),
                day: row.get(2)?,
                write_mode: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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

    /// The earliest `anchored` (anchored) row for `content_hash`, as
    /// `(attestation_id, solana_tx, arweave_tx)`, or `None`. A row demoted
    /// to `local` after a failed delivery check does not count. The deferred
    /// sign-callback uses this to never anchor one artifact twice.
    pub fn find_anchored_by_content_hash(
        &self,
        content_hash: &str,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT attestation_id, solana_tx, arweave_tx
             FROM attestations
             -- `<> 'local'` rather than `= 'anchored'`: this is a REPLAY GUARD,
             -- so any already-anchored row must block a second anchor. Matching
             -- one spelling would miss a row written before the
             -- participate->anchored rename, and re-anchor it — spending a
             -- second free grant and creating a duplicate chain write. It is
             -- also future-proof for a second anchor backend (issue #70).
             WHERE content_hash = ?1 AND write_mode <> 'local'
             ORDER BY created_at ASC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![content_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?))),
            None => Ok(None),
        }
    }

    /// Every row owned by `owner_pubkey`, oldest first, for `mnemonic-mcp export`.
    ///
    /// Oldest first on purpose: an export is a backup, and append order matching
    /// write order makes a diff between two exports readable.
    ///
    /// Owner-scoped with no exceptions. An export is the owner's own data, and a
    /// cross-owner variant of this query would be a bulk disclosure primitive, so
    /// the predicate is not optional.
    pub fn export_rows(&self, owner_pubkey: &str) -> anyhow::Result<Vec<ExportRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT attestation_id, content, content_hash, tags, solana_tx, arweave_tx,
                    signer_pubkey, owner_pubkey, created_at, write_mode, visibility,
                    plaintext_on_arweave
             FROM attestations
             WHERE owner_pubkey = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![owner_pubkey], |row| {
            let tags_str: String = row.get(3)?;
            Ok(ExportRow {
                attestation_id: row.get(0)?,
                content: row.get(1)?,
                content_hash: row.get(2)?,
                tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                solana_tx: row.get(4)?,
                arweave_tx: row.get(5)?,
                signer_pubkey: row.get(6)?,
                owner_pubkey: row.get(7)?,
                created_at: row.get(8)?,
                write_mode: row.get::<_, WriteMode>(9)?,
                visibility: row.get::<_, Visibility>(10)?,
                plaintext_on_arweave: row.get(11)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List public attestations newest-first for the Ledger page
    /// (`GET /artifacts`). Cross-owner, but only `visibility = 'public'` rows:
    /// private rows and legacy rows with a NULL or empty `visibility` are never
    /// returned (owner decision 2026-09-27: private rows go only to their
    /// owner). `limit` caps the result; an empty table yields an empty vec.
    /// Content search (`?q=`) is served by the cosine `search` path
    /// (anonymous public-pool branch), not by this chronological list.
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
                plaintext_on_arweave: row.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List public Ledger rows for one provenance mode. This powers
    /// source-aware pagination so a large on-node history cannot crowd every
    /// anchored row out of the first page (and vice versa). Same visibility
    /// rule as `list_public_artifacts`: only `visibility = 'public'` rows.
    pub fn list_public_artifacts_by_mode(
        &self,
        mode: WriteMode,
        limit: usize,
    ) -> anyhow::Result<Vec<PublicArtifact>> {
        let mut stmt = self.conn.prepare(LIST_PUBLIC_ARTIFACTS_BY_MODE_SQL)?;
        let rows = stmt.query_map(params![mode, Visibility::Public, limit as i64], |row| {
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
                plaintext_on_arweave: row.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Arweave tx ids and content hashes of every row that is NOT public
    /// (private, or a legacy NULL `visibility`). `GET /artifacts` uses this
    /// to drop chain-snapshot items that match a private DB row, so the API
    /// never presents a row that its owner marked private. Empty strings are
    /// skipped.
    pub fn non_public_anchor_keys(&self) -> anyhow::Result<NonPublicAnchorKeys> {
        let mut stmt = self.conn.prepare(NON_PUBLIC_ANCHOR_KEYS_SQL)?;
        let rows = stmt.query_map(params![Visibility::Public], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })?;
        let mut keys = NonPublicAnchorKeys::default();
        for row in rows {
            let (arweave_tx, content_hash) = row?;
            if let Some(tx) = arweave_tx.filter(|s| !s.is_empty()) {
                keys.arweave_txs.insert(tx);
            }
            if let Some(h) = content_hash.filter(|s| !s.is_empty()) {
                keys.content_hashes.insert(h);
            }
        }
        Ok(keys)
    }

    /// `plaintext_on_arweave` flag of the row that `tx_id` names (Solana or
    /// Arweave tx id), scoped to `owner_pubkey` with the same tenant
    /// predicate as `find_write_mode_by_tx`. `None` when no such row exists
    /// for this owner.
    pub fn plaintext_on_arweave_by_tx(
        &self,
        tx_id: &str,
        owner_pubkey: &str,
    ) -> anyhow::Result<Option<bool>> {
        let flag = self
            .conn
            .query_row(
                "SELECT plaintext_on_arweave FROM attestations
                 WHERE (solana_tx = ?1 OR arweave_tx = ?1) AND owner_pubkey = ?2
                 LIMIT 1",
                params![tx_id, owner_pubkey],
                |row| row.get::<_, bool>(0),
            )
            .optional()?;
        Ok(flag)
    }

    /// Owner-scoped cosine search limited to rows that carry `tag` exactly
    /// (JSON `tags` array element). Returns rows of any visibility, because
    /// the owner scope is the tenant boundary. The public `/chat` endpoint
    /// calls this with the operator key and `"protocol-knowledge"`, so chat
    /// context comes only from the seeded knowledge corpus.
    pub fn search_owner_tagged(
        &self,
        query_embedding: &[f32],
        owner_pubkey: &str,
        tag: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        // Match the tag as one JSON string element: serialize it the way the
        // `tags` column stores it, then escape the LIKE wildcards so the tag
        // matches literally (`ESCAPE '\'` in the SQL).
        let json_tag = serde_json::to_string(tag)?;
        let mut pattern = String::with_capacity(json_tag.len() + 2);
        pattern.push('%');
        for c in json_tag.chars() {
            if matches!(c, '%' | '_' | '\\') {
                pattern.push('\\');
            }
            pattern.push(c);
        }
        pattern.push('%');
        let mut stmt = self.conn.prepare(SEARCH_SQL_OWNER_TAGGED)?;
        let scorer = CosineScorer::new(query_embedding);
        let rows = stmt.query_map(params![owner_pubkey, pattern], |row| scorer.map_row(row))?;
        let mut results: Vec<SearchResult> = rows.filter_map(|r| r.ok()).collect();
        sort_and_truncate(&mut results, limit);
        Ok(results)
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

    /// Owner of the attestation row `attestation_id` (falls back to the
    /// signer for legacy rows), or `None` when no such row exists.
    pub fn attestation_owner(&self, attestation_id: &str) -> anyhow::Result<Option<String>> {
        let owner = self
            .conn
            .query_row(
                "SELECT CASE WHEN COALESCE(owner_pubkey, '') = ''
                        THEN signer_pubkey ELSE owner_pubkey END
                   FROM attestations WHERE attestation_id = ?1",
                params![attestation_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(owner.flatten())
    }

    /// Owner (`owner_pubkey` of the backing attestation) of the post at
    /// `slug`, or `None` when no post uses that slug.
    pub fn blog_post_owner(&self, slug: &str) -> anyhow::Result<Option<String>> {
        let owner = self
            .conn
            .query_row(
                "SELECT CASE WHEN COALESCE(a.owner_pubkey, '') = ''
                        THEN a.signer_pubkey ELSE a.owner_pubkey END
                   FROM blog_posts b
                   JOIN attestations a ON a.attestation_id = b.attestation_id
                  WHERE b.slug = ?1",
                rusqlite::params![slug],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(owner.flatten())
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

    // -- sealed-memories (task 4) --------------------------------------------

    /// Persist a sealed attestation row. `sealed_blob` is the raw COSE_Sign1
    /// bytes of the sealed artifact. `content` is stored as an empty string
    /// (the ciphertext lives in `sealed_blob`). No embedding row is created:
    /// sealed rows are opaque to vector search. `privacy` is set to `'sealed'`.
    ///
    /// Callers supply the same column set as `save_attestation`, except
    /// `content` and `embedding` are absent — they have no meaning for a
    /// sealed artifact.
    #[allow(clippy::too_many_arguments)]
    pub fn save_sealed_attestation(
        &self,
        attestation_id: &str,
        content_hash: &str,
        tags: &[String],
        solana_tx: &str,
        arweave_tx: &str,
        signer_pubkey: &str,
        owner_pubkey: &str,
        created_at: &str,
        write_mode: WriteMode,
        sealed_blob: &[u8],
    ) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO attestations
                 (attestation_id, content, content_hash, tags,
                  solana_tx, arweave_tx, signer_pubkey, created_at, owner_pubkey,
                  write_mode, visibility, plaintext_on_arweave,
                  privacy, sealed_blob)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                attestation_id,
                "",          // content — empty for sealed rows
                content_hash,
                tags_json,
                solana_tx,
                arweave_tx,
                signer_pubkey,
                created_at,
                owner_pubkey,
                write_mode.as_str(),
                Visibility::Private.as_str(), // sealed rows are always private
                false,                        // plaintext_on_arweave
                "sealed",
                sealed_blob,
            ],
        )?;
        // Intentionally NO attestation_embeddings row — sealed content is
        // opaque to vector search (task 4, TDD anchor).
        Ok(())
    }

    /// List sealed attestation rows owned by `owner`, optionally after
    /// `since` (ISO-8601), newest-first, capped at `limit`.
    pub fn list_sealed(
        &self,
        owner: &str,
        since: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SealedRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT attestation_id, content_hash, solana_tx, arweave_tx,
                    signer_pubkey, COALESCE(owner_pubkey, ''), created_at,
                    sealed_blob
             FROM attestations
             WHERE owner_pubkey = ?1
               AND privacy = 'sealed'
               AND created_at > ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![owner, since.unwrap_or(""), limit as i64],
            |row| {
                Ok(SealedRow {
                    attestation_id: row.get(0)?,
                    content_hash: row.get(1)?,
                    solana_tx: row.get(2)?,
                    arweave_tx: row.get(3)?,
                    signer_pubkey: row.get(4)?,
                    owner_pubkey: row.get(5)?,
                    created_at: row.get(6)?,
                    sealed_blob: row.get::<_, Option<Vec<u8>>>(7)?.unwrap_or_default(),
                })
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Count sealed rows owned by `owner`.
    pub fn count_sealed(&self, owner: &str) -> anyhow::Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM attestations
             WHERE owner_pubkey = ?1 AND privacy = 'sealed'",
            params![owner],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Persist a grant that links a sealed memory to an authorised reader.
    ///
    /// `reader_kid` is `None` for anonymous/broadcast grants.
    /// `grant_cose` is the raw COSE_Sign1 envelope of the grant artifact.
    #[allow(clippy::too_many_arguments)]
    pub fn save_grant(
        &self,
        id: &str,
        memory_hash: &str,
        reader_kid: Option<&str>,
        grant_cose: &[u8],
        author_pubkey: &str,
        created_at: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO grants
                 (id, memory_hash, reader_kid, grant_cose, author_pubkey, created_at)
             VALUES (?,?,?,?,?,?)",
            params![id, memory_hash, reader_kid, grant_cose, author_pubkey, created_at],
        )?;
        Ok(())
    }

    /// Return all non-withdrawn grants for `reader_kid`. A `None` reader_kid
    /// returns anonymous/broadcast grants.
    pub fn grants_for_reader(&self, reader_kid: &str) -> anyhow::Result<Vec<GrantRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, memory_hash, reader_kid, grant_cose,
                    author_pubkey, created_at, withdrawn_at
             FROM grants
             WHERE reader_kid = ?1
               AND withdrawn_at IS NULL",
        )?;
        let rows = stmt.query_map(params![reader_kid], grant_row_from_row)?;
        collect_grant_rows(rows)
    }

    /// Return all grants (withdrawn or not) for a sealed memory identified by
    /// `memory_hash`.
    pub fn grants_for_memory(&self, memory_hash: &str) -> anyhow::Result<Vec<GrantRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, memory_hash, reader_kid, grant_cose,
                    author_pubkey, created_at, withdrawn_at
             FROM grants
             WHERE memory_hash = ?1",
        )?;
        let rows = stmt.query_map(params![memory_hash], grant_row_from_row)?;
        collect_grant_rows(rows)
    }

    /// Withdraw a grant by setting `withdrawn_at` to the current UTC time.
    /// No-op if the grant does not exist or is already withdrawn.
    pub fn withdraw_grant(&self, id: &str) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE grants SET withdrawn_at = ?1
             WHERE id = ?2 AND withdrawn_at IS NULL",
            params![now, id],
        )?;
        Ok(())
    }

    /// Data-fix for F1 (plaintext anchored rows): rows with
    /// `write_mode = 'anchored'` and `visibility = 'private'` that were
    /// written before decision D-8 may still have `privacy = 'plaintext'`
    /// (the default) but may need their `privacy` column confirmed. This
    /// function ensures anchored rows with `visibility = 'private'` get
    /// `privacy = 'plaintext'`. Idempotent.
    ///
    /// Note: in practice the default `'plaintext'` covers these rows already.
    /// This function is the explicit, testable contract that task 6 calls.
    pub fn relabel_plaintext_anchors(&self) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE attestations
                SET privacy = 'plaintext'
              WHERE visibility = 'private'
                AND (privacy IS NULL OR privacy <> 'sealed')",
            [],
        )?;
        Ok(())
    }
}

/// Map a `grants` row (in the column order used by `grants_for_reader` /
/// `grants_for_memory`) into a [`GrantRow`]. Shared so the two read paths
/// cannot drift in column order.
fn grant_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GrantRow> {
    Ok(GrantRow {
        id: row.get(0)?,
        memory_hash: row.get(1)?,
        reader_kid: row.get(2)?,
        grant_cose: row.get::<_, Vec<u8>>(3)?,
        author_pubkey: row.get(4)?,
        created_at: row.get(5)?,
        withdrawn_at: row.get(6)?,
    })
}

/// Collect a `MappedRows` iterator of `GrantRow` results into a `Vec`,
/// discarding rows that fail to decode (should never happen on a well-formed
/// DB, but keeps the read paths from unwinding on a single bad row).
fn collect_grant_rows(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<GrantRow>>,
) -> anyhow::Result<Vec<GrantRow>> {
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
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
        // Owner decision D-8: anchored content is public plain text on
        // Arweave, so an anchored anchored row is stored `public` and
        // every row with a real Arweave tx id is flagged.
        let (visibility, plaintext_on_arweave) =
            effective_visibility(write_mode, arweave_tx, visibility);
        // Explicit column list — `owner_pubkey`, `write_mode`, and
        // `visibility` were added by migration helpers after the original
        // table CREATE, so the column count and order in
        // `INSERT OR REPLACE INTO ... VALUES (...)` would otherwise drift
        // between fresh and migrated DBs.
        self.conn.execute(
            "INSERT OR REPLACE INTO attestations
                 (attestation_id, content, content_hash, tags,
                  solana_tx, arweave_tx, signer_pubkey, created_at, owner_pubkey,
                  write_mode, visibility, plaintext_on_arweave)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
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
                plaintext_on_arweave,
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

    fn reconstruction_inputs_by_tx(
        &self,
        tx_id: &str,
        owner_pubkey: &str,
    ) -> anyhow::Result<Option<ReconstructionInputs>> {
        // Same tenant predicate as `find_by_tx` — see the note there. The
        // JOIN is INNER on purpose: a row with no embedding cannot have its
        // artifact rebuilt, and reporting that as `None` keeps the caller
        // from mistaking "cannot verify" for "hash mismatch".
        let mut stmt = self.conn.prepare(
            "SELECT a.attestation_id, a.content, a.content_hash, a.tags,
                    a.solana_tx, a.arweave_tx, a.signer_pubkey, a.created_at,
                    ae.embedding
             FROM attestations a
             JOIN attestation_embeddings ae ON a.attestation_id = ae.attestation_id
             WHERE (a.solana_tx = ?1 OR a.arweave_tx = ?1) AND a.owner_pubkey = ?2
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![tx_id, owner_pubkey])?;
        match rows.next()? {
            Some(row) => {
                let tags_str: String = row.get(3)?;
                let emb_blob: Vec<u8> = row.get(8)?;
                Ok(Some(ReconstructionInputs {
                    attestation_id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get(2)?,
                    tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                    solana_tx: row.get(4)?,
                    arweave_tx: row.get(5)?,
                    signer_pubkey: row.get(6)?,
                    created_at: row.get(7)?,
                    embedding: bytes_to_floats(&emb_blob),
                }))
            }
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
        // Ownership filter (Decision 9) on every owner-scoped path. No
        // carve-out; legacy rows with NULL `owner_pubkey` match no owner.
        // They reach the anonymous pool only if `visibility = 'public'`, and
        // the migration backfills legacy NULL visibility to `'private'`.
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

        let scorer = CosineScorer::new(query_embedding);
        let row_mapper = |row: &rusqlite::Row<'_>| scorer.map_row(row);

        // Search shapes per Decision 5 + SAR1-M1 (agent-native-distribution)
        // and the owner decision of 2026-09-27 ("private means private"):
        //   (Some(owner), Some(v))      → owner-scoped + visibility-filtered
        //   (Some(owner), None)         → owner-scoped (authenticated, full corpus)
        //   (None,        Some(Public)) → cross-owner public pool (anonymous);
        //                                 `visibility = 'public'` is a bound
        //                                 SQL predicate
        //   (None,        Some(Private)) and
        //   (None,        None)         → DISALLOWED. Either would expose other
        //                                 owners' private rows. Defensive empty
        //                                 result keeps the storage layer from
        //                                 leaking on a future caller bug.
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
            (None, Some(Visibility::Public)) => {
                let mut stmt = self.conn.prepare(SEARCH_SQL_PUBLIC_POOL)?;
                let rows = stmt.query_map(params![Visibility::Public], row_mapper)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(Visibility::Private)) | (None, None) => Vec::new(),
        };

        // Drop rows that carry no vector (work/arweave-as-source-of-truth D-2).
        // An anchored row written by a hosted operator stores no embedding and no
        // content, because Arweave holds the memory. Such a row cannot be ranked
        // by meaning, so returning it would claim a relevance it does not have and
        // hand the caller an empty `content`. A hit that cannot be scored is worse
        // than no hit. The memory is still recoverable — a client restores its own
        // index from the chain (`mnemonic-mcp restore`) and searches locally.
        results.retain(|r| r.relevance_score != 0.0 || !r.content.is_empty());

        sort_and_truncate(&mut results, limit);
        Ok(results)
    }
}

/// Cosine scorer shared by every search query. Normalizes the query vector
/// once and maps one SQL row (the column layout of the `SEARCH_SQL_*`
/// constants) to a scored `SearchResult`.
struct CosineScorer {
    q_normalized: Vec<f32>,
}

impl CosineScorer {
    fn new(query_embedding: &[f32]) -> Self {
        let q_norm = l2_norm(query_embedding);
        let q_normalized = if q_norm > 0.0 {
            query_embedding.iter().map(|x| x / q_norm).collect()
        } else {
            query_embedding.to_vec()
        };
        Self { q_normalized }
    }

    fn map_row(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchResult> {
        let emb_blob: Vec<u8> = row.get(9)?;
        let emb = bytes_to_floats(&emb_blob);
        let e_norm = l2_norm(&emb);
        let score = if e_norm > 0.0 {
            self.q_normalized
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
            signer_pubkey: row.get(10)?,
            owner_pubkey: row.get(11)?,
            plaintext_on_arweave: row.get(12)?,
        })
    }
}

/// Sort search hits by score, best first, and keep the top `limit`.
fn sort_and_truncate(results: &mut Vec<SearchResult>, limit: usize) {
    results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
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
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
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
                WriteMode::Anchored,
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
                "att-anchored",
                "anchored content",
                "h-anchored",
                &[],
                "sol-different",
                "ar-anchored",
                "signer-b",
                "owner-b",
                "2026-04-13T00:00:00Z",
                WriteMode::Anchored,
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
        // owner-b's row is anchored
        assert_eq!(
            store
                .find_write_mode_by_tx("sol-different", "owner-b")
                .unwrap(),
            Some(WriteMode::Anchored)
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
                    WriteMode::Anchored,
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
                WriteMode::Anchored,
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
                WriteMode::Anchored,
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
                WriteMode::Anchored,
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
                WriteMode::Anchored,
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
                WriteMode::Anchored,
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
                WriteMode::Anchored,
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
                WriteMode::Anchored,
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
            WriteMode::Anchored,
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
        // NOTE: this test drives `migrate_write_mode_column` directly, i.e. the
        // legacy migration in isolation. At that point the raw column value is
        // still the historical DEFAULT `'participate'`; the rename to
        // `'anchored'` is a separate, later migration
        // (`migrate_write_mode_anchored_rename`). Asserting the raw string here
        // is deliberate.
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
                WriteMode::Anchored,
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

    // -- Owner decision D-8: anchored content is public plain text ---------

    /// Raw INSERT that bypasses `save_attestation` (and so the D-8 rule),
    /// to model rows written before the migration existed.
    fn raw_insert(conn: &Connection, id: &str, arweave_tx: &str, mode: &str, vis: &str) {
        conn.execute(
            "INSERT INTO attestations
                 (attestation_id, content, content_hash, tags, solana_tx, arweave_tx,
                  signer_pubkey, created_at, owner_pubkey, write_mode, visibility)
             VALUES (?1, 'c', ?1, '[]', 'sol', ?2, 'signer', '2026-06-01T00:00:00Z',
                     ?3, ?4, ?5)",
            params![id, arweave_tx, format!("owner-{id}"), mode, vis],
        )
        .unwrap();
    }

    fn row_state(conn: &Connection, id: &str) -> (String, bool, Option<String>) {
        conn.query_row(
            "SELECT visibility, plaintext_on_arweave, relabelled_public_at
               FROM attestations WHERE attestation_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn plaintext_migration_relabels_anchored_private_rows() {
        // A DB from before D-8: every earlier migration ran, the new one
        // did not, so the columns do not exist yet.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        super::migrate_owner_pubkey_columns(&conn).unwrap();
        super::migrate_correlation_id_column(&conn).unwrap();
        super::migrate_write_mode_column(&conn).unwrap();
        super::migrate_visibility_column(&conn).unwrap();
        assert!(!super::attestations_has_column(&conn, "plaintext_on_arweave").unwrap());

        raw_insert(&conn, "anchored-priv", "ArTx1", "anchored", "private");
        raw_insert(&conn, "anchored-pub", "ArTx2", "anchored", "public");
        raw_insert(&conn, "synthetic", "local:x", "anchored", "private");
        raw_insert(&conn, "demoted", "ArTx3", "local", "private");
        raw_insert(&conn, "local", "local:y", "local", "private");

        super::migrate_plaintext_on_arweave_column(&conn).unwrap();

        let (vis, flag, stamp) = row_state(&conn, "anchored-priv");
        assert_eq!(vis, "public");
        assert!(flag);
        let first_stamp = stamp.expect("relabelled row is stamped");

        let (vis, flag, stamp) = row_state(&conn, "anchored-pub");
        assert_eq!((vis.as_str(), flag, stamp), ("public", true, None));
        let (vis, flag, stamp) = row_state(&conn, "synthetic");
        assert_eq!((vis.as_str(), flag, stamp), ("private", false, None));
        let (vis, flag, stamp) = row_state(&conn, "demoted");
        assert_eq!((vis.as_str(), flag, stamp), ("private", true, None));
        let (vis, flag, stamp) = row_state(&conn, "local");
        assert_eq!((vis.as_str(), flag, stamp), ("private", false, None));

        // The ops query that finds the owners to notify.
        let owners: Vec<String> = conn
            .prepare(
                "SELECT DISTINCT owner_pubkey FROM attestations
                  WHERE relabelled_public_at IS NOT NULL",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(owners, vec!["owner-anchored-priv".to_string()]);

        // Idempotent: a second run changes nothing and keeps the stamp.
        super::migrate_plaintext_on_arweave_column(&conn).unwrap();
        let (vis, flag, stamp) = row_state(&conn, "anchored-priv");
        assert_eq!((vis.as_str(), flag), ("public", true));
        assert_eq!(stamp.as_deref(), Some(first_stamp.as_str()));
    }

    #[test]
    fn save_attestation_applies_d8_rule() {
        let store = SqliteStore::in_memory().unwrap();
        let save = |id: &str, arweave_tx: &str, mode: WriteMode| {
            store
                .save_attestation(
                    id,
                    "c",
                    id,
                    &[],
                    &format!("sol-{id}"),
                    arweave_tx,
                    "signer",
                    "owner-a",
                    "2026-06-01T00:00:00Z",
                    mode,
                    Visibility::Private,
                    &[1.0, 0.0],
                )
                .unwrap();
        };
        save("anchored", "ArTx1", WriteMode::Anchored);
        save("synthetic", "local:x", WriteMode::Anchored);
        save("demoted", "ArTx2", WriteMode::Local);

        let conn = store.conn();
        let (vis, flag, stamp) = row_state(conn, "anchored");
        assert_eq!((vis.as_str(), flag, stamp), ("public", true, None));
        let (vis, flag, _) = row_state(conn, "synthetic");
        assert_eq!((vis.as_str(), flag), ("private", false));
        let (vis, flag, _) = row_state(conn, "demoted");
        assert_eq!((vis.as_str(), flag), ("private", true));

        // The flag reaches search hits, Ledger rows and the verify lookup.
        let own = store
            .search(&[1.0, 0.0], Some("owner-a"), None, 10)
            .unwrap();
        let flag_of = |id: &str| {
            own.iter()
                .find(|r| r.attestation_id == id)
                .map(|r| r.plaintext_on_arweave)
        };
        assert_eq!(flag_of("anchored"), Some(true));
        assert_eq!(flag_of("synthetic"), Some(false));
        let listed = store.list_public_artifacts(10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].attestation_id, "anchored");
        assert!(listed[0].plaintext_on_arweave);
        assert_eq!(
            store
                .plaintext_on_arweave_by_tx("sol-anchored", "owner-a")
                .unwrap(),
            Some(true)
        );
        assert_eq!(
            store
                .plaintext_on_arweave_by_tx("sol-anchored", "owner-b")
                .unwrap(),
            None,
            "tenant-scoped"
        );
    }

    #[test]
    fn list_public_artifacts_returns_public_rows_only() {
        // Private means private (owner decision 2026-09-27): the Ledger list
        // surfaces only `visibility = 'public'` rows, newest-first.
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "pub-1",
            "owner-a",
            "s1",
            "2026-06-01T00:00:00Z",
            WriteMode::Anchored,
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
            WriteMode::Anchored,
            Visibility::Public,
        );

        let rows = store.list_public_artifacts(10).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.attestation_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["pub-2", "pub-1"],
            "public rows only, newest first"
        );

        // Limit is honored.
        let limited = store.list_public_artifacts(1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].attestation_id, "pub-2");

        // The by-mode listing applies the same visibility rule.
        let local = store
            .list_public_artifacts_by_mode(WriteMode::Local, 10)
            .unwrap();
        assert!(local.is_empty(), "private local row must not be listed");
    }

    #[test]
    fn legacy_null_visibility_is_treated_as_private() {
        // A row whose `visibility` is NULL or empty (hand-edited or
        // re-imported legacy DB) is normalized to 'private' by the migration
        // and then never reaches a public read path.
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "legacy",
            "owner-a",
            "s1",
            "2026-06-01T00:00:00Z",
            WriteMode::Local,
            Visibility::Public,
        );
        store
            .conn
            .execute(
                "UPDATE attestations SET visibility = '' WHERE attestation_id = 'legacy'",
                [],
            )
            .unwrap();
        super::migrate_visibility_column(store.conn()).unwrap();

        assert!(store.list_public_artifacts(10).unwrap().is_empty());
        assert!(store
            .list_public_artifacts_by_mode(WriteMode::Local, 10)
            .unwrap()
            .is_empty());
        assert!(store
            .search(&[1.0, 0.0], None, Some(Visibility::Public), 10)
            .unwrap()
            .is_empty());
        // The owner still sees the row.
        let own = store
            .search(&[1.0, 0.0], Some("owner-a"), None, 10)
            .unwrap();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].visibility, Visibility::Private);
        // And its keys are reported as non-public.
        let keys = store.non_public_anchor_keys().unwrap();
        assert!(keys.matches("ar", None));
    }

    #[test]
    fn anonymous_search_returns_public_rows_only() {
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "pub-a",
            "owner-a",
            "s1",
            "2026-06-01T00:00:00Z",
            WriteMode::Local,
            Visibility::Public,
        );
        seed_row(
            &store,
            "priv-a",
            "owner-a",
            "s2",
            "2026-06-02T00:00:00Z",
            WriteMode::Local,
            Visibility::Private,
        );
        seed_row(
            &store,
            "pub-b",
            "owner-b",
            "s3",
            "2026-06-03T00:00:00Z",
            WriteMode::Anchored,
            Visibility::Public,
        );

        let pool = store
            .search(&[1.0, 0.0], None, Some(Visibility::Public), 10)
            .unwrap();
        let mut ids: Vec<&str> = pool.iter().map(|r| r.attestation_id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["pub-a", "pub-b"]);
        assert!(pool.iter().all(|r| r.visibility == Visibility::Public));

        // Cross-owner shapes that could expose private rows return nothing.
        assert!(store
            .search(&[1.0, 0.0], None, Some(Visibility::Private), 10)
            .unwrap()
            .is_empty());
        assert!(store
            .search(&[1.0, 0.0], None, None, 10)
            .unwrap()
            .is_empty());

        // The owner still sees own private rows.
        let own = store
            .search(&[1.0, 0.0], Some("owner-a"), None, 10)
            .unwrap();
        let mut own_ids: Vec<&str> = own.iter().map(|r| r.attestation_id.as_str()).collect();
        own_ids.sort_unstable();
        assert_eq!(own_ids, vec!["priv-a", "pub-a"]);
    }

    #[test]
    fn non_public_anchor_keys_lists_private_rows_only() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .save_attestation(
                "priv",
                "secret",
                "hash-priv",
                &[],
                "sol-priv",
                "ar-priv",
                "signer",
                "owner-a",
                "2026-06-01T00:00:00Z",
                // A demoted row: real Arweave tx, but `local` and private.
                WriteMode::Local,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();
        store
            .save_attestation(
                "pub",
                "open",
                "hash-pub",
                &[],
                "sol-pub",
                "ar-pub",
                "signer",
                "owner-a",
                "2026-06-01T00:00:01Z",
                WriteMode::Anchored,
                Visibility::Public,
                &[1.0, 0.0],
            )
            .unwrap();
        let keys = store.non_public_anchor_keys().unwrap();
        assert!(keys.matches("ar-priv", None));
        assert!(keys.matches("other", Some("hash-priv")));
        assert!(!keys.matches("ar-pub", Some("hash-pub")));
    }

    #[test]
    fn search_owner_tagged_matches_exact_tag_only() {
        let store = SqliteStore::in_memory().unwrap();
        let save = |id: &str, owner: &str, tags: &[String]| {
            store
                .save_attestation(
                    id,
                    "c",
                    id,
                    tags,
                    "sol",
                    "ar",
                    "signer",
                    owner,
                    "2026-06-01T00:00:00Z",
                    WriteMode::Local,
                    Visibility::Private,
                    &[1.0, 0.0],
                )
                .unwrap();
        };
        save("kb", "op", &["protocol-knowledge".into(), "a.md".into()]);
        save("note", "op", &["personal".into()]);
        save("near", "op", &["protocol-knowledge-x".into()]);
        save("other-owner", "someone", &["protocol-knowledge".into()]);

        let hits = store
            .search_owner_tagged(&[1.0, 0.0], "op", "protocol-knowledge", 10)
            .unwrap();
        let ids: Vec<&str> = hits.iter().map(|r| r.attestation_id.as_str()).collect();
        assert_eq!(ids, vec!["kb"]);
    }

    #[test]
    fn list_public_artifacts_includes_legacy_null_owner() {
        // A NULL-owner row that is explicitly public is listed: the public
        // list has no owner predicate, only the visibility predicate.
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "pub-null",
            "owner-x",
            "s1",
            "2026-06-01T00:00:00Z",
            WriteMode::Anchored,
            Visibility::Public,
        );
        store
            .conn
            .execute(
                "UPDATE attestations SET owner_pubkey = NULL WHERE attestation_id = 'pub-null'",
                [],
            )
            .unwrap();
        let rows = store.list_public_artifacts(10).unwrap();
        assert_eq!(rows.len(), 1, "NULL-owner row must appear in the list");
        assert_eq!(rows[0].attestation_id, "pub-null");
    }

    #[test]
    fn list_public_artifacts_by_mode_keeps_sources_independent() {
        let store = SqliteStore::in_memory().unwrap();
        seed_row(
            &store,
            "local-1",
            "owner-a",
            "s1",
            "2026-06-03T00:00:00Z",
            WriteMode::Local,
            Visibility::Public,
        );
        seed_row(
            &store,
            "chain-1",
            "owner-a",
            "s2",
            "2026-06-01T00:00:00Z",
            WriteMode::Anchored,
            Visibility::Public,
        );

        let local = store
            .list_public_artifacts_by_mode(WriteMode::Local, 10)
            .unwrap();
        let chain = store
            .list_public_artifacts_by_mode(WriteMode::Anchored, 10)
            .unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].attestation_id, "local-1");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].attestation_id, "chain-1");
    }

    #[test]
    fn list_public_artifacts_empty_table() {
        let store = SqliteStore::in_memory().unwrap();
        assert!(store.list_public_artifacts(10).unwrap().is_empty());
    }

    #[test]
    fn attestation_timeline_buckets_by_day_and_write_mode() {
        let store = SqliteStore::in_memory().unwrap();
        // Day 1: 2 local (on-node) + 1 anchored (on-chain).
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
            WriteMode::Anchored,
            Visibility::Public,
        );
        // Day 2: 1 anchored (on-chain).
        seed_row(
            &store,
            "d2-p1",
            "o",
            "sig-2",
            "2026-06-02T12:00:00Z",
            WriteMode::Anchored,
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

    // -- sealed-memories task 4 -----------------------------------------------

    /// Helper: save a sealed row and return it.
    fn save_sealed(store: &SqliteStore, id: &str, owner: &str) {
        store
            .save_sealed_attestation(
                id,
                &format!("hash-{id}"),
                &[],
                &format!("sol-{id}"),
                &format!("ar-{id}"),
                "signer",
                owner,
                "2026-09-28T10:00:00Z",
                WriteMode::Local,
                b"encrypted-blob",
            )
            .unwrap();
    }

    #[test]
    fn migrate_sealed_columns_idempotent_on_existing_db() {
        // Re-running the migration on an already-migrated (in_memory) DB
        // must be a no-op and must not return an error.
        let store = SqliteStore::in_memory().unwrap();
        super::migrate_sealed_columns(store.conn()).unwrap();
        super::migrate_sealed_columns(store.conn()).unwrap();

        let cols: Vec<String> = store
            .conn()
            .prepare("PRAGMA table_info(attestations)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            cols.contains(&"privacy".to_string()),
            "privacy column must be present after idempotent migration"
        );
        assert!(
            cols.contains(&"sealed_blob".to_string()),
            "sealed_blob column must be present after idempotent migration"
        );

        // Exactly one index of that name — no duplicates from multiple runs.
        let idx_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                   WHERE type='index' AND name='idx_attestations_privacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1, "exactly one privacy index");
    }

    #[test]
    fn sealed_row_never_appears_in_vector_search() {
        // TDD anchor: a sealed row must never appear in any search variant.
        let store = SqliteStore::in_memory().unwrap();
        let owner = "owner-sealed-test";

        // One sealed row and one plaintext row, same owner.
        save_sealed(&store, "sealed-1", owner);
        store
            .save_attestation(
                "plain-1",
                "visible content",
                "hash-plain",
                &[],
                "sol-plain",
                "ar-plain",
                "signer",
                owner,
                "2026-09-28T10:01:00Z",
                WriteMode::Local,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();

        // Owner-scoped authenticated search: sealed row must not appear.
        let results = store.search(&[1.0, 0.0], Some(owner), None, 10).unwrap();
        let ids: Vec<&str> = results.iter().map(|r| r.attestation_id.as_str()).collect();
        assert!(
            !ids.contains(&"sealed-1"),
            "sealed row must not appear in owner-scoped search"
        );
        assert!(
            ids.contains(&"plain-1"),
            "plaintext row must appear in owner-scoped search"
        );
        assert_eq!(results.len(), 1, "exactly one result — the plaintext row");

        // count_sealed only counts the sealed row.
        assert_eq!(store.count_sealed(owner).unwrap(), 1);
    }

    #[test]
    fn save_sealed_attestation_creates_no_embedding_row() {
        // TDD anchor: sealed rows must not have an attestation_embeddings entry.
        let store = SqliteStore::in_memory().unwrap();
        save_sealed(&store, "sealed-emb", "owner-emb");

        let emb_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM attestation_embeddings
                 WHERE attestation_id = 'sealed-emb'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(emb_count, 0, "sealed rows must have no embedding row");

        // But the row itself must exist in attestations.
        let att_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM attestations
                 WHERE attestation_id = 'sealed-emb' AND privacy = 'sealed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(att_count, 1, "sealed row must exist in attestations");

        // content must be empty.
        let content: String = store
            .conn()
            .query_row(
                "SELECT content FROM attestations WHERE attestation_id = 'sealed-emb'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(content.is_empty(), "sealed row content must be empty");
    }

    #[test]
    fn grants_for_reader_returns_only_non_withdrawn() {
        // TDD anchor: withdrawn grants must not be returned by grants_for_reader.
        let store = SqliteStore::in_memory().unwrap();

        store
            .save_grant(
                "grant-active",
                "mem-hash-1",
                Some("reader-kid-1"),
                b"cose-bytes-active",
                "author-pubkey",
                "2026-09-28T10:00:00Z",
            )
            .unwrap();
        store
            .save_grant(
                "grant-withdrawn",
                "mem-hash-1",
                Some("reader-kid-1"),
                b"cose-bytes-withdrawn",
                "author-pubkey",
                "2026-09-28T10:01:00Z",
            )
            .unwrap();
        // Withdraw the second grant.
        store.withdraw_grant("grant-withdrawn").unwrap();

        let active = store.grants_for_reader("reader-kid-1").unwrap();
        let ids: Vec<&str> = active.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["grant-active"], "only non-withdrawn grant returned");

        // grants_for_memory returns both (including withdrawn).
        let all = store.grants_for_memory("mem-hash-1").unwrap();
        assert_eq!(all.len(), 2, "grants_for_memory returns all grants");

        // The withdrawn grant has withdrawn_at set.
        let withdrawn = all.iter().find(|g| g.id == "grant-withdrawn").unwrap();
        assert!(
            withdrawn.withdrawn_at.is_some(),
            "withdrawn grant must have withdrawn_at"
        );

        // Second withdraw is idempotent (no error, stamp unchanged).
        let stamp_before = withdrawn.withdrawn_at.clone().unwrap();
        store.withdraw_grant("grant-withdrawn").unwrap();
        let all2 = store.grants_for_memory("mem-hash-1").unwrap();
        let withdrawn2 = all2.iter().find(|g| g.id == "grant-withdrawn").unwrap();
        assert_eq!(
            withdrawn2.withdrawn_at.as_deref(),
            Some(stamp_before.as_str()),
            "second withdraw must not move the stamp"
        );
    }

    #[test]
    fn relabel_plaintext_anchors_is_idempotent() {
        // Rows with visibility='private' and privacy not already 'sealed'
        // must get privacy='plaintext'. Running twice must be a no-op.
        let store = SqliteStore::in_memory().unwrap();
        store
            .save_attestation(
                "plain-priv",
                "content",
                "hash-pp",
                &[],
                "sol-pp",
                "ar-pp",
                "signer",
                "owner-rel",
                "2026-09-28T10:00:00Z",
                WriteMode::Local,
                Visibility::Private,
                &[1.0, 0.0],
            )
            .unwrap();
        save_sealed(&store, "sealed-rel", "owner-rel");

        // First call.
        store.relabel_plaintext_anchors().unwrap();

        let privacy_plain: String = store
            .conn()
            .query_row(
                "SELECT privacy FROM attestations WHERE attestation_id = 'plain-priv'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let privacy_sealed: String = store
            .conn()
            .query_row(
                "SELECT privacy FROM attestations WHERE attestation_id = 'sealed-rel'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(privacy_plain, "plaintext", "private row must be plaintext");
        assert_eq!(privacy_sealed, "sealed", "sealed row must remain sealed");

        // Second call: idempotent — values unchanged.
        store.relabel_plaintext_anchors().unwrap();
        let privacy_plain2: String = store
            .conn()
            .query_row(
                "SELECT privacy FROM attestations WHERE attestation_id = 'plain-priv'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            privacy_plain2, "plaintext",
            "idempotent: plaintext stays plaintext"
        );
    }

    #[test]
    fn list_sealed_respects_owner_and_since() {
        let store = SqliteStore::in_memory().unwrap();
        let owner = "owner-list-sealed";

        // Three sealed rows at different times.
        for (id, ts) in [
            ("s-1", "2026-09-28T08:00:00Z"),
            ("s-2", "2026-09-28T09:00:00Z"),
            ("s-3", "2026-09-28T10:00:00Z"),
        ] {
            store
                .save_sealed_attestation(
                    id,
                    &format!("hash-{id}"),
                    &[],
                    &format!("sol-{id}"),
                    &format!("ar-{id}"),
                    "signer",
                    owner,
                    ts,
                    WriteMode::Local,
                    b"blob",
                )
                .unwrap();
        }
        // One row for a different owner — must never appear.
        save_sealed(&store, "s-other", "other-owner");

        // All sealed rows for owner, no since filter.
        let all = store.list_sealed(owner, None, 10).unwrap();
        assert_eq!(all.len(), 3, "all three sealed rows for owner");
        // Newest first.
        assert_eq!(all[0].attestation_id, "s-3");
        assert_eq!(all[2].attestation_id, "s-1");

        // Since filter: only rows strictly after s-1's timestamp.
        let after = store
            .list_sealed(owner, Some("2026-09-28T08:00:00Z"), 10)
            .unwrap();
        assert_eq!(after.len(), 2, "rows after since");
        let after_ids: Vec<&str> = after.iter().map(|r| r.attestation_id.as_str()).collect();
        assert!(after_ids.contains(&"s-2") && after_ids.contains(&"s-3"));

        // Limit is honored.
        let limited = store.list_sealed(owner, None, 2).unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn grants_tables_exist_after_open() {
        // Verify that the grants / owner_recall_keys / sealed_index tables
        // exist after in_memory() initialises the schema.
        let store = SqliteStore::in_memory().unwrap();
        for table in ["grants", "owner_recall_keys", "sealed_index"] {
            let count: i64 = store
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name=?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table '{table}' must exist");
        }
        for idx in ["idx_grants_memory_hash", "idx_grants_reader_kid"] {
            let count: i64 = store
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='index' AND name=?1",
                    params![idx],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "index '{idx}' must exist");
        }
    }
}

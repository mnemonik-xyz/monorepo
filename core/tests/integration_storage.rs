//! Integration tests for the agent-native-distribution `visibility` column
//! (feature T3 / Decision 2 + 5 + 11).
//!
//! Covers:
//! - `migrate_visibility_column` idempotency on a clean DB (column + index
//!   appear; second open is a no-op).
//! - Legacy backfill: rows that predate the column get `'private'`.
//! - `save_attestation` persists the explicit `Visibility` value.
//! - `search` with `visibility_filter = Some(Public)` excludes private rows.
//! - `search` with `visibility_filter = None` returns both (authenticated
//!   callers see all their own rows — Decision 5).
//!
//! These tests use `SqliteStore::open` against a tempfile so the migration
//! path under `open()` is exercised end-to-end (the in-memory variant runs
//! the same migrations, but `open()` is what production hits).

use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};
use rusqlite::{params, Connection};

const OWNER: &str = "owner-pubkey-integration";
const SIGNER: &str = "signer-pubkey-integration";

fn open_temp_store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("att.db");
    let store = SqliteStore::open(&path).expect("open store");
    (store, dir)
}

/// Pull every column name on `attestations` so the tests can assert the
/// shape without coupling to column order.
fn attestation_columns(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("PRAGMA table_info(attestations)").unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
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

// AC: migrate_visibility_column adds the column with DEFAULT 'private' AND
// creates the visibility index on a fresh DB; re-running `open()` is a no-op.
#[test]
fn migrate_visibility_column_idempotent_on_clean_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clean.db");

    // First open runs the migration.
    {
        let store = SqliteStore::open(&path).unwrap();
        let cols = attestation_columns(store.conn());
        assert!(
            cols.iter().any(|c| c == "visibility"),
            "visibility column must exist after migration, got: {cols:?}"
        );
        assert!(
            index_exists(store.conn(), "idx_attestations_visibility"),
            "idx_attestations_visibility must be created"
        );

        // Confirm DEFAULT is 'private' via PRAGMA's dflt_value column (idx 4).
        let mut stmt = store
            .conn()
            .prepare("PRAGMA table_info(attestations)")
            .unwrap();
        let row = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, Option<String>>(4)?))
            })
            .unwrap()
            .find_map(|r| r.ok().filter(|(n, _)| n == "visibility"))
            .expect("visibility column present");
        assert_eq!(
            row.1.as_deref(),
            Some("'private'"),
            "DEFAULT must be 'private' (privacy-by-default)"
        );
    }

    // Second open must be a no-op — exactly one index of that name, same
    // column count on attestations.
    {
        let store = SqliteStore::open(&path).unwrap();
        let idx_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                   WHERE type='index' AND name='idx_attestations_visibility'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1, "index must not duplicate on re-open");
        let cols = attestation_columns(store.conn());
        assert_eq!(
            cols.iter().filter(|c| *c == "visibility").count(),
            1,
            "visibility column must not duplicate on re-open"
        );
    }
}

// AC: legacy rows (column-absent DB) get backfilled to 'private' on first
// migration. We simulate a pre-v1 DB by raw-SQL building the schema without
// the visibility column, inserting rows, then running the migration.
#[test]
fn migrate_visibility_column_backfills_legacy_rows() {
    // Standalone connection — we want to control schema construction.
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
            owner_pubkey TEXT,
            write_mode TEXT NOT NULL DEFAULT 'participate'
        );",
    )
    .unwrap();

    for id in ["att-legacy-a", "att-legacy-b", "att-legacy-c"] {
        conn.execute(
            "INSERT INTO attestations
                (attestation_id, content, content_hash, tags,
                 solana_tx, arweave_tx, signer_pubkey, created_at,
                 owner_pubkey, write_mode)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
            params![
                id,
                "legacy content",
                "h",
                "[]",
                "sol",
                "ar",
                SIGNER,
                "2026-01-01",
                OWNER,
                "participate",
            ],
        )
        .unwrap();
    }

    // SqliteStore's migration helpers are crate-private — but the migration
    // is wired into `SqliteStore::open` / `in_memory`. Open a fresh in-memory
    // store first to confirm the wiring works, then reproduce the legacy
    // shape via the standalone connection above and assert backfill by
    // running the public API: open a tempfile-backed store from a path that
    // points at the same on-disk DB we just built.
    //
    // The simplest portable path: snapshot the legacy DB to a tempfile,
    // then open it via SqliteStore::open and assert visibility was
    // backfilled to 'private' on every row.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    // Build the same legacy schema directly on the file.
    {
        let file_conn = Connection::open(&path).unwrap();
        file_conn
            .execute_batch(
                "CREATE TABLE attestations (
                    attestation_id TEXT PRIMARY KEY,
                    content TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    tags TEXT NOT NULL DEFAULT '[]',
                    solana_tx TEXT NOT NULL,
                    arweave_tx TEXT NOT NULL,
                    signer_pubkey TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    owner_pubkey TEXT,
                    write_mode TEXT NOT NULL DEFAULT 'participate'
                );",
            )
            .unwrap();
        for id in ["att-legacy-a", "att-legacy-b", "att-legacy-c"] {
            file_conn
                .execute(
                    "INSERT INTO attestations
                        (attestation_id, content, content_hash, tags,
                         solana_tx, arweave_tx, signer_pubkey, created_at,
                         owner_pubkey, write_mode)
                     VALUES (?,?,?,?,?,?,?,?,?,?)",
                    params![
                        id,
                        "legacy content",
                        "h",
                        "[]",
                        "sol",
                        "ar",
                        SIGNER,
                        "2026-01-01",
                        OWNER,
                        "participate",
                    ],
                )
                .unwrap();
        }
    }

    // Run the migration via the production open() path.
    let store = SqliteStore::open(&path).expect("open legacy db runs migration");

    // Backfill: every pre-existing row must now have visibility='private'.
    let mut stmt = store
        .conn()
        .prepare("SELECT visibility FROM attestations ORDER BY attestation_id")
        .unwrap();
    let visibilities: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        visibilities,
        vec!["private", "private", "private"],
        "every legacy row must be backfilled to 'private'"
    );
}

// AC: save_attestation persists the explicit `visibility` argument.
#[test]
fn save_attestation_persists_visibility() {
    let (store, _dir) = open_temp_store();

    store
        .save_attestation(
            "att-public-1",
            "shareable content",
            "hash-public-1",
            &["public".to_string()],
            "sol-tx-public",
            "ar-tx-public",
            SIGNER,
            OWNER,
            "2026-01-01T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
            &[0.5, 0.5],
        )
        .expect("save public row");

    store
        .save_attestation(
            "att-private-1",
            "private content",
            "hash-private-1",
            &["private".to_string()],
            "sol-tx-private",
            "ar-tx-private",
            SIGNER,
            OWNER,
            "2026-01-01T00:00:00Z",
            WriteMode::Participate,
            Visibility::Private,
            &[1.0, 0.0],
        )
        .expect("save private row");

    let stored: Vec<(String, String)> = {
        let mut stmt = store
            .conn()
            .prepare("SELECT attestation_id, visibility FROM attestations ORDER BY attestation_id")
            .unwrap();
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    };

    assert_eq!(
        stored,
        vec![
            ("att-private-1".to_string(), "private".to_string()),
            ("att-public-1".to_string(), "public".to_string()),
        ]
    );
}

// AC13: search with visibility_filter = Some(Public) returns only public rows.
#[test]
fn search_visibility_filter_excludes_private() {
    let (store, _dir) = open_temp_store();
    let query = [1.0_f32, 0.0_f32];

    // Both rows match the query exactly (same embedding) so the filter is
    // the discriminator, not the relevance score.
    store
        .save_attestation(
            "att-public",
            "public memory",
            "h-public",
            &[],
            "sol-public",
            "ar-public",
            SIGNER,
            OWNER,
            "2026-01-01",
            WriteMode::Participate,
            Visibility::Public,
            &query,
        )
        .unwrap();
    store
        .save_attestation(
            "att-private",
            "private memory",
            "h-private",
            &[],
            "sol-private",
            "ar-private",
            SIGNER,
            OWNER,
            "2026-01-01",
            WriteMode::Participate,
            Visibility::Private,
            &query,
        )
        .unwrap();

    let public_only = store
        .search(&query, OWNER, Some(Visibility::Public), 10)
        .expect("search succeeds");
    assert_eq!(public_only.len(), 1, "expected exactly one row");
    assert_eq!(public_only[0].attestation_id, "att-public");
    assert_eq!(public_only[0].visibility, Visibility::Public);

    // Symmetrically, Some(Private) returns only the private row — supports
    // authenticated callers who want to enumerate their own private set.
    let private_only = store
        .search(&query, OWNER, Some(Visibility::Private), 10)
        .expect("search succeeds");
    assert_eq!(private_only.len(), 1);
    assert_eq!(private_only[0].attestation_id, "att-private");
    assert_eq!(private_only[0].visibility, Visibility::Private);
}

// AC13 / Decision 5: search with no visibility filter returns both rows —
// authenticated callers see all of their own rows regardless of visibility.
#[test]
fn search_no_filter_returns_both() {
    let (store, _dir) = open_temp_store();
    let query = [1.0_f32, 0.0_f32];

    store
        .save_attestation(
            "att-public",
            "public memory",
            "h-public",
            &[],
            "sol-public",
            "ar-public",
            SIGNER,
            OWNER,
            "2026-01-01",
            WriteMode::Participate,
            Visibility::Public,
            &query,
        )
        .unwrap();
    store
        .save_attestation(
            "att-private",
            "private memory",
            "h-private",
            &[],
            "sol-private",
            "ar-private",
            SIGNER,
            OWNER,
            "2026-01-01",
            WriteMode::Participate,
            Visibility::Private,
            &query,
        )
        .unwrap();

    let both = store.search(&query, OWNER, None, 10).expect("search ok");
    assert_eq!(
        both.len(),
        2,
        "authenticated owner must see both private and public rows"
    );
    let ids: Vec<&str> = both.iter().map(|r| r.attestation_id.as_str()).collect();
    assert!(ids.contains(&"att-public"));
    assert!(ids.contains(&"att-private"));
}

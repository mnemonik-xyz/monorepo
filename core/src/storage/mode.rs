//! Per-attestation write mode and visibility.
//!
//! Two pure types — no I/O, no protocol coupling — that encode the per-request
//! user intents the protocol surfaces in JSON.
//!
//! `WriteMode` encodes the modes-user-choice intent: either keep the artifact
//! local (free, offline) or anchor it on Arweave + Solana (paid, durable,
//! verifiable).
//!
//! `Visibility` encodes the agent-native-distribution intent for
//! anchored-mode writes: `Private` (default) keeps the row hidden from
//! anonymous discovery; `Public` opts the row into anonymous recall. Local
//! writes never carry a meaningful visibility — they don't leave the user's
//! machine — so the resolver in `mcp/` rejects `mode=local + visibility=...`
//! at the boundary (Decision 3 / AC14). The column lives on every row so the
//! storage layer can filter without a join.
//!
//! Both types live in `core/` so both the storage layer (which persists them
//! on every attestation row) and the MCP layer (which resolves them from JSON
//! input) share one type. JSON wire format is lowercase
//! (`"local"`/`"anchored"`, `"private"`/`"public"`) to match the user-spec
//! tables; rusqlite round-trips via the same lowercase strings stored in the
//! `attestations.write_mode` and `attestations.visibility` columns.

use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    ToSql,
};
use serde::{Deserialize, Serialize};

/// Per-attestation write intent.
///
/// `Local` — artifact stays on the user's own filesystem / self-hosted store.
/// Free, offline. Whitepaper §5.7.1 guaranteed-free path.
///
/// `Anchored` — artifact is stored on Arweave, with its hash in a Solana SPL
/// Memo, and proved retrievable. Paid service-layer path; "delivered =
/// anchored AND verified".
///
/// The two modes name *where the memory lives*, and nothing else: `Local` means
/// the agent's own machine only, `Anchored` means Arweave.
///
/// Default is `Local` — the user-spec default ("default `local`; кто ничего
/// не настраивал получает бесплатную личную память").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    #[default]
    Local,
    /// Renamed from `Anchored` on 2026-09-27
    /// (work/arweave-as-source-of-truth). The serde alias keeps existing
    /// clients and existing database rows readable; see `from_str_strict`.
    #[serde(alias = "participate")]
    Anchored,
}

/// Value of the optional signed `anchor` field on an anchored artifact
/// (work/arweave-as-source-of-truth). It names the durable backend that holds
/// the bytes, so a restore can recover `WriteMode` from the artifact itself
/// rather than from a database column. A second backend (issue #70) adds a new
/// value here; it never changes this one.
pub const ANCHOR_ARWEAVE: &str = "arweave";

/// Deprecated wire and column spelling of [`WriteMode::Anchored`]. Accepted on
/// input for one release, never produced on output. Remove it once the release
/// that introduced `"anchored"` is the oldest supported client.
pub const LEGACY_ANCHORED_TOKEN: &str = "participate";

impl WriteMode {
    /// Canonical lowercase string form. This is the on-the-wire (JSON) and
    /// in-DB representation — both sides round-trip via this exact spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            WriteMode::Local => "local",
            WriteMode::Anchored => "anchored",
        }
    }

    /// Parser for the canonical lowercase tokens `"local"` / `"anchored"`,
    /// plus the deprecated alias `"anchored"` for [`WriteMode::Anchored`].
    /// Every other input — case-variant (`"Local"`, `"ANCHORED"`), empty,
    /// whitespace, unknown, trailing-space — returns `None`.
    ///
    /// Strictness is otherwise intentional: the resolver in `mcp/` maps `None`
    /// to a typed JSON-RPC `-32602 InvalidParams` error rather than silently
    /// downgrading or normalizing. Loosening this contract further would let
    /// hand-crafted clients drift from the documented wire format.
    ///
    /// The one alias is what makes the rename free of a data migration: this
    /// function backs [`FromSql`], so a row written as `'participate'` before
    /// the rename still reads back as [`WriteMode::Anchored`]. New writes
    /// always store `'anchored'`, so the column may legitimately hold both
    /// spellings. Never compare the column against a string literal in SQL —
    /// see the query helpers in `core/src/storage/sqlite.rs`.
    pub fn from_str_strict(s: &str) -> Option<Self> {
        match s {
            "local" => Some(WriteMode::Local),
            "anchored" => Some(WriteMode::Anchored),
            LEGACY_ANCHORED_TOKEN => Some(WriteMode::Anchored),
            _ => None,
        }
    }
}

impl ToSql for WriteMode {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(
            self.as_str().as_bytes(),
        )))
    }
}

// SAFETY note (security-auditor round 1, deferred to T2):
// The error path below echoes the raw column value back through
// `FromSqlError::Other`. That is acceptable here because the only inputs
// that ever reach this column are (a) the literal `'participate'` DEFAULT
// added by `migrate_write_mode_column`, (b) the lowercase strings written
// by `WriteMode::to_sql` via `save_attestation`, and (c) the legacy
// backfill UPDATE which writes only `'local'`. Once T2 lands the JSON-input
// resolver in `mcp/`, every user-supplied `mode` value is rejected at the
// dispatcher boundary (`-32602 InvalidParams`) before it could be persisted —
// so this error variant only fires on a tampered DB or a future migration
// bug, where echoing the value is a useful diagnostic, not a leak vector.
// T2 owns the input boundary; revisit this comment if the assumption shifts.
impl FromSql for WriteMode {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        WriteMode::from_str_strict(s).ok_or_else(|| {
            FromSqlError::Other(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown write_mode column value: {s:?}"),
            )))
        })
    }
}

/// Per-attestation visibility (agent-native-distribution).
///
/// `Private` — default. Row is invisible to anonymous (unauthenticated)
/// `recall`. Authenticated owners always see their own private rows.
///
/// `Public` — row is included in anonymous `recall` results. Only valid on
/// `WriteMode::Anchored` writes; the resolver in `mcp/` rejects
/// `mode=local + visibility=...` at the JSON-RPC boundary (Decision 3 / AC14).
///
/// Default is `Private` — user-spec privacy-by-default for anchored writes
/// (the column exists on every row, so the `'private'` default also covers
/// the legacy backfill case).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    #[default]
    Private,
    Public,
}

impl Visibility {
    /// Canonical lowercase string form — on-the-wire (JSON) and in-DB
    /// representation. Round-trips with `from_str_strict`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Public => "public",
        }
    }

    /// Strict parser: accepts ONLY the canonical lowercase tokens
    /// `"private"` / `"public"`. Mirrors `WriteMode::from_str_strict` —
    /// case variants, whitespace, unknown values all return `None` so the
    /// MCP resolver can surface a typed `-32602 InvalidParams` error rather
    /// than silently normalizing.
    pub fn from_str_strict(s: &str) -> Option<Self> {
        match s {
            "private" => Some(Visibility::Private),
            "public" => Some(Visibility::Public),
            _ => None,
        }
    }
}

impl ToSql for Visibility {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(
            self.as_str().as_bytes(),
        )))
    }
}

// SAFETY note: mirrors the WriteMode FromSql rationale above. The error path
// echoes the raw column value through `FromSqlError::Other` because the only
// values that ever reach this column are (a) the literal `'private'` DEFAULT
// added by `migrate_visibility_column`, (b) the lowercase strings written by
// `Visibility::to_sql` via `save_attestation`, and (c) the legacy backfill
// UPDATE which writes only `'private'`. User-supplied visibility values are
// rejected at the MCP dispatcher boundary before persistence, so this
// variant only fires on a tampered DB or future migration bug.
impl FromSql for Visibility {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        Visibility::from_str_strict(s).ok_or_else(|| {
            FromSqlError::Other(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown visibility column value: {s:?}"),
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn write_mode_default_is_local() {
        assert_eq!(WriteMode::default(), WriteMode::Local);
    }

    #[test]
    fn write_mode_as_str_round_trips_with_from_str_strict() {
        for variant in [WriteMode::Local, WriteMode::Anchored] {
            let s = variant.as_str();
            assert_eq!(WriteMode::from_str_strict(s), Some(variant));
        }
    }

    #[test]
    fn write_mode_from_str_strict_accepts_only_canonical_lowercase() {
        assert_eq!(WriteMode::from_str_strict("local"), Some(WriteMode::Local));
        assert_eq!(
            WriteMode::from_str_strict("anchored"),
            Some(WriteMode::Anchored)
        );
        // Deprecated alias, kept for one release so existing clients and
        // un-migrated rows keep parsing. See LEGACY_ANCHORED_TOKEN.
        assert_eq!(
            WriteMode::from_str_strict(LEGACY_ANCHORED_TOKEN),
            Some(WriteMode::Anchored)
        );
        for bad in [
            "Local",
            "PARTICIPATE",
            "Anchored",
            "Anchored",
            "ANCHORED",
            "LOCAL",
            "",
            " ",
            "  ",
            "unknown",
            "local ",
            " local",
            "anchored\n",
            "null",
            "0",
        ] {
            assert_eq!(
                WriteMode::from_str_strict(bad),
                None,
                "input {bad:?} must not parse"
            );
        }
    }

    #[test]
    fn write_mode_serde_json_round_trip() {
        // Local
        let json = serde_json::to_string(&WriteMode::Local).unwrap();
        assert_eq!(json, "\"local\"");
        let back: WriteMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WriteMode::Local);

        // Anchored serializes to the canonical spelling only.
        let json = serde_json::to_string(&WriteMode::Anchored).unwrap();
        assert_eq!(json, "\"anchored\"");
        let back: WriteMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WriteMode::Anchored);
    }

    /// The rename must not break a client that still sends the old token.
    /// Deserialization accepts it; serialization never emits it again.
    #[test]
    fn write_mode_serde_accepts_legacy_participate_alias() {
        let back: WriteMode = serde_json::from_str("\"anchored\"").unwrap();
        assert_eq!(back, WriteMode::Anchored);
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            "\"anchored\"",
            "the legacy token must never be produced on output"
        );
    }

    #[test]
    fn write_mode_serde_rejects_non_canonical_case() {
        // Serde uses the same `rename_all = "lowercase"` mapping; uppercase
        // and capitalized inputs are unknown variants.
        assert!(serde_json::from_str::<WriteMode>("\"Local\"").is_err());
        assert!(serde_json::from_str::<WriteMode>("\"PARTICIPATE\"").is_err());
        assert!(serde_json::from_str::<WriteMode>("\"unknown\"").is_err());
        assert!(serde_json::from_str::<WriteMode>("null").is_err());
    }

    #[test]
    fn write_mode_rusqlite_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, mode TEXT NOT NULL)")
            .unwrap();

        for mode in [WriteMode::Local, WriteMode::Anchored] {
            conn.execute("INSERT INTO t (mode) VALUES (?)", rusqlite::params![mode])
                .unwrap();
        }

        let mut stmt = conn.prepare("SELECT mode FROM t ORDER BY id").unwrap();
        let modes: Vec<WriteMode> = stmt
            .query_map([], |row| row.get::<_, WriteMode>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(modes, vec![WriteMode::Local, WriteMode::Anchored]);
    }

    #[test]
    fn write_mode_rusqlite_from_sql_rejects_unknown_value() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (mode TEXT NOT NULL)")
            .unwrap();
        conn.execute("INSERT INTO t (mode) VALUES ('weird')", [])
            .unwrap();

        let err = conn
            .query_row("SELECT mode FROM t", [], |row| row.get::<_, WriteMode>(0))
            .unwrap_err();
        // rusqlite wraps FromSqlError in `FromSqlConversionFailure`.
        let msg = err.to_string();
        assert!(
            msg.contains("write_mode") || msg.contains("weird"),
            "error should mention the bad column or value, got: {msg}"
        );
    }

    // -- Visibility (agent-native-distribution) ----------------------------

    #[test]
    fn visibility_default_is_private() {
        assert_eq!(Visibility::default(), Visibility::Private);
    }

    #[test]
    fn visibility_as_str_round_trips_with_from_str_strict() {
        for variant in [Visibility::Private, Visibility::Public] {
            let s = variant.as_str();
            assert_eq!(Visibility::from_str_strict(s), Some(variant));
        }
    }

    #[test]
    fn visibility_from_str_strict_accepts_only_canonical_lowercase() {
        assert_eq!(
            Visibility::from_str_strict("private"),
            Some(Visibility::Private)
        );
        assert_eq!(
            Visibility::from_str_strict("public"),
            Some(Visibility::Public)
        );
        for bad in [
            "Private",
            "PUBLIC",
            "Public",
            "PRIVATE",
            "",
            " ",
            "  ",
            "unknown",
            "public ",
            " public",
            "private\n",
            "null",
            "0",
        ] {
            assert_eq!(
                Visibility::from_str_strict(bad),
                None,
                "input {bad:?} must not parse"
            );
        }
    }

    #[test]
    fn visibility_serde_json_round_trip() {
        let json = serde_json::to_string(&Visibility::Private).unwrap();
        assert_eq!(json, "\"private\"");
        let back: Visibility = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Visibility::Private);

        let json = serde_json::to_string(&Visibility::Public).unwrap();
        assert_eq!(json, "\"public\"");
        let back: Visibility = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Visibility::Public);
    }

    #[test]
    fn visibility_serde_rejects_non_canonical_case() {
        assert!(serde_json::from_str::<Visibility>("\"Private\"").is_err());
        assert!(serde_json::from_str::<Visibility>("\"PUBLIC\"").is_err());
        assert!(serde_json::from_str::<Visibility>("\"unknown\"").is_err());
        assert!(serde_json::from_str::<Visibility>("null").is_err());
    }

    #[test]
    fn visibility_rusqlite_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL)")
            .unwrap();

        for v in [Visibility::Private, Visibility::Public] {
            conn.execute("INSERT INTO t (v) VALUES (?)", rusqlite::params![v])
                .unwrap();
        }

        let mut stmt = conn.prepare("SELECT v FROM t ORDER BY id").unwrap();
        let values: Vec<Visibility> = stmt
            .query_map([], |row| row.get::<_, Visibility>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(values, vec![Visibility::Private, Visibility::Public]);
    }

    #[test]
    fn visibility_rusqlite_from_sql_rejects_unknown_value() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (v TEXT NOT NULL)")
            .unwrap();
        conn.execute("INSERT INTO t (v) VALUES ('weird')", [])
            .unwrap();

        let err = conn
            .query_row("SELECT v FROM t", [], |row| row.get::<_, Visibility>(0))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("visibility") || msg.contains("weird"),
            "error should mention the bad column or value, got: {msg}"
        );
    }
}

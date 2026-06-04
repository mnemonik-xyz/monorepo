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
//! participate-mode writes: `Private` (default) keeps the row hidden from
//! anonymous discovery; `Public` opts the row into anonymous recall. Local
//! writes never carry a meaningful visibility — they don't leave the user's
//! machine — so the resolver in `mcp/` rejects `mode=local + visibility=...`
//! at the boundary (Decision 3 / AC14). The column lives on every row so the
//! storage layer can filter without a join.
//!
//! Both types live in `core/` so both the storage layer (which persists them
//! on every attestation row) and the MCP layer (which resolves them from JSON
//! input) share one type. JSON wire format is lowercase
//! (`"local"`/`"participate"`, `"private"`/`"public"`) to match the user-spec
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
/// `Participate` — artifact is anchored on Arweave + Solana and proved
/// retrievable. Paid service-layer path; "delivered = anchored AND verified
/// by recall."
///
/// Default is `Local` — the user-spec default ("default `local`; кто ничего
/// не настраивал получает бесплатную личную память").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    #[default]
    Local,
    Participate,
}

impl WriteMode {
    /// Canonical lowercase string form. This is the on-the-wire (JSON) and
    /// in-DB representation — both sides round-trip via this exact spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            WriteMode::Local => "local",
            WriteMode::Participate => "participate",
        }
    }

    /// Strict parser: accepts ONLY the exact canonical lowercase tokens
    /// `"local"` / `"participate"`. Every other input — case-variant
    /// (`"Local"`, `"PARTICIPATE"`), empty, whitespace, unknown,
    /// trailing-space — returns `None`.
    ///
    /// Strictness is intentional: the resolver in `mcp/` maps `None` to a
    /// typed JSON-RPC `-32602 InvalidParams` error rather than silently
    /// downgrading or normalizing. Loosening this contract would let
    /// hand-crafted clients drift from the documented wire format.
    pub fn from_str_strict(s: &str) -> Option<Self> {
        match s {
            "local" => Some(WriteMode::Local),
            "participate" => Some(WriteMode::Participate),
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
/// `WriteMode::Participate` writes; the resolver in `mcp/` rejects
/// `mode=local + visibility=...` at the JSON-RPC boundary (Decision 3 / AC14).
///
/// Default is `Private` — user-spec privacy-by-default for participate writes
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
        for variant in [WriteMode::Local, WriteMode::Participate] {
            let s = variant.as_str();
            assert_eq!(WriteMode::from_str_strict(s), Some(variant));
        }
    }

    #[test]
    fn write_mode_from_str_strict_accepts_only_canonical_lowercase() {
        assert_eq!(WriteMode::from_str_strict("local"), Some(WriteMode::Local));
        assert_eq!(
            WriteMode::from_str_strict("participate"),
            Some(WriteMode::Participate)
        );
        for bad in [
            "Local",
            "PARTICIPATE",
            "Participate",
            "LOCAL",
            "",
            " ",
            "  ",
            "unknown",
            "local ",
            " local",
            "participate\n",
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

        // Participate
        let json = serde_json::to_string(&WriteMode::Participate).unwrap();
        assert_eq!(json, "\"participate\"");
        let back: WriteMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WriteMode::Participate);
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

        for mode in [WriteMode::Local, WriteMode::Participate] {
            conn.execute("INSERT INTO t (mode) VALUES (?)", rusqlite::params![mode])
                .unwrap();
        }

        let mut stmt = conn.prepare("SELECT mode FROM t ORDER BY id").unwrap();
        let modes: Vec<WriteMode> = stmt
            .query_map([], |row| row.get::<_, WriteMode>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(modes, vec![WriteMode::Local, WriteMode::Participate]);
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

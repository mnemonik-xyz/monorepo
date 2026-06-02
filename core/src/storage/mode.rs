//! Per-attestation write mode.
//!
//! `WriteMode` is a pure type — no I/O, no protocol coupling. It encodes the
//! single per-request user intent that the modes-user-choice feature spec
//! reduces the surface to: either keep the artifact local (free, offline) or
//! anchor it on Arweave + Solana (paid, durable, verifiable).
//!
//! Lives in `core/` so both the storage layer (which persists it on every
//! attestation row) and the MCP layer (which resolves it from JSON input)
//! share one type. JSON wire format is lowercase (`"local"` / `"participate"`)
//! to match the user-spec table; rusqlite round-trips via the same lowercase
//! strings stored in the new `attestations.write_mode` column.

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
}

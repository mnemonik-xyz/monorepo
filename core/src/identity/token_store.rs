//! File-backed read/write/delete for `~/.mnemonic/token.json`.
//!
//! Shared on-disk format with the Node CLI at
//! `packages/cli/src/config.ts:39-65`. The JWT is the bearer token the MCP
//! server issued at `/oauth/token`; `expires_at` is an **ISO-8601 string**
//! (NOT a unix-seconds integer) — matches `config.ts:42` field-for-field
//! and the validator at `config.ts:354`.
//!
//! Decision 7 (agent-native-distribution tech-spec): the V1 token cache is
//! file-only — no OS keychain wrapper. Tokens are short-lived (1 hour TTL
//! per `JWT_TTL_SECS` in `mcp/src/oauth/mod.rs:58`) and re-OAuth is cheap,
//! so the file is the canonical store across both Node CLI and Rust binary.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

/// On-disk token shape. Field-for-field copy of Node CLI's `TokenJson`
/// (`packages/cli/src/config.ts:39-65`). `expires_at` is ISO-8601 — parsed
/// via [`DateTime::parse_from_rfc3339`] for comparisons; an unparseable
/// timestamp is treated as malformed → [`read_token`] returns `Ok(None)`
/// so the caller re-OAuths instead of crashing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenJson {
    pub jwt: String,
    pub expires_at: String,
    pub sub: String,
}

/// Errors surfaced by the token-store helpers.
///
/// `Expired` is the only variant that carries semantic meaning for the
/// JSON-RPC boundary — it maps to `-32099 TokenExpired`. The other two
/// variants surface unexpected I/O / serde failures that callers should
/// treat as fatal.
#[derive(Debug)]
pub enum TokenStoreError {
    /// `read_token` parsed a well-formed token whose `expires_at` is in the
    /// past. Mapped to JSON-RPC `-32099 TokenExpired` at the boundary.
    Expired {
        expires_at: String,
        sub: String,
    },
    Io(std::io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for TokenStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenStoreError::Expired { expires_at, .. } => {
                write!(f, "token expired at {expires_at}")
            }
            TokenStoreError::Io(e) => write!(f, "token I/O error: {e}"),
            TokenStoreError::Parse(e) => write!(f, "token parse error: {e}"),
        }
    }
}

impl std::error::Error for TokenStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TokenStoreError::Expired { .. } => None,
            TokenStoreError::Io(e) => Some(e),
            TokenStoreError::Parse(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for TokenStoreError {
    fn from(e: std::io::Error) -> Self {
        TokenStoreError::Io(e)
    }
}

impl From<serde_json::Error> for TokenStoreError {
    fn from(e: serde_json::Error) -> Self {
        TokenStoreError::Parse(e)
    }
}

/// `~/.mnemonic/token.json`. Falls back to a synthetic absolute path on the
/// `dirs::home_dir() == None` edge case (HOME unset on a non-standard host);
/// callers see the failure surface through [`read_token`]/[`save_token`].
pub fn token_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".mnemonic")
        .join("token.json")
}

/// Read the token at the production path. See [`read_token_from`] for the
/// path-injection variant used by tests.
pub fn read_token() -> Result<Option<TokenJson>, TokenStoreError> {
    read_token_from(&token_path())
}

/// Read a token from `path`.
///
/// Returns:
/// - `Ok(None)` — file is missing, unreadable as JSON, or the JSON does not
///   match `TokenJson`. This degrades to "no usable token; re-OAuth"
///   rather than crashing the binary. Decision 7 + Round 3 test review.
/// - `Ok(Some(token))` — file parsed and `expires_at` is in the future.
/// - `Err(TokenStoreError::Expired)` — file parsed but `expires_at` is in
///   the past (or unparseable as RFC 3339, which we treat as expired —
///   "I don't know when this expires" is safer than "assume it's fine").
pub fn read_token_from(path: &Path) -> Result<Option<TokenJson>, TokenStoreError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TokenStoreError::Io(e)),
    };
    let token: TokenJson = match serde_json::from_slice(&bytes) {
        Ok(t) => t,
        // Malformed JSON or wrong shape — degrade to "no token". The caller
        // re-OAuths; corrupted state must NEVER hard-fail the binary.
        Err(_) => return Ok(None),
    };
    let exp = match DateTime::parse_from_rfc3339(&token.expires_at) {
        Ok(dt) => dt.with_timezone(&Utc),
        // Unparseable timestamp — treat as expired so the caller re-OAuths
        // rather than silently accepting an undated token.
        Err(_) => {
            return Err(TokenStoreError::Expired {
                expires_at: token.expires_at,
                sub: token.sub,
            });
        }
    };
    if exp <= Utc::now() {
        return Err(TokenStoreError::Expired {
            expires_at: token.expires_at,
            sub: token.sub,
        });
    }
    Ok(Some(token))
}

/// Save `token` to the production path (creating `~/.mnemonic/` if needed).
pub fn save_token(token: &TokenJson) -> Result<(), TokenStoreError> {
    save_token_to(&token_path(), token)
}

/// Atomic write of `token` to `path`. Tempfile-in-same-dir + rename so a
/// concurrent reader sees either the previous file contents or the new
/// ones — never a half-written file. Mode 0600 on Unix.
pub fn save_token_to(path: &Path, token: &TokenJson) -> Result<(), TokenStoreError> {
    let parent = path.parent().ok_or_else(|| {
        TokenStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "token path has no parent directory",
        ))
    })?;
    fs::create_dir_all(parent)?;

    let mut tmp = NamedTempFile::new_in(parent)?;
    let json = serde_json::to_string_pretty(token)?;
    tmp.write_all(json.as_bytes())?;
    tmp.as_file().sync_all()?;

    // Set mode 0600 on the tempfile BEFORE persist — set_mode after persist
    // would race with a reader who opened the file in the interim. Persist
    // preserves the source file's mode across the rename.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o600))?;
    }

    tmp.persist(path)
        .map_err(|e| TokenStoreError::Io(e.error))?;
    Ok(())
}

/// Delete the production-path token. Idempotent — removing a missing file
/// is `Ok(())`. Backing the Task 5 logout subcommand.
pub fn delete_token() -> Result<(), TokenStoreError> {
    delete_token_at(&token_path())
}

/// Delete the token at `path`. Idempotent on `NotFound`.
pub fn delete_token_at(path: &Path) -> Result<(), TokenStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TokenStoreError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample(expires_at: &str) -> TokenJson {
        TokenJson {
            jwt: "header.payload.sig".to_string(),
            expires_at: expires_at.to_string(),
            sub: "TestPubkey111111111111111111111111111111111".to_string(),
        }
    }

    #[test]
    fn save_then_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("token.json");
        let future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let original = sample(&future);
        save_token_to(&path, &original).unwrap();
        let loaded = read_token_from(&path).unwrap().unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("never-written.json");
        assert!(read_token_from(&path).unwrap().is_none());
    }

    #[test]
    fn read_malformed_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("token.json");
        fs::write(&path, b"not valid json {{{").unwrap();
        assert!(read_token_from(&path).unwrap().is_none());
    }

    #[test]
    fn delete_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("token.json");
        delete_token_at(&path).unwrap();
        delete_token_at(&path).unwrap();
    }

    #[test]
    fn unparseable_timestamp_returns_expired() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("token.json");
        let bad = sample("not-an-iso-timestamp");
        save_token_to(&path, &bad).unwrap();
        let err = read_token_from(&path).unwrap_err();
        match err {
            TokenStoreError::Expired { expires_at, .. } => {
                assert_eq!(expires_at, "not-an-iso-timestamp");
            }
            other => panic!("expected Expired, got {other:?}"),
        }
    }
}

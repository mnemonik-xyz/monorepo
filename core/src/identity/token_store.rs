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

/// `~/.mnemonic/token.json`.
///
/// `$MNEMONIC_CONFIG_DIR`, when set and non-empty, overrides the base
/// directory — parity with the Node CLI's `configDir()` at
/// `packages/cli/src/config.ts:48-52`. This lets integration tests sandbox
/// the token path without mutating `$HOME` (which is process-global and
/// requires `unsafe` on Rust 2024). When the override is absent the
/// production path is `~/.mnemonic/token.json` via [`dirs::home_dir`].
///
/// Returns [`TokenStoreError::Io`] (with `InvalidInput` kind and the
/// message "cannot determine home directory; set $HOME or
/// $MNEMONIC_CONFIG_DIR") when neither the env override is set nor
/// `dirs::home_dir()` resolves a HOME. Round-1 security audit SA6-001:
/// the spec contract is "token file must live inside HOME, never
/// elsewhere"; the previous infallible signature silently fell back to
/// `/.mnemonic/token.json` which would attempt to write at the
/// filesystem root.
pub fn token_path() -> Result<PathBuf, TokenStoreError> {
    if let Some(override_dir) = std::env::var_os("MNEMONIC_CONFIG_DIR") {
        let s = override_dir.to_string_lossy();
        if !s.is_empty() {
            return Ok(PathBuf::from(override_dir).join("token.json"));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| {
        TokenStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "cannot determine home directory; set $HOME or $MNEMONIC_CONFIG_DIR",
        ))
    })?;
    Ok(home.join(".mnemonic").join("token.json"))
}

/// Read the token at the production path. See [`read_token_from`] for the
/// path-injection variant used by tests. Propagates the
/// [`TokenStoreError::Io`] from [`token_path`] when neither
/// `MNEMONIC_CONFIG_DIR` nor `HOME` is resolvable.
pub fn read_token() -> Result<Option<TokenJson>, TokenStoreError> {
    read_token_from(&token_path()?)
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
        // rather than silently accepting an undated token. The tech-spec's
        // "returns None" language at line 332 was superseded by the Task 6
        // TDD anchor `expired_token_returns_expired_error` and refined here:
        // "I don't know when this expires" is safer than "assume valid".
        // Decisions.md (Task 6) records the spec discrepancy.
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
/// Propagates the [`TokenStoreError::Io`] from [`token_path`] when neither
/// `MNEMONIC_CONFIG_DIR` nor `HOME` is resolvable.
pub fn save_token(token: &TokenJson) -> Result<(), TokenStoreError> {
    save_token_to(&token_path()?, token)
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
    // Tighten the parent directory to 0o700 on Unix — round-1 security
    // audit SA6-002. `fs::create_dir_all` honours umask, which on many
    // Linux distros leaves world-readable mode 0o755; the directory
    // listing exposes "this user has a Mnemonic token" even though the
    // token file itself is 0o600. `set_permissions` here is idempotent
    // and matches the Node CLI's `mkdirSync(dir, { recursive: true,
    // mode: 0o700 })` at `packages/cli/src/config.ts:71`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort tightening — if the directory already exists with
        // different ownership (shared dev machine), this may fail with
        // EPERM. Don't bubble that as a save failure since the file mode
        // 0o600 still protects the contents.
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }

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
/// is `Ok(())`. Backing the Task 5 logout subcommand. Propagates the
/// [`TokenStoreError::Io`] from [`token_path`] when neither
/// `MNEMONIC_CONFIG_DIR` nor `HOME` is resolvable.
pub fn delete_token() -> Result<(), TokenStoreError> {
    delete_token_at(&token_path()?)
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

    /// Verify `$MNEMONIC_CONFIG_DIR` overrides the base directory. This is
    /// the only test that mutates the env var — the integration tests use
    /// the path-injected `save_token_to`/`read_token_from` variants
    /// (round-2 code review R2-NOTE-1). Guarded by [`ENV_GUARD`] so it
    /// does not race with itself across test re-runs in the same process.
    #[test]
    fn config_dir_override_routes_through_token_path() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let previous = std::env::var_os("MNEMONIC_CONFIG_DIR");
        // SAFETY: this test owns ENV_GUARD for the duration of the
        // mutation and restores the prior value on exit. The only other
        // code reading `MNEMONIC_CONFIG_DIR` in this binary is
        // `token_path()` itself, which we call inside the lock.
        unsafe {
            std::env::set_var("MNEMONIC_CONFIG_DIR", dir.path());
        }
        let resolved = token_path();
        unsafe {
            match previous {
                Some(v) => std::env::set_var("MNEMONIC_CONFIG_DIR", v),
                None => std::env::remove_var("MNEMONIC_CONFIG_DIR"),
            }
        }
        let resolved = resolved.expect("token_path must succeed under override");
        assert_eq!(resolved, dir.path().join("token.json"));
    }

    /// Shared guard for tests that mutate `MNEMONIC_CONFIG_DIR`. The
    /// `into_inner()` pattern is intentional: a panic inside a prior test
    /// leaves the env var possibly stale, but every guarded test sets the
    /// env var explicitly before reading, so a poisoned lock does not
    /// corrupt the next test's observed environment.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

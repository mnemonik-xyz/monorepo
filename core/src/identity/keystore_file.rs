//! Filesystem-backed `KeyStore` implementation.
//!
//! Reads and writes `~/.mnemonic/identity.json` (or any path provided at
//! construction time) using the legacy JSON shape:
//!
//! ```json
//! {"secret":[...64 bytes...],"pubkey_base58":"..."}
//! ```
//!
//! `set` uses an atomic `NamedTempFile::new_in(parent) → persist()` write so
//! a crash mid-write never leaves a partially-written file.  Mode 0600 is
//! enforced on Unix.
use std::path::PathBuf;

use crate::identity::keystore::{KeyStore, KeystoreEntry, KeystoreError};

pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    /// Create a `FileKeyStore` operating on `path`.
    ///
    /// The directory containing `path` must exist before calling `set`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl KeyStore for FileKeyStore {
    /// Read the entry from disk.
    ///
    /// Returns:
    /// - `Ok(Some(entry))` — legacy-shape file with `secret` key present.
    /// - `Ok(None)` — file absent, or file present but `secret` key missing
    ///   (stub shape). Callers should fall back to a different backend.
    /// - `Err(KeystoreError::Io(_))` — unexpected I/O error.
    /// - `Err(KeystoreError::Serde(_))` — file exists but is not valid JSON at
    ///   all (distinct from a stub-shape file).
    fn get(&self) -> Result<Option<KeystoreEntry>, KeystoreError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(KeystoreError::Io(e)),
        };

        // Parse as a generic JSON object first to detect stub shape (no "secret" key).
        let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
        if raw.get("secret").is_none() {
            // Stub shape — secret lives in the OS keychain, not here.
            return Ok(None);
        }

        // Full legacy shape — deserialize into KeystoreEntry.
        let entry: KeystoreEntry = serde_json::from_value(raw)?;
        Ok(Some(entry))
    }

    /// Write the entry to disk atomically.
    ///
    /// Uses `NamedTempFile::new_in(parent)` + `.persist()` so a crash during
    /// the write never leaves the file in a partially-written state.  On Unix,
    /// the temp file inherits `NamedTempFile`'s default mode 0600, which is
    /// then preserved by the rename.
    fn set(&self, entry: &KeystoreEntry) -> Result<(), KeystoreError> {
        use tempfile::NamedTempFile;

        let parent = self.path.parent().ok_or_else(|| {
            KeystoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "identity path has no parent directory",
            ))
        })?;

        // Create a temp file in the same directory so the rename is atomic.
        let mut tmp = NamedTempFile::new_in(parent)?;

        // On Unix, enforce mode 0600 before writing.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(tmp.path(), perms)?;
        }

        // Write the JSON.  `serde_json::to_writer` is compact (no whitespace).
        serde_json::to_writer(&mut tmp, entry)?;

        // Atomically rename to the final path.
        tmp.persist(&self.path).map_err(|e| e.error)?;

        Ok(())
    }

    /// Delete the file if it exists.  No error if already absent.
    fn remove(&self) -> Result<(), KeystoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(KeystoreError::Io(e)),
        }
    }

    /// Returns `true` if the parent directory exists and is writable.
    fn available(&self) -> Result<bool, KeystoreError> {
        let parent = match self.path.parent() {
            Some(p) => p,
            None => return Ok(false),
        };
        // Probe write access by attempting a temp file creation.
        match tempfile::NamedTempFile::new_in(parent) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn name(&self) -> &'static str {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_entry(n: u8) -> KeystoreEntry {
        let mut secret = [0u8; 64];
        secret[0] = n;
        KeystoreEntry {
            secret,
            pubkey_base58: format!("pubkey_{n}"),
        }
    }

    fn store_in(dir: &TempDir) -> FileKeyStore {
        FileKeyStore::new(dir.path().join("identity.json"))
    }

    #[test]
    fn write_then_read_yields_same_entry() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let entry = make_entry(99);
        store.set(&entry).unwrap();
        let got = store.get().unwrap().expect("entry should be readable");
        assert_eq!(got.secret, entry.secret);
        assert_eq!(got.pubkey_base58, entry.pubkey_base58);
    }

    #[cfg(unix)]
    #[test]
    fn mode_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set(&make_entry(1)).unwrap();
        let meta = std::fs::metadata(dir.path().join("identity.json")).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "file mode must be 0600, got {mode:o}");
    }

    #[test]
    fn remove_deletes_file() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set(&make_entry(2)).unwrap();
        store.remove().unwrap();
        assert!(!dir.path().join("identity.json").exists());
    }

    #[test]
    fn remove_on_missing_file_is_ok() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        // Remove without a prior set — must succeed.
        store.remove().unwrap();
    }

    #[test]
    fn get_on_missing_returns_ok_none() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let got = store.get().unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn get_on_stub_shape_returns_ok_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.json");
        // Write a stub (no "secret" key) directly.
        let stub = r#"{"pubkey_base58":"abc123","keychain_ref":"xyz.mnemonik.identity/default","created_at":"2026-01-01T00:00:00Z"}"#;
        std::fs::write(&path, stub).unwrap();

        let store = FileKeyStore::new(path);
        let got = store.get().unwrap();
        assert!(
            got.is_none(),
            "stub-shaped file must return Ok(None), not an entry"
        );
    }

    #[test]
    fn available_returns_true_for_writable_dir() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        assert!(store.available().unwrap());
    }
}

//! Bootstrap algorithm for the Mnemonic Ed25519 identity.
//!
//! Implements the five-path algorithm from tech-spec.md §Architecture
//! "Bootstrap call path (Rust mcp-server)":
//!
//! 1. File absent  → generate keypair, write to OS keychain + stub file.
//! 2. File is stub (`keychain_ref` present) → read secret from OS keychain.
//! 3. File is legacy (`secret` present), OS available → migrate to keychain.
//! 4. File is legacy, OS unavailable → keep on file (file-fallback path).
//! 5. Partial-failure rollback: keychain set OK, stub write fails → remove
//!    keychain entry then propagate original error.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use chrono::Utc;
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use tempfile::NamedTempFile;

use crate::identity::keystore::{KeyStore, KeystoreEntry, KeystoreError};
use crate::identity::{Identity, IdentityStorage};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Wires up default OS + file stores and calls [`ensure_with_stores`].
///
/// Production entry-point. Tests call `ensure_with_stores` directly with
/// injected `MemoryKeyStore` backends.
pub fn ensure() -> anyhow::Result<Identity> {
    ensure_with_stores(default_stores()?)
}

/// Test seam: all I/O goes through `stores`, no hard-coded paths.
pub fn ensure_with_stores(stores: KeyStores) -> anyhow::Result<Identity> {
    let KeyStores {
        os,
        file,
        identity_path,
        readme_path,
    } = stores;

    if identity_path.exists() {
        // --- File present: determine shape ---
        let raw_bytes = std::fs::read(&identity_path)
            .with_context(|| format!("reading {}", identity_path.display()))?;
        let json: Value = serde_json::from_slice(&raw_bytes)
            .with_context(|| format!("parsing {}", identity_path.display()))?;

        if json.get("secret").is_some() {
            // LEGACY path: file carries the raw secret bytes.
            handle_legacy(
                json,
                os.as_deref(),
                file.as_ref(),
                &identity_path,
                &readme_path,
            )
        } else if json.get("keychain_ref").is_some() {
            // STUB path: secret lives in the OS keychain.
            handle_stub(json, os.as_deref(), &identity_path)
        } else {
            bail!(
                "{} is neither a legacy nor a stub identity file (missing both \
                 'secret' and 'keychain_ref' keys); delete it and re-run",
                identity_path.display()
            )
        }
    } else {
        // --- File absent: create from scratch ---
        handle_create(os.as_deref(), file.as_ref(), &identity_path, &readme_path)
    }
}

// ---------------------------------------------------------------------------
// KeyStores
// ---------------------------------------------------------------------------

/// All I/O abstractions needed by [`ensure_with_stores`].
pub struct KeyStores {
    /// OS keychain backend. `None` means skip the OS path entirely (tests may
    /// pass `None` to exercise pure file-fallback without an `unavailable()`
    /// store).
    pub os: Option<Box<dyn KeyStore>>,
    /// File-backed key store (used as fallback and for stub file writes).
    pub file: Box<dyn KeyStore>,
    /// Path for `~/.mnemonic/identity.json` (or equivalent in tests).
    pub identity_path: PathBuf,
    /// Path for `~/.mnemonic/README.txt` (or equivalent in tests).
    pub readme_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Default store wiring
// ---------------------------------------------------------------------------

fn default_stores() -> anyhow::Result<KeyStores> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory; set $HOME"))?;
    let mnemonic_dir = home.join(".mnemonic");

    std::fs::create_dir_all(&mnemonic_dir)
        .with_context(|| format!("creating {}", mnemonic_dir.display()))?;

    let identity_path = mnemonic_dir.join("identity.json");
    let readme_path = mnemonic_dir.join("README.txt");

    use crate::identity::keystore_file::FileKeyStore;
    use crate::identity::keystore_os::OsKeyStore;

    Ok(KeyStores {
        os: Some(Box::new(OsKeyStore::new())),
        file: Box::new(FileKeyStore::new(identity_path.clone())),
        identity_path,
        readme_path,
    })
}

// ---------------------------------------------------------------------------
// Path handlers
// ---------------------------------------------------------------------------

/// Path 1: identity.json absent — generate a new keypair.
fn handle_create(
    os: Option<&dyn KeyStore>,
    file: &dyn KeyStore,
    identity_path: &Path,
    readme_path: &Path,
) -> anyhow::Result<Identity> {
    let keypair = Keypair::new();
    let pubkey_base58 = keypair.pubkey().to_string();
    let secret = keypair.to_bytes();
    let entry = KeystoreEntry {
        secret,
        pubkey_base58: pubkey_base58.clone(),
    };
    let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let os_available = os.map(|s| s.available().unwrap_or(false)).unwrap_or(false);

    if os_available {
        let os_store = os.expect("os_available true implies Some");

        // Arm rollback guard before touching keychain.
        let mut guard = KeychainRollback::new(os_store);
        os_store
            .set(&entry)
            .context("writing new identity to OS keychain")?;
        guard.arm();

        // Write stub file — on failure the guard's Drop rolls back the keychain.
        let stub = build_stub_json(&pubkey_base58, &created_at);
        write_stub_file(identity_path, &stub)?;
        guard.disarm();

        write_readme_once(readme_path)?;
        maybe_log_line(&format!(
            "mnemonic: identity created did:sol:{pubkey_base58} stored in OS keychain"
        ));

        Ok(Identity {
            keypair,
            pubkey_base58,
            created_at,
            storage: IdentityStorage::OsKeychain,
        })
    } else {
        // File-fallback path.
        let reason = describe_unavailability(os);
        file.set(&entry)
            .context("writing new identity to file store")?;

        write_readme_once(readme_path)?;
        // Path is rendered as the literal "~/.mnemonic/identity.json" string
        // to match the Node CLI byte-for-byte (Decision 7). The real path is
        // derived from $HOME via `dirs::home_dir()` in `default_stores()`, so
        // 99% of the time this is the actual on-disk location; tests use
        // tempdir paths but don't assert on the message.
        let _ = identity_path; // suppress unused-binding lint without changing the signature
        maybe_log_line(&format!(
            "mnemonic: identity created did:sol:{pubkey_base58} stored in ~/.mnemonic/identity.json (OS keychain unavailable: {reason})"
        ));

        Ok(Identity {
            keypair,
            pubkey_base58,
            created_at,
            storage: IdentityStorage::File,
        })
    }
}

/// Path 2 (stub): read pubkey from file, secret from OS keychain.
fn handle_stub(
    json: Value,
    os: Option<&dyn KeyStore>,
    identity_path: &Path,
) -> anyhow::Result<Identity> {
    let pubkey_base58 = json
        .get("pubkey_base58")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "stub file {} missing 'pubkey_base58'",
                identity_path.display()
            )
        })?
        .to_owned();

    let created_at = json
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();

    let os_store = os.ok_or_else(|| {
        anyhow::anyhow!(
            "identity file {} is a stub pointing at the OS keychain, but no OS \
             keychain backend is available; run `mnemonic identity pull-from-webapp` \
             to restore the secret",
            identity_path.display()
        )
    })?;

    let entry = os_store
        .get()
        .context("reading identity from OS keychain")?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "identity file {} references an OS keychain entry that no longer \
                 exists; run `mnemonic identity pull-from-webapp` to restore it, \
                 or delete {} and let Mnemonic create a new identity",
                identity_path.display(),
                identity_path.display()
            )
        })?;

    let keypair = Keypair::try_from(&entry.secret[..])
        .map_err(|e| anyhow::anyhow!("invalid keypair bytes in OS keychain: {e}"))?;

    Ok(Identity {
        keypair,
        pubkey_base58,
        created_at,
        storage: IdentityStorage::OsKeychain,
    })
}

/// Paths 3 + 4: file carries legacy secret.
fn handle_legacy(
    json: Value,
    os: Option<&dyn KeyStore>,
    file: &dyn KeyStore,
    identity_path: &Path,
    readme_path: &Path,
) -> anyhow::Result<Identity> {
    // Deserialise legacy entry from the already-parsed JSON.
    let entry: KeystoreEntry = serde_json::from_value(json)
        .with_context(|| format!("parsing legacy identity from {}", identity_path.display()))?;

    let pubkey_base58 = entry.pubkey_base58.clone();
    let keypair = Keypair::try_from(&entry.secret[..])
        .map_err(|e| anyhow::anyhow!("invalid keypair bytes in legacy file: {e}"))?;
    let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let os_available = os.map(|s| s.available().unwrap_or(false)).unwrap_or(false);

    if os_available {
        // Path 3: migrate legacy file → OS keychain + stub.
        let os_store = os.expect("os_available true implies Some");

        let mut guard = KeychainRollback::new(os_store);
        os_store
            .set(&entry)
            .context("migrating identity to OS keychain")?;
        guard.arm();

        // Rewrite file as stub.
        let stub = build_stub_json(&pubkey_base58, &created_at);
        write_stub_file(identity_path, &stub)?;
        guard.disarm();

        // README: idempotent — won't overwrite if exists.
        write_readme_once(readme_path)?;
        maybe_log_line("mnemonic: legacy identity migrated to OS keychain");

        Ok(Identity {
            keypair,
            pubkey_base58,
            created_at,
            storage: IdentityStorage::OsKeychain,
        })
    } else {
        // Path 4: OS unavailable — keep legacy on file, return as-is.
        // Ensure the README exists (may be absent on very old installations).
        write_readme_once(readme_path)?;

        // Re-write the same entry via file store to normalise mode 0600 if needed.
        file.set(&entry)
            .context("refreshing legacy file identity")?;

        Ok(Identity {
            keypair,
            pubkey_base58,
            created_at: created_at.clone(),
            storage: IdentityStorage::File,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the stub JSON string (no `secret` key).
fn build_stub_json(pubkey_base58: &str, created_at: &str) -> String {
    // Use serde_json::json! for correctness; field order is human-readable only
    // (the stub is not byte-compared across languages).
    serde_json::json!({
        "pubkey_base58": pubkey_base58,
        "did_sol": format!("did:sol:{pubkey_base58}"),
        "keychain_ref": "xyz.mnemonik.identity/default",
        "created_at": created_at,
    })
    .to_string()
}

/// Atomic tempfile-based write of a stub JSON string, mode 0600 on Unix.
fn write_stub_file(identity_path: &Path, content: &str) -> anyhow::Result<()> {
    let parent = identity_path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "identity path {} has no parent directory",
            identity_path.display()
        )
    })?;

    let mut tmp = NamedTempFile::new_in(parent).context("creating temp file for stub")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(tmp.path(), perms)
            .context("setting mode 0600 on stub temp file")?;
    }

    tmp.write_all(content.as_bytes())
        .context("writing stub JSON to temp file")?;
    tmp.as_file()
        .sync_all()
        .context("syncing stub temp file to disk")?;
    tmp.persist(identity_path)
        .map_err(|e| e.error)
        .context("persisting stub file")?;

    Ok(())
}

/// Write README.txt exactly once; silently skip if it already exists.
fn write_readme_once(readme_path: &Path) -> anyhow::Result<()> {
    let result = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(readme_path);

    match result {
        Ok(mut f) => {
            f.write_all(README_CONTENT.as_bytes())
                .context("writing README.txt")?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).context("opening README.txt for creation"),
    }
    Ok(())
}

/// Short human-readable reason the OS keychain is unavailable.
fn describe_unavailability(os: Option<&dyn KeyStore>) -> String {
    match os {
        None => "no OS keychain backend configured".to_string(),
        Some(s) => match s.available() {
            Ok(true) => "unknown".to_string(),
            Ok(false) => "platform keychain unavailable".to_string(),
            Err(KeystoreError::PlatformUnavailable { reason }) => reason,
            Err(e) => e.to_string(),
        },
    }
}

/// Emit a single `mnemonic:`-prefixed line to stderr unless `MNEMONIC_QUIET=1`
/// is set. Bypasses the `tracing` subscriber for these three Decision-7
/// creation/migration messages so wording matches `packages/cli/src/identity/
/// ensure.ts` byte-for-byte — operators / QA / docs see one canonical line per
/// event regardless of which surface (Rust mcp-server vs Node CLI) bootstrapped
/// the identity.
fn maybe_log_line(line: &str) {
    if std::env::var("MNEMONIC_QUIET").is_ok() {
        return;
    }
    eprintln!("{line}");
}

// ---------------------------------------------------------------------------
// Drop-guard for keychain rollback (Decision 6)
// ---------------------------------------------------------------------------

struct KeychainRollback<'a> {
    os: &'a dyn KeyStore,
    armed: bool,
}

impl<'a> KeychainRollback<'a> {
    fn new(os: &'a dyn KeyStore) -> Self {
        Self { os, armed: false }
    }

    fn arm(&mut self) {
        self.armed = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for KeychainRollback<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(e) = self.os.remove() {
                tracing::warn!(
                    target: "mnemonic::identity",
                    "rollback: failed to remove keychain entry after file-write failure: {e}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// README content
// ---------------------------------------------------------------------------

const README_CONTENT: &str = "\
Mnemonic Identity Directory
============================

This directory holds your Mnemonic Ed25519 identity.

  identity.json  — public key (stub) or keypair (legacy fallback)
  README.txt     — this file

Your private key is stored in the OS keychain (macOS Keychain, Linux Secret
Service, or Windows Credential Manager). The identity.json file only holds
the public key and a reference to the keychain entry.

For help: https://mnemonik.xyz/docs/identity
";

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keystore_memory::MemoryKeyStore;
    use tempfile::TempDir;

    // Helper: build a KeyStores with MemoryKeyStore for both OS and file
    // backends, and real tempdir paths.
    fn mem_stores(
        dir: &TempDir,
    ) -> (
        KeyStores,
        std::sync::Arc<MemoryKeyStore>,
        std::sync::Arc<MemoryKeyStore>,
    ) {
        let os_store = std::sync::Arc::new(MemoryKeyStore::new());
        let file_store = std::sync::Arc::new(MemoryKeyStore::new());
        let stores = KeyStores {
            os: Some(Box::new(ArcMemoryKeyStore(os_store.clone()))),
            file: Box::new(ArcMemoryKeyStore(file_store.clone())),
            identity_path: dir.path().join("identity.json"),
            readme_path: dir.path().join("README.txt"),
        };
        (stores, os_store, file_store)
    }

    // ---------------------------------------------------------------------------
    // ArcMemoryKeyStore shim — lets tests share Arc<MemoryKeyStore> with ensure()
    // while still passing Box<dyn KeyStore>. Delegates everything to the inner Arc.
    // ---------------------------------------------------------------------------
    struct ArcMemoryKeyStore(std::sync::Arc<MemoryKeyStore>);

    impl KeyStore for ArcMemoryKeyStore {
        fn get(&self) -> Result<Option<KeystoreEntry>, KeystoreError> {
            self.0.get()
        }
        fn set(&self, entry: &KeystoreEntry) -> Result<(), KeystoreError> {
            self.0.set(entry)
        }
        fn remove(&self) -> Result<(), KeystoreError> {
            self.0.remove()
        }
        fn available(&self) -> Result<bool, KeystoreError> {
            self.0.available()
        }
        fn name(&self) -> &'static str {
            self.0.name()
        }
    }

    // ---------------------------------------------------------------------------
    // Test: create from scratch (OS available)
    // ---------------------------------------------------------------------------
    #[test]
    fn ensure_creates_when_absent() {
        let dir = TempDir::new().unwrap();
        let (stores, os_arc, _file_arc) = mem_stores(&dir);

        let id = ensure_with_stores(stores).expect("ensure should succeed");

        // Keychain entry was set.
        let kc_entry = os_arc.get().unwrap().expect("keychain entry must exist");
        assert_eq!(kc_entry.pubkey_base58, id.pubkey_base58);

        // Stub file was written.
        let stub_path = dir.path().join("identity.json");
        assert!(stub_path.exists(), "stub file must exist");
        let stub_json: Value = serde_json::from_slice(&std::fs::read(&stub_path).unwrap()).unwrap();
        assert!(
            stub_json.get("secret").is_none(),
            "stub must not contain secret"
        );
        assert!(
            stub_json.get("keychain_ref").is_some(),
            "stub must have keychain_ref"
        );
        assert_eq!(
            stub_json["pubkey_base58"].as_str().unwrap(),
            id.pubkey_base58
        );

        // README was created.
        assert!(
            dir.path().join("README.txt").exists(),
            "README.txt must exist"
        );

        // Identity storage is OsKeychain.
        assert!(matches!(id.storage, IdentityStorage::OsKeychain));
    }

    // ---------------------------------------------------------------------------
    // Test: read existing stub + keychain
    // ---------------------------------------------------------------------------
    #[test]
    fn ensure_reads_existing_stub() {
        let dir = TempDir::new().unwrap();

        // Pre-seed a real keypair.
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey().to_string();
        let secret = keypair.to_bytes();
        let created_at = "2026-05-21T10:00:00.000Z";

        // Write stub file.
        let stub_path = dir.path().join("identity.json");
        let stub = serde_json::json!({
            "pubkey_base58": pubkey,
            "did_sol": format!("did:sol:{pubkey}"),
            "keychain_ref": "xyz.mnemonik.identity/default",
            "created_at": created_at,
        })
        .to_string();
        std::fs::write(&stub_path, &stub).unwrap();

        // Pre-seed OS keychain with matching entry.
        let os_arc = std::sync::Arc::new(MemoryKeyStore::new());
        let entry = KeystoreEntry {
            secret,
            pubkey_base58: pubkey.clone(),
        };
        os_arc.set(&entry).unwrap();

        // Write a placeholder README.
        let readme_path = dir.path().join("README.txt");
        std::fs::write(&readme_path, "original").unwrap();
        let mtime_before = std::fs::metadata(&readme_path).unwrap().modified().unwrap();

        let stores = KeyStores {
            os: Some(Box::new(ArcMemoryKeyStore(os_arc.clone()))),
            file: Box::new(MemoryKeyStore::new()),
            identity_path: stub_path,
            readme_path: readme_path.clone(),
        };

        let id = ensure_with_stores(stores).expect("ensure should succeed");

        assert_eq!(id.pubkey_base58, pubkey);
        assert!(matches!(id.storage, IdentityStorage::OsKeychain));

        // README must be untouched (create_new skips existing).
        let content = std::fs::read_to_string(&readme_path).unwrap();
        assert_eq!(content, "original", "README must not be overwritten");
        let mtime_after = std::fs::metadata(&readme_path).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "README mtime must not change");
    }

    // ---------------------------------------------------------------------------
    // Test: migrate legacy to OS keychain
    // ---------------------------------------------------------------------------
    #[test]
    fn ensure_migrates_legacy_to_stub() {
        let dir = TempDir::new().unwrap();

        // Generate a real keypair to use as legacy.
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey().to_string();
        let secret = keypair.to_bytes();

        // Write legacy file.
        let identity_path = dir.path().join("identity.json");
        let legacy_entry = KeystoreEntry {
            secret,
            pubkey_base58: pubkey.clone(),
        };
        let legacy_json = serde_json::to_string(&legacy_entry).unwrap();
        std::fs::write(&identity_path, &legacy_json).unwrap();

        // Empty OS keychain.
        let os_arc = std::sync::Arc::new(MemoryKeyStore::new());

        let stores = KeyStores {
            os: Some(Box::new(ArcMemoryKeyStore(os_arc.clone()))),
            file: Box::new(MemoryKeyStore::new()),
            identity_path: identity_path.clone(),
            readme_path: dir.path().join("README.txt"),
        };

        let id = ensure_with_stores(stores).expect("ensure should succeed");

        // Same pubkey throughout migration.
        assert_eq!(id.pubkey_base58, pubkey);
        assert!(matches!(id.storage, IdentityStorage::OsKeychain));

        // OS keychain now has the entry.
        let kc_entry = os_arc
            .get()
            .unwrap()
            .expect("keychain entry must exist after migration");
        assert_eq!(kc_entry.pubkey_base58, pubkey);

        // identity.json is now a stub (no secret).
        let new_content = std::fs::read_to_string(&identity_path).unwrap();
        let new_json: Value = serde_json::from_str(&new_content).unwrap();
        assert!(
            new_json.get("secret").is_none(),
            "migrated file must not contain secret"
        );
        assert!(
            new_json.get("keychain_ref").is_some(),
            "migrated file must have keychain_ref"
        );
        assert_eq!(new_json["pubkey_base58"].as_str().unwrap(), pubkey);
    }

    // ---------------------------------------------------------------------------
    // Test: fall back to file when OS keychain is unavailable
    // ---------------------------------------------------------------------------
    #[test]
    fn ensure_falls_back_to_file_when_keychain_unavailable() {
        let dir = TempDir::new().unwrap();

        // Use real FileKeyStore so we can inspect the written file including mode.
        use crate::identity::keystore_file::FileKeyStore;
        let identity_path = dir.path().join("identity.json");
        let readme_path = dir.path().join("README.txt");

        let os_unavail = MemoryKeyStore::unavailable();

        let stores = KeyStores {
            os: Some(Box::new(os_unavail)),
            file: Box::new(FileKeyStore::new(identity_path.clone())),
            identity_path: identity_path.clone(),
            readme_path,
        };

        let id = ensure_with_stores(stores).expect("ensure should succeed");

        assert!(matches!(id.storage, IdentityStorage::File));

        // File should be in legacy format (has "secret" key).
        let written: Value =
            serde_json::from_slice(&std::fs::read(&identity_path).unwrap()).unwrap();
        assert!(
            written.get("secret").is_some(),
            "file-fallback must write legacy format with secret"
        );
        assert_eq!(written["pubkey_base58"].as_str().unwrap(), id.pubkey_base58);

        // Mode 0600 on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&identity_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "file mode must be 0600");
        }
    }

    // ---------------------------------------------------------------------------
    // Test: rollback on partial failure (keychain OK, stub write fails)
    // ---------------------------------------------------------------------------
    #[test]
    fn ensure_rolls_back_on_partial_failure() {
        let dir = TempDir::new().unwrap();

        let os_arc = std::sync::Arc::new(MemoryKeyStore::new());

        // Use a read-only identity path to force the stub-file write to fail.
        // We create a subdirectory and immediately remove write permissions on it.
        let ro_dir = dir.path().join("readonly");
        std::fs::create_dir(&ro_dir).unwrap();

        let identity_path = ro_dir.join("identity.json");
        let readme_path = dir.path().join("README.txt");

        // Make the directory read-only (Unix only; on Windows this test is vacuous).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }

        let stores = KeyStores {
            os: Some(Box::new(ArcMemoryKeyStore(os_arc.clone()))),
            file: Box::new(MemoryKeyStore::new()),
            identity_path,
            readme_path,
        };

        // On Unix the stub write will fail (read-only dir).
        // On non-Unix we skip the assertion but still call ensure to verify
        // it doesn't panic.
        #[cfg(unix)]
        {
            let result = ensure_with_stores(stores);
            assert!(result.is_err(), "ensure must fail when stub write fails");

            // Keychain entry must have been rolled back.
            let after = os_arc.get().unwrap();
            assert!(
                after.is_none(),
                "keychain entry must be rolled back after partial failure"
            );

            // Restore permissions so TempDir cleanup succeeds.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        #[cfg(not(unix))]
        {
            // On non-Unix, just verify no panic.
            let _ = ensure_with_stores(stores);
        }
    }

    // ---------------------------------------------------------------------------
    // Test: stub points at missing keychain entry → actionable error
    // ---------------------------------------------------------------------------
    #[test]
    fn ensure_stub_missing_keychain_entry_returns_err() {
        let dir = TempDir::new().unwrap();

        let keypair = Keypair::new();
        let pubkey = keypair.pubkey().to_string();

        let stub_path = dir.path().join("identity.json");
        let stub = serde_json::json!({
            "pubkey_base58": pubkey,
            "did_sol": format!("did:sol:{pubkey}"),
            "keychain_ref": "xyz.mnemonik.identity/default",
            "created_at": "2026-05-21T10:00:00.000Z",
        })
        .to_string();
        std::fs::write(&stub_path, &stub).unwrap();

        // OS keychain is empty — no entry.
        let stores = KeyStores {
            os: Some(Box::new(MemoryKeyStore::new())),
            file: Box::new(MemoryKeyStore::new()),
            identity_path: stub_path,
            readme_path: dir.path().join("README.txt"),
        };

        let result = ensure_with_stores(stores);
        assert!(result.is_err(), "must fail when keychain entry is missing");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("pull-from-webapp") || msg.contains("keychain entry"),
            "error must be actionable: {msg}"
        );
    }
}

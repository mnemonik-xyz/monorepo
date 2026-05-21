/// OS keychain-backed `KeyStore` implementation.
///
/// Wraps `keyring::Entry::new("xyz.mnemonik.identity", "default")` per
/// Decision 1.  Maps to macOS Keychain, Linux Secret Service (via D-Bus), and
/// Windows Credential Manager.
///
/// The value stored in the keychain is the compact JSON string:
///   `{"secret":[...64 bytes...],"pubkey_base58":"..."}`
/// — byte-identical to the legacy file-fallback format per Decision 13.
use keyring::Entry;

use crate::identity::keystore::{KeyStore, KeystoreEntry, KeystoreError};

/// Service and account identifiers for the keychain entry (Decision 1).
const SERVICE: &str = "xyz.mnemonik.identity";
const ACCOUNT: &str = "default";

pub struct OsKeyStore;

impl OsKeyStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(&self) -> Result<Entry, KeystoreError> {
        Entry::new(SERVICE, ACCOUNT).map_err(|e| KeystoreError::Backend(anyhow::anyhow!("{e}")))
    }
}

impl Default for OsKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore for OsKeyStore {
    /// Retrieve the entry from the OS keychain.
    ///
    /// Returns `Ok(None)` when no entry has been stored yet
    /// (`keyring::Error::NoEntry`).
    fn get(&self) -> Result<Option<KeystoreEntry>, KeystoreError> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(json) => {
                let ke: KeystoreEntry = serde_json::from_str(&json)?;
                Ok(Some(ke))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoStorageAccess(_)) => Err(KeystoreError::PlatformUnavailable {
                reason: "no storage access".to_string(),
            }),
            Err(e) => Err(KeystoreError::Backend(anyhow::anyhow!("{e}"))),
        }
    }

    /// Persist the entry in the OS keychain.
    ///
    /// Serializes to compact JSON with key order `secret` → `pubkey_base58`
    /// (Decision 13) and stores the UTF-8 string.
    fn set(&self, entry: &KeystoreEntry) -> Result<(), KeystoreError> {
        let json = serde_json::to_string(entry)?;
        let ke = self.entry()?;
        ke.set_password(&json)
            .map_err(|e| KeystoreError::Backend(anyhow::anyhow!("{e}")))?;
        Ok(())
    }

    /// Delete the keychain entry.  Idempotent — `NoEntry` is treated as success.
    fn remove(&self) -> Result<(), KeystoreError> {
        let entry = self.entry()?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeystoreError::Backend(anyhow::anyhow!("{e}"))),
        }
    }

    /// Probe the keychain to determine if this backend is usable.
    ///
    /// Returns `false` on `PlatformFailure` / `NoStorageAccess` — those
    /// indicate the keychain subsystem is not available (headless Docker, SSH
    /// without keyring forwarding, etc.).  Returns `true` on `NoEntry`
    /// (backend works, just no entry yet) and on success.
    fn available(&self) -> Result<bool, KeystoreError> {
        match Entry::new(SERVICE, ACCOUNT) {
            Err(_) => Ok(false),
            Ok(e) => match e.get_password() {
                Ok(_) | Err(keyring::Error::NoEntry) => Ok(true),
                Err(keyring::Error::NoStorageAccess(_)) => Ok(false),
                // Any other error (e.g. PlatformFailure on D-Bus timeout) →
                // treat as unavailable so callers fall back to file.
                Err(_) => Ok(false),
            },
        }
    }

    fn name(&self) -> &'static str {
        "os"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real keychain round-trip test.
    ///
    /// Skipped by default so `cargo test` does not touch the OS keychain on CI
    /// without explicit opt-in.  Wave 3 Task 9 exercises this in a
    /// Linux + gnome-keyring container via:
    ///
    ///   cargo test -p mnemonic-core -- --ignored identity::keystore_os
    ///
    #[test]
    #[ignore]
    fn roundtrip_real_keychain() {
        let store = OsKeyStore::new();

        // Clean slate.
        let _ = store.remove();

        // set → get
        let mut secret = [0u8; 64];
        for (i, b) in secret.iter_mut().enumerate() {
            *b = i as u8;
        }
        let entry = KeystoreEntry {
            secret,
            pubkey_base58: "RealKeychainTest".to_string(),
        };
        store.set(&entry).unwrap();

        let got = store.get().unwrap().expect("entry should exist after set");
        assert_eq!(got.secret, secret);
        assert_eq!(got.pubkey_base58, "RealKeychainTest");

        // remove → get == None
        store.remove().unwrap();
        let after_remove = store.get().unwrap();
        assert!(after_remove.is_none(), "entry should be None after remove");
    }
}

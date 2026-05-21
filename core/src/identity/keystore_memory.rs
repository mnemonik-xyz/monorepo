/// In-memory `KeyStore` implementation for use in tests only.
///
/// Backed by a `Mutex<Option<KeystoreEntry>>`. Thread-safe, zero I/O.
/// Not compiled into production builds.
use std::sync::Mutex;

use crate::identity::keystore::{KeyStore, KeystoreEntry, KeystoreError};

pub struct MemoryKeyStore {
    inner: Mutex<Option<StoredEntry>>,
    /// When `Some(false)` the store reports itself as unavailable, allowing
    /// tests to simulate a platform that has no keychain.
    force_unavailable: bool,
}

/// Internal owned copy of a `KeystoreEntry` (the trait deals in references on
/// write and owned values on read, so we need our own storage).
struct StoredEntry {
    secret: [u8; 64],
    pubkey_base58: String,
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            force_unavailable: false,
        }
    }

    /// Return a store that reports `available() == false`.
    pub fn unavailable() -> Self {
        Self {
            inner: Mutex::new(None),
            force_unavailable: true,
        }
    }
}

impl Default for MemoryKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore for MemoryKeyStore {
    fn get(&self) -> Result<Option<KeystoreEntry>, KeystoreError> {
        let guard = self.inner.lock().expect("MemoryKeyStore mutex poisoned");
        Ok(guard.as_ref().map(|e| KeystoreEntry {
            secret: e.secret,
            pubkey_base58: e.pubkey_base58.clone(),
        }))
    }

    fn set(&self, entry: &KeystoreEntry) -> Result<(), KeystoreError> {
        let mut guard = self.inner.lock().expect("MemoryKeyStore mutex poisoned");
        *guard = Some(StoredEntry {
            secret: entry.secret,
            pubkey_base58: entry.pubkey_base58.clone(),
        });
        Ok(())
    }

    fn remove(&self) -> Result<(), KeystoreError> {
        let mut guard = self.inner.lock().expect("MemoryKeyStore mutex poisoned");
        *guard = None;
        Ok(())
    }

    fn available(&self) -> Result<bool, KeystoreError> {
        if self.force_unavailable {
            return Ok(false);
        }
        Ok(true)
    }

    fn name(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(n: u8) -> KeystoreEntry {
        let mut secret = [0u8; 64];
        secret[0] = n;
        KeystoreEntry {
            secret,
            pubkey_base58: format!("pubkey_{n}"),
        }
    }

    #[test]
    fn roundtrip_set_get_equal() {
        let store = MemoryKeyStore::new();
        let entry = make_entry(42);
        store.set(&entry).unwrap();
        let got = store.get().unwrap().expect("entry should exist after set");
        assert_eq!(got.secret, entry.secret);
        assert_eq!(got.pubkey_base58, entry.pubkey_base58);
    }

    #[test]
    fn remove_clears_entry() {
        let store = MemoryKeyStore::new();
        store.set(&make_entry(1)).unwrap();
        store.remove().unwrap();
        let got = store.get().unwrap();
        assert!(got.is_none(), "entry should be None after remove");
    }

    #[test]
    fn double_remove_is_ok() {
        let store = MemoryKeyStore::new();
        store.set(&make_entry(2)).unwrap();
        store.remove().unwrap();
        // Second remove must not error.
        store.remove().unwrap();
        assert!(store.get().unwrap().is_none());
    }

    #[test]
    fn available_returns_true_by_default() {
        let store = MemoryKeyStore::new();
        assert!(store.available().unwrap());
    }

    #[test]
    fn unavailable_store_reports_false() {
        let store = MemoryKeyStore::unavailable();
        assert!(!store.available().unwrap());
    }

    #[test]
    fn trait_object_construction() {
        let store = MemoryKeyStore::new();
        store.set(&make_entry(7)).unwrap();
        let boxed: Box<dyn KeyStore> = Box::new(store);
        let got = boxed.get().unwrap();
        assert!(got.is_some());
        assert_eq!(boxed.name(), "memory");
    }
}

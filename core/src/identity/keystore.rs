use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

// ---------------------------------------------------------------------------
// KeystoreEntry
// ---------------------------------------------------------------------------

/// Ed25519 keypair entry stored in a `KeyStore`.
///
/// SECURITY: `secret` is NEVER printed. The `Debug` impl redacts it.
pub struct KeystoreEntry {
    /// Raw Ed25519 secret key bytes (seed + public, 64 bytes total).
    pub secret: [u8; 64],
    /// Base58-encoded Ed25519 public key.
    pub pubkey_base58: String,
}

impl fmt::Debug for KeystoreEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreEntry")
            .field("secret", &"[redacted; 64]")
            .field("pubkey_base58", &self.pubkey_base58)
            .finish()
    }
}

// Manual Serialize to guarantee field order: `secret` THEN `pubkey_base58`.
// `serde_json` does not guarantee struct field order across versions, so we
// implement it manually per Decision 13.
impl Serialize for KeystoreEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("KeystoreEntry", 2)?;
        // Field order is load-bearing: secret must come first.
        s.serialize_field("secret", &self.secret.as_slice())?;
        s.serialize_field("pubkey_base58", &self.pubkey_base58)?;
        s.end()
    }
}

// Manual Deserialize so we can recognise both the legacy shape
// `{"secret":[...],"pubkey_base58":"..."}` and return None-like behaviour to
// callers when `secret` is absent.  Callers get `Ok(None)` from `FileKeyStore`
// when the stub shape is detected; here we simply error on missing `secret` so
// that the calling code in `FileKeyStore::get` can handle the `Err` case and
// convert it to `Ok(None)`.
impl<'de> Deserialize<'de> for KeystoreEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntryVisitor;

        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = KeystoreEntry;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#"object with "secret" (array of 64 u8) and "pubkey_base58" (string)"#)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut secret: Option<Vec<u8>> = None;
                let mut pubkey_base58: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "secret" => {
                            secret = Some(map.next_value()?);
                        }
                        "pubkey_base58" => {
                            pubkey_base58 = Some(map.next_value()?);
                        }
                        _ => {
                            // Ignore unknown fields (e.g. stub-shape fields like keychain_ref).
                            let _ = map.next_value::<serde_json::Value>()?;
                        }
                    }
                }

                let bytes = secret.ok_or_else(|| de::Error::missing_field("secret"))?;
                let pubkey_base58 =
                    pubkey_base58.ok_or_else(|| de::Error::missing_field("pubkey_base58"))?;

                if bytes.len() != 64 {
                    return Err(de::Error::invalid_length(
                        bytes.len(),
                        &"exactly 64 bytes for Ed25519 secret",
                    ));
                }
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&bytes);
                Ok(KeystoreEntry {
                    secret: arr,
                    pubkey_base58,
                })
            }
        }

        deserializer.deserialize_map(EntryVisitor)
    }
}

// ---------------------------------------------------------------------------
// KeystoreError
// ---------------------------------------------------------------------------

/// Errors returned by `KeyStore` implementations.
///
/// SECURITY: no variant carries raw key material.
#[derive(thiserror::Error, Debug)]
pub enum KeystoreError {
    /// The OS keychain platform is unavailable (no D-Bus, no Keychain, etc.).
    #[error("keystore platform unavailable: {reason}")]
    PlatformUnavailable { reason: String },

    /// The keychain is locked and could not be unlocked.
    #[error("keystore is locked")]
    Locked,

    /// No entry exists in the store.
    #[error("keystore entry not found")]
    NotFound,

    /// An I/O error from the filesystem.
    #[error("keystore I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A (de)serialization error.
    #[error("keystore serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A backend-specific error (e.g. from the `keyring` crate).
    ///
    /// The error message is forwarded verbatim; ensure the `keyring` crate
    /// does not surface secret bytes in its error messages (it does not).
    #[error("keystore backend error: {0}")]
    Backend(#[source] anyhow::Error),
}

// ---------------------------------------------------------------------------
// KeyStore trait
// ---------------------------------------------------------------------------

/// Abstraction over secret-storage backends.
///
/// All methods are synchronous and `Send + Sync` so the trait object can be
/// shared across threads (the `OsKeyStore` wraps its `keyring::Entry` without
/// interior mutability; `FileKeyStore` is path-only).
pub trait KeyStore: Send + Sync {
    /// Retrieve the entry, or `Ok(None)` if no entry is stored yet.
    fn get(&self) -> Result<Option<KeystoreEntry>, KeystoreError>;

    /// Persist an entry, replacing any existing one.
    fn set(&self, entry: &KeystoreEntry) -> Result<(), KeystoreError>;

    /// Remove the entry if it exists; idempotent.
    fn remove(&self) -> Result<(), KeystoreError>;

    /// Return `true` if this backend is usable on the current machine.
    ///
    /// Allowed to be optimistic — a `false` here skips the backend entirely;
    /// a `true` may still fail on the first `get`/`set`.
    fn available(&self) -> Result<bool, KeystoreError>;

    /// Short identifier used in status output: `"os"`, `"file"`, or `"memory"`.
    fn name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden test: serialized bytes MUST be exactly
    /// `{"secret":[0,0,...,0],"pubkey_base58":"test"}` with no whitespace and
    /// `secret` before `pubkey_base58`.
    #[test]
    fn golden_json_bytes_are_stable() {
        let entry = KeystoreEntry {
            secret: [0u8; 64],
            pubkey_base58: "test".to_string(),
        };
        let json = serde_json::to_string(&entry).expect("serialization failed");

        // Build expected string: {"secret":[0,0,...,0],"pubkey_base58":"test"}
        let zeros: Vec<String> = (0..64).map(|_| "0".to_string()).collect();
        let expected = format!(
            r#"{{"secret":[{}],"pubkey_base58":"test"}}"#,
            zeros.join(",")
        );
        assert_eq!(json, expected, "golden JSON bytes deviated");
    }

    /// Verify that `KeystoreEntry` round-trips through JSON.
    #[test]
    fn json_roundtrip() {
        let mut secret = [0u8; 64];
        for (i, b) in secret.iter_mut().enumerate() {
            *b = i as u8;
        }
        let entry = KeystoreEntry {
            secret,
            pubkey_base58: "SomePubkey".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: KeystoreEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.secret, secret);
        assert_eq!(decoded.pubkey_base58, "SomePubkey");
    }

    /// Trait object construction sanity check.
    #[test]
    fn trait_object_is_object_safe() {
        // Importing MemoryKeyStore here directly would create a circular dep;
        // instead we just verify the trait is object-safe by checking that a
        // Box<dyn KeyStore> can be constructed (done in keystore_memory.rs
        // tests). This test just ensures the file compiles.
        let _size: usize = std::mem::size_of::<Box<dyn KeyStore>>();
        // If we get here, the trait is object-safe.
    }

    /// Verify that stub-shaped JSON (no `secret` key) fails to deserialize.
    #[test]
    fn stub_json_fails_deserialization() {
        let stub = r#"{"pubkey_base58":"abc","keychain_ref":"xyz.mnemonik.identity/default"}"#;
        let result: Result<KeystoreEntry, _> = serde_json::from_str(stub);
        assert!(
            result.is_err(),
            "stub JSON should fail to deserialize as KeystoreEntry"
        );
    }

    /// Cross-language golden byte-equality test (Task 10).
    ///
    /// The fixture is a deterministic `KeystoreEntry` built from a 32-byte
    /// seed `[0x42; 32]` using `Keypair::new_from_array`. The 64-byte `secret`
    /// field is `kp.to_bytes()` = seed_bytes || pubkey_bytes (Solana/ed25519_dalek
    /// layout); only the first 32 bytes are the seed, the rest is the derived
    /// public key. The `EXPECTED` string was generated once by
    /// `cargo run --example golden-keystore-gen` and must be identical to the
    /// constant in `packages/cli/test/identity/keystore.test.ts`.
    #[test]
    fn golden_json_bytes_cross_language_canonical() {
        use solana_sdk::signature::{Keypair, Signer};

        let kp = Keypair::new_from_array([0x42u8; 32]);
        let secret64 = kp.to_bytes();
        let pubkey = kp.pubkey().to_string();

        let entry = KeystoreEntry {
            secret: secret64,
            pubkey_base58: pubkey,
        };
        let actual = serde_json::to_string(&entry).unwrap();

        const EXPECTED: &str = r#"{"secret":[66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,66,33,82,248,209,155,121,29,36,69,50,66,225,95,46,171,108,183,207,250,123,106,94,211,0,151,150,14,6,152,129,219,18],"pubkey_base58":"3F5qRPtKg8GhGNnbd3qCj6nVJxWsGxq7pvH84okYLAqf"}"#;
        assert_eq!(actual, EXPECTED, "cross-language byte-equality must hold");
    }

    /// Verify `Debug` impl redacts the secret.
    #[test]
    fn debug_redacts_secret() {
        let entry = KeystoreEntry {
            secret: [0xABu8; 64],
            pubkey_base58: "visible".to_string(),
        };
        let debug = format!("{:?}", entry);
        assert!(
            !debug.contains("171"),
            "secret bytes must not appear in Debug output"
        );
        assert!(
            debug.contains("redacted"),
            "Debug output must say 'redacted'"
        );
        assert!(
            debug.contains("visible"),
            "pubkey must be visible in Debug output"
        );
    }
}

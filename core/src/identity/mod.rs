pub mod ensure;
pub mod keystore;
pub mod keystore_file;
#[cfg(test)]
pub mod keystore_memory;
pub mod keystore_os;
pub mod token_store;

pub use ensure::{ensure, ensure_with_stores, KeyStores};
pub use keystore::{KeyStore, KeystoreEntry, KeystoreError};
pub use keystore_file::FileKeyStore;
pub use keystore_os::OsKeyStore;
pub use token_store::{
    delete_token, delete_token_at, read_token, read_token_from, save_token, save_token_to,
    token_path, TokenJson, TokenStoreError,
};

use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// A resolved Mnemonic Ed25519 identity.
///
/// The `keypair` is eagerly loaded at bootstrap time. Task 4 may convert this
/// to lazy access via a `SecretAccessor` if the startup cost proves material;
/// for now eager caching is sufficient and simpler.
pub struct Identity {
    pub keypair: Keypair,
    /// Base58-encoded public key, cached for cheap reads.
    pub pubkey_base58: String,
    /// RFC3339 timestamp from the stub file, or "now" on first creation.
    pub created_at: String,
    /// Where the secret is persisted.
    pub storage: IdentityStorage,
}

impl std::fmt::Debug for Identity {
    /// Secret-redacted Debug — never prints `keypair` bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("keypair", &"[redacted; 64]")
            .field("pubkey_base58", &self.pubkey_base58)
            .field("created_at", &self.created_at)
            .field("storage", &self.storage)
            .finish()
    }
}

/// Where the identity secret is persisted.
#[derive(Debug)]
pub enum IdentityStorage {
    /// Secret lives in the OS keychain (macOS Keychain, Linux Secret Service,
    /// Windows Credential Manager). The `identity.json` file holds only the
    /// public key (stub shape).
    OsKeychain,
    /// Secret lives in `~/.mnemonic/identity.json` (legacy / file-fallback
    /// shape). Used when no OS keychain is available.
    File,
}

// ---------------------------------------------------------------------------
// Crypto helpers — unchanged; existing mcp/ call-sites kept working.
// ---------------------------------------------------------------------------

/// Base58-encoded public key.
pub fn pubkey_base58(kp: &Keypair) -> String {
    kp.pubkey().to_string()
}

/// did:sol:<base58_pubkey>
pub fn did_sol(kp: &Keypair) -> String {
    format!("did:sol:{}", kp.pubkey())
}

/// did:key:z<base58btc(multicodec_ed25519 + raw_pubkey)>
pub fn did_key(kp: &Keypair) -> String {
    let raw = kp.pubkey().to_bytes();
    // Ed25519 multicodec prefix: 0xed01
    let mut mc = vec![0xed, 0x01];
    mc.extend_from_slice(&raw);
    let encoded = bs58::encode(&mc).into_string();
    format!("did:key:z{encoded}")
}

/// did:key:z<...> derived from an already-base58-encoded Ed25519 public key.
///
/// Mirrors [`did_key`] but takes the public key as a base58 string (e.g. an
/// OAuth JWT `sub`) instead of a [`Keypair`]. Used by `whoami` on the
/// HTTP/JWT path, where the server holds only the authenticated caller's
/// public key — never their secret — so it cannot build a `Keypair` to feed
/// [`did_key`]. Returns `None` if `pubkey_base58` is not a valid 32-byte
/// Ed25519 public key.
pub fn did_key_from_base58(pubkey_base58: &str) -> Option<String> {
    let raw = bs58::decode(pubkey_base58).into_vec().ok()?;
    if raw.len() != 32 {
        return None;
    }
    // Ed25519 multicodec prefix: 0xed01
    let mut mc = vec![0xed, 0x01];
    mc.extend_from_slice(&raw);
    Some(format!("did:key:z{}", bs58::encode(&mc).into_string()))
}

/// Sign arbitrary bytes with Ed25519.
pub fn sign_bytes(kp: &Keypair, message: &[u8]) -> Vec<u8> {
    kp.sign_message(message).as_ref().to_vec()
}

/// Verify an Ed25519 signature.
pub fn verify_signature(pubkey: &Pubkey, message: &[u8], signature: &[u8]) -> bool {
    if signature.len() != 64 {
        return false;
    }
    let sig = solana_sdk::signature::Signature::from(<[u8; 64]>::try_from(signature).unwrap());
    sig.verify(pubkey.as_ref(), message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_did_sol() {
        let kp = Keypair::new();
        let d = did_sol(&kp);
        assert!(d.starts_with("did:sol:"));
        assert!(d.contains(&kp.pubkey().to_string()));
    }

    #[test]
    fn test_did_key() {
        let kp = Keypair::new();
        let d = did_key(&kp);
        assert!(d.starts_with("did:key:z"));
        // Deterministic
        assert_eq!(d, did_key(&kp));
    }

    #[test]
    fn test_did_key_from_base58_matches_keypair_path() {
        let kp = Keypair::new();
        // The base58-string path must produce byte-identical output to the
        // Keypair path for the same public key — whoami relies on this so the
        // HTTP/JWT response matches the stdio response shape.
        let from_str = did_key_from_base58(&pubkey_base58(&kp));
        assert_eq!(from_str.as_deref(), Some(did_key(&kp).as_str()));
    }

    #[test]
    fn test_did_key_from_base58_rejects_garbage() {
        assert_eq!(did_key_from_base58("not-base58!!!"), None);
        // Valid base58 but wrong length (not a 32-byte Ed25519 key).
        assert_eq!(did_key_from_base58("abc"), None);
    }

    #[test]
    fn test_sign_verify() {
        let kp = Keypair::new();
        let msg = b"hello mnemonic";
        let sig = sign_bytes(&kp, msg);
        assert_eq!(sig.len(), 64);
        assert!(verify_signature(&kp.pubkey(), msg, &sig));
        assert!(!verify_signature(&kp.pubkey(), b"wrong", &sig));
    }
}

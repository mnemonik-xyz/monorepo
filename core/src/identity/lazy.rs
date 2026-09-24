//! Deferred access to the identity secret.
//!
//! The public key lives in the `identity.json` stub, so everything that only
//! needs to *name* the identity (recall scoping, whoami, local writes) can run
//! without touching the OS keychain. The secret is read the first time an
//! operation actually has to sign (on-chain anchoring, prove_identity,
//! publish). This keeps a locked Secret Service / macOS Keychain from
//! prompting on every MCP server start.

use std::sync::{Mutex, OnceLock};

use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};

type SecretLoader = Box<dyn Fn() -> anyhow::Result<Keypair> + Send + Sync>;

/// An identity whose public key is known up front and whose secret is loaded
/// on first use, then cached for the life of the process.
pub struct LazyKeypair {
    pubkey: Pubkey,
    cell: OnceLock<Keypair>,
    loader: Option<SecretLoader>,
    load_lock: Mutex<()>,
}

impl LazyKeypair {
    /// Already-loaded keypair (explicit keypair file, file-fallback identity,
    /// tests).
    pub fn ready(keypair: Keypair) -> Self {
        let cell = OnceLock::new();
        let pubkey = keypair.pubkey();
        let _ = cell.set(keypair);
        Self {
            pubkey,
            cell,
            loader: None,
            load_lock: Mutex::new(()),
        }
    }

    /// Public key known now; secret fetched by `loader` on first
    /// [`keypair`](Self::keypair) call. A failed load is not cached, so a
    /// dismissed unlock prompt can be retried by the next signing request.
    pub fn deferred(
        pubkey: Pubkey,
        loader: impl Fn() -> anyhow::Result<Keypair> + Send + Sync + 'static,
    ) -> Self {
        Self {
            pubkey,
            cell: OnceLock::new(),
            loader: Some(Box::new(loader)),
            load_lock: Mutex::new(()),
        }
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    pub fn pubkey_base58(&self) -> String {
        self.pubkey.to_string()
    }

    pub fn did_sol(&self) -> String {
        super::did_sol_from_pubkey(&self.pubkey)
    }

    pub fn did_key(&self) -> String {
        super::did_key_from_pubkey(&self.pubkey)
    }

    /// True once the secret is in memory (no further keychain access needed).
    pub fn is_loaded(&self) -> bool {
        self.cell.get().is_some()
    }

    /// The signing keypair, reading the secret store on first call.
    pub fn keypair(&self) -> anyhow::Result<&Keypair> {
        if let Some(kp) = self.cell.get() {
            return Ok(kp);
        }
        // Serialize loads so concurrent signers produce one unlock prompt.
        let _guard = self
            .load_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("identity load lock poisoned"))?;
        if let Some(kp) = self.cell.get() {
            return Ok(kp);
        }
        let loader = self
            .loader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity secret unavailable"))?;
        let kp = loader()?;
        if kp.pubkey() != self.pubkey {
            anyhow::bail!(
                "identity secret does not match public key {} (got {})",
                self.pubkey,
                kp.pubkey()
            );
        }
        Ok(self.cell.get_or_init(|| kp))
    }
}

impl std::fmt::Debug for LazyKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyKeypair")
            .field("pubkey", &self.pubkey)
            .field("loaded", &self.is_loaded())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn deferred_does_not_load_until_asked() {
        let kp = Keypair::new();
        let bytes = kp.to_bytes();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let lazy = LazyKeypair::deferred(kp.pubkey(), move || {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(Keypair::try_from(&bytes[..])?)
        });
        assert_eq!(lazy.pubkey(), kp.pubkey());
        assert!(lazy.did_sol().starts_with("did:sol:"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!lazy.is_loaded());

        lazy.keypair().unwrap();
        lazy.keypair().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(lazy.is_loaded());
    }

    #[test]
    fn failed_load_is_retried() {
        let kp = Keypair::new();
        let bytes = kp.to_bytes();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let lazy = LazyKeypair::deferred(kp.pubkey(), move || {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                anyhow::bail!("user dismissed unlock prompt");
            }
            Ok(Keypair::try_from(&bytes[..])?)
        });
        assert!(lazy.keypair().is_err());
        assert!(lazy.keypair().is_ok());
    }

    #[test]
    fn mismatched_secret_is_rejected() {
        let lazy = LazyKeypair::deferred(Keypair::new().pubkey(), || Ok(Keypair::new()));
        assert!(lazy.keypair().is_err());
        assert!(!lazy.is_loaded());
    }
}

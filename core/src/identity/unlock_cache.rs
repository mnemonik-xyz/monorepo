//! Per-process cache for the X25519 decryption secret derived from the
//! Ed25519 identity key (sealed-memories Task 12).
//!
//! The OS keychain (macOS Keychain, Linux Secret Service) is read at most
//! **once** per process: after the first successful unlock the derived
//! X25519 secret is kept in memory until either the configurable TTL
//! expires or the process exits.  Default is process-lifetime (no TTL).
//!
//! # Design constraints
//!
//! - The SQLite mutex must NEVER be held across the `.await`-free keychain
//!   call — the cache lock is a plain `std::sync::Mutex` (never held across
//!   await), and the keychain loader closure is called while NOT holding any
//!   other application lock.
//! - `UnlockedX25519.secret` lives in a [`Zeroizing`] wrapper so the
//!   content-encryption key is wiped from memory on drop.
//! - A failed load is NOT cached: a dismissed OS prompt can be retried by
//!   the next caller.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

/// The X25519 secret derived from the agent's Ed25519 identity key.
pub struct UnlockedX25519 {
    /// The 32-byte X25519 secret scalar, zeroized on drop.
    pub secret: Zeroizing<[u8; 32]>,
    /// Wall-clock time this entry was populated (used for TTL eviction).
    loaded_at: Instant,
}

/// Process-local cache for the unlocked X25519 secret.
///
/// # Thread safety
///
/// The inner `Mutex` is a plain `std::sync::Mutex`. It is always taken for
/// a short synchronous section and NEVER across any `.await` point — the
/// keychain loader is a plain `Fn() -> anyhow::Result<[u8;32]>` with no
/// async component.
pub struct UnlockCache {
    inner: Mutex<Option<UnlockedX25519>>,
    /// `None` = process-lifetime (never evict).
    ttl: Option<Duration>,
}

impl UnlockCache {
    /// Construct a cache whose TTL is read from the `MNEMONIC_UNLOCK_TTL`
    /// environment variable (seconds).  If the variable is absent or zero the
    /// cache lives for the duration of the process.
    pub fn from_env() -> Self {
        let ttl = std::env::var("MNEMONIC_UNLOCK_TTL")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&n| n > 0)
            .map(Duration::from_secs);
        Self {
            inner: Mutex::new(None),
            ttl,
        }
    }

    /// Construct with an explicit TTL.  `None` means process lifetime.
    pub fn with_ttl(ttl: Option<Duration>) -> Self {
        Self {
            inner: Mutex::new(None),
            ttl,
        }
    }

    /// Return the cached X25519 secret, or unlock it via `keychain_fn` and
    /// cache the result.
    ///
    /// The `keychain_fn` closure MUST NOT acquire any application mutex —
    /// the cache lock is already held when it is called.
    ///
    /// # Errors
    ///
    /// Returns the error from `keychain_fn` on the first call.  A failed
    /// load is NOT cached; the next caller will retry the keychain.
    pub fn get_or_unlock<F>(&self, keychain_fn: F) -> anyhow::Result<Zeroizing<[u8; 32]>>
    where
        F: FnOnce() -> anyhow::Result<[u8; 32]>,
    {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("unlock_cache mutex poisoned"))?;

        // Check TTL expiry.
        if let Some(entry) = guard.as_ref() {
            if let Some(ttl) = self.ttl {
                if entry.loaded_at.elapsed() > ttl {
                    *guard = None;
                }
            }
        }

        if let Some(entry) = guard.as_ref() {
            // Cache hit — copy the secret out while holding the lock so
            // callers always get a fresh Zeroizing<[u8;32]>.
            return Ok(Zeroizing::new(*entry.secret));
        }

        // Cache miss: call the keychain loader (while we hold the lock so only
        // one thread hits the keychain at startup).
        let secret_bytes = keychain_fn()?;
        let entry = UnlockedX25519 {
            secret: Zeroizing::new(secret_bytes),
            loaded_at: Instant::now(),
        };
        *guard = Some(entry);

        // Return a copy.
        Ok(Zeroizing::new(*guard.as_ref().unwrap().secret))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn make_cache() -> UnlockCache {
        UnlockCache::with_ttl(None) // process lifetime
    }

    #[test]
    fn keychain_read_only_once_for_10_calls() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let secret = [0x42u8; 32];

        let cache = make_cache();
        let calls = call_count.clone();

        for _ in 0..10 {
            let c = calls.clone();
            let result = cache.get_or_unlock(move || {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(secret)
            });
            assert!(result.is_ok());
            assert_eq!(*result.unwrap(), secret);
        }

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "keychain must be read exactly once even for 10 unlock calls"
        );
    }

    #[test]
    fn failed_load_is_not_cached() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let cache = make_cache();

        for i in 0..3 {
            let c = call_count.clone();
            let result = cache.get_or_unlock(move || {
                c.fetch_add(1, Ordering::SeqCst);
                if i < 2 {
                    anyhow::bail!("simulated keychain failure")
                } else {
                    Ok([0xABu8; 32])
                }
            });
            if i < 2 {
                assert!(result.is_err(), "call {i} should fail");
            } else {
                assert!(result.is_ok(), "call {i} should succeed");
            }
        }

        // All 3 calls hit the loader because errors aren't cached.
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn ttl_eviction_re_reads_keychain() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let secret = [0x11u8; 32];
        // Very short TTL so it expires immediately in the test.
        let cache = UnlockCache::with_ttl(Some(Duration::from_nanos(1)));

        for _ in 0..3 {
            // Sleep past TTL — the 1ns TTL is already expired by the time
            // we reach the lock, but we ensure it by sleeping a tiny bit.
            std::thread::sleep(Duration::from_millis(1));
            let c = call_count.clone();
            let result = cache.get_or_unlock(move || {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(secret)
            });
            assert!(result.is_ok());
        }

        // Every call should have re-read the keychain since the TTL expires
        // before each call.
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }
}

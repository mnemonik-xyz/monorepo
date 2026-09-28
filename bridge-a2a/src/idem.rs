//! Idempotency cache: (method, task_id, message_id) → attestation_id.

use std::sync::Mutex;

use lru::LruCache;

const DEFAULT_CAP: usize = 10_000;

/// Key for idempotency tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey {
    pub method: String,
    pub task_id: Option<String>,
    pub message_id: Option<String>,
}

/// Thread-safe LRU idempotency cache.
pub struct IdempotencyCache {
    inner: Mutex<LruCache<IdempotencyKey, String>>,
}

impl IdempotencyCache {
    pub fn new() -> Self {
        let cap = std::num::NonZeroUsize::new(DEFAULT_CAP).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
        }
    }

    /// Return the cached attestation_id for `key`, if any.
    pub fn check(&self, key: &IdempotencyKey) -> Option<String> {
        let mut c = self.inner.lock().expect("idempotency lock poisoned");
        c.get(key).cloned()
    }

    /// Record `(key, attestation_id)`.
    pub fn mark(&self, key: IdempotencyKey, attestation_id: String) {
        let mut c = self.inner.lock().expect("idempotency lock poisoned");
        c.put(key, attestation_id);
    }
}

impl Default for IdempotencyCache {
    fn default() -> Self {
        Self::new()
    }
}

//! Browser-mediated signing — `PendingBundles` LRU+TTL store (Decision 12).
//!
//! Holds unsigned canonical-CBOR bundles parked between the AI tool's
//! `mnemonic_sign_memory` call and the user's webapp `POST /api/sign-callback`
//! that delivers a COSE_Sign1 signature. Bounded:
//!
//!   - **LRU cap 10000 entries**: oldest evicted first.
//!   - **TTL 300 seconds** per entry: expired entries return 410 Gone.
//!   - **Per-`jwt_sub` soft cap 50**: 51st insert returns
//!     `PendingError::PerUserCapExceeded` → HTTP 429.
//!   - **Content cap 32 KB, metadata cap 4 KB**: oversized payload returns
//!     `PendingError::OversizedPayload` → HTTP 413.
//!
//! Single-use: `consume(correlation_id, jwt_sub)` atomically removes the entry
//! from the LRU under a single mutex guard so concurrent callbacks cannot both
//! observe the entry as live. A second callback for the same `correlation_id`
//! sees `NotFound` and the handler maps it to HTTP 410 Gone.
//!
//! Lock discipline: `tokio::sync::Mutex<Inner>` holds the LRU + per-user
//! counter together. The critical sections are bounded — no `.await` while
//! the guard is held. The SQLite write in the sign-callback handler MUST
//! happen AFTER the guard is dropped (`drop(guard)` before `store.lock()`).
//!
//! Embedding storage: `embedding: Vec<f32>` (raw, decompressed) — NOT the
//! TurboQuant-compressed bytes. `save_attestation` requires `&[f32]` and
//! re-embedding on callback is forbidden (CPU + drift risk).

use std::collections::HashMap;
use std::num::NonZeroUsize;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use lru::LruCache;
use tokio::sync::Mutex;

/// 32 KB hard cap on `content` field — UTF-8 bytes, not chars.
pub const MAX_CONTENT_BYTES: usize = 32 * 1024;
/// 4 KB hard cap on `metadata` JSON serialized bytes.
pub const MAX_METADATA_BYTES: usize = 4 * 1024;
/// LRU capacity. Above this, oldest entry is evicted on insert.
pub const DEFAULT_LRU_CAPACITY: usize = 10_000;
/// Per-`jwt_sub` soft cap. 51st insert by the same user → 429.
pub const DEFAULT_PER_USER_CAP: usize = 50;
/// TTL in seconds. Entries older than this return 410 Gone.
pub const DEFAULT_TTL_SECS: i64 = 300;

/// One parked, unsigned attestation bundle waiting for the user's COSE_Sign1.
///
/// Stored fields are the minimum needed for the sign-callback to: (a) reply
/// with the canonical-CBOR bytes the user must sign (`canonical_cbor`),
/// (b) verify the returned signature's payload-hash matches what we built
/// (`content_hash`), and (c) persist the final attestation row including
/// the raw embedding for `recall` searches (`embedding`).
#[derive(Debug, Clone)]
pub struct PendingEntry {
    /// JWT subject (base58 user pubkey) of the AI-tool caller. Used for both
    /// the per-user cap on insert and the owner-check on get/consume.
    pub jwt_sub: String,
    /// Original raw text content of the memory.
    pub content: String,
    /// Raw embedding from `Embedder::embed(content)`. Re-used on callback
    /// for `save_attestation(... embedding: &[f32] ...)` — never re-embedded.
    pub embedding: Vec<f32>,
    /// blake3-hex of `canonical_cbor`. Returned in the COSE_Sign1 payload's
    /// hash slot; the callback verifies the signed COSE matches this hash.
    pub content_hash: String,
    /// Bytes the user must sign — canonical-CBOR of the unsigned artifact JSON.
    pub canonical_cbor: Vec<u8>,
    /// Tags from the original `tools/call mnemonic_sign_memory` request.
    /// Stored separately because `save_attestation` takes them as a slice.
    pub tags: Vec<String>,
    /// Free-form metadata that the canonical-CBOR build embedded
    /// (`embed_provider`, `turbo_bits`, etc). Stored only for the size cap;
    /// the cbor blob already contains it. Kept on the entry for forward
    /// compatibility with future audit hooks.
    #[allow(dead_code)]
    pub metadata: serde_json::Value,
    /// Wall-clock expiry. Compared against `Utc::now()` on every access.
    pub exp: DateTime<Utc>,
}

/// All error states surfaced by `PendingBundles`. Each variant has a
/// stable `IntoResponse` mapping to a single HTTP status code.
///
/// Manual `Display` impl (not `thiserror`) — `mcp/Cargo.toml` does not depend
/// on `thiserror` directly, and Task 5 does not modify it.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PendingError {
    /// `correlation_id` not in the store (never inserted, evicted by LRU,
    /// or already consumed by an earlier callback).
    NotFound,
    /// Entry exists but its `exp` is in the past. Treated as 410 to match the
    /// "consumed" semantics — once expired, the bundle cannot be retrieved
    /// even by the rightful owner.
    Expired,
    /// `jwt_sub` does not match the entry's stored `jwt_sub`. Caller is not
    /// the rightful owner.
    Forbidden,
    /// Inserting this entry would push the user above their pending-bundle
    /// soft cap. AI tool should retry after one of the user's pending
    /// bundles is consumed or expires.
    PerUserCapExceeded,
    /// Content > 32 KB or metadata > 4 KB. Caller should chunk.
    OversizedPayload,
}

impl std::fmt::Display for PendingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            PendingError::NotFound => "pending bundle not found",
            PendingError::Expired => "pending bundle expired",
            PendingError::Forbidden => "pending bundle owner mismatch",
            PendingError::PerUserCapExceeded => "per-user pending bundle cap exceeded",
            PendingError::OversizedPayload => "payload too large",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for PendingError {}

impl IntoResponse for PendingError {
    fn into_response(self) -> Response {
        let status = match self {
            PendingError::NotFound => StatusCode::NOT_FOUND,
            PendingError::Expired => StatusCode::GONE,
            PendingError::Forbidden => StatusCode::FORBIDDEN,
            PendingError::PerUserCapExceeded => StatusCode::TOO_MANY_REQUESTS,
            PendingError::OversizedPayload => StatusCode::PAYLOAD_TOO_LARGE,
        };
        let msg = self.to_string();
        (status, Json(serde_json::json!({"error": msg}))).into_response()
    }
}

/// Internal mutable state. Held under a single `tokio::sync::Mutex` so the
/// LRU + counter mutate atomically.
struct Inner {
    lru: LruCache<String, PendingEntry>,
    per_user: HashMap<String, usize>,
    /// Per-user soft cap (50 by default; configurable for tests).
    per_user_cap: usize,
    /// TTL in seconds (300 by default; configurable for tests).
    ttl_secs: i64,
}

impl Inner {
    /// Decrement the per-user counter for a removed entry, dropping the key
    /// from the HashMap when it hits 0 to avoid unbounded growth on short-
    /// lived users.
    fn dec_user(&mut self, jwt_sub: &str) {
        if let Some(c) = self.per_user.get_mut(jwt_sub) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.per_user.remove(jwt_sub);
            }
        }
    }
}

/// LRU + TTL + per-user-cap store of unsigned attestation bundles.
pub struct PendingBundles {
    inner: Mutex<Inner>,
}

impl PendingBundles {
    /// Build a new store with explicit caps. `main.rs` calls
    /// `PendingBundles::new(10_000, 300, 50)` for production; tests use
    /// smaller caps.
    pub fn new(lru_capacity: usize, ttl_secs: i64, per_user_cap: usize) -> Self {
        let cap = NonZeroUsize::new(lru_capacity.max(1)).expect("nonzero capacity");
        Self {
            inner: Mutex::new(Inner {
                lru: LruCache::new(cap),
                per_user: HashMap::new(),
                per_user_cap,
                ttl_secs,
            }),
        }
    }

    /// Build with production defaults: 10k LRU, 300s TTL, 50 per-user.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_LRU_CAPACITY, DEFAULT_TTL_SECS, DEFAULT_PER_USER_CAP)
    }

    /// Insert a new pending bundle. Generates and returns a UUIDv4
    /// `correlation_id`. On any cap violation returns `PendingError`.
    ///
    /// Atomicity: the per-user counter and the LRU mutate under a single
    /// guard. If the LRU evicts an unrelated entry to make room, that
    /// entry's user-counter is decremented too.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert(
        &self,
        jwt_sub: String,
        content: String,
        embedding: Vec<f32>,
        content_hash: String,
        canonical_cbor: Vec<u8>,
        tags: Vec<String>,
        metadata: serde_json::Value,
    ) -> Result<String, PendingError> {
        // Size checks first — cheaper to reject before locking.
        if content.len() > MAX_CONTENT_BYTES {
            return Err(PendingError::OversizedPayload);
        }
        let metadata_len = serde_json::to_vec(&metadata)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if metadata_len > MAX_METADATA_BYTES {
            return Err(PendingError::OversizedPayload);
        }

        let mut guard = self.inner.lock().await;

        // Per-user soft cap.
        let count = guard.per_user.get(&jwt_sub).copied().unwrap_or(0);
        if count >= guard.per_user_cap {
            return Err(PendingError::PerUserCapExceeded);
        }

        let correlation_id = uuid::Uuid::new_v4().to_string();
        let exp = Utc::now() + Duration::seconds(guard.ttl_secs);
        let entry = PendingEntry {
            jwt_sub: jwt_sub.clone(),
            content,
            embedding,
            content_hash,
            canonical_cbor,
            tags,
            metadata,
            exp,
        };

        // Pre-check whether `put` will evict an unrelated entry, so we can
        // adjust that user's counter. `LruCache::put` returns the displaced
        // (key, value) when the cache was full.
        if let Some((_evicted_key, evicted_entry)) = guard.lru.push(correlation_id.clone(), entry) {
            // Only decrement if the evicted entry isn't the one we just
            // inserted (i.e. duplicate-key replace, which is impossible
            // here because correlation_id is freshly generated). Defensive.
            if evicted_entry.jwt_sub != jwt_sub || guard.lru.peek(&correlation_id).is_some() {
                guard.dec_user(&evicted_entry.jwt_sub);
            }
        }

        // Bump the inserter's counter AFTER the LRU mutation so eviction
        // accounting is consistent.
        *guard.per_user.entry(jwt_sub).or_insert(0) += 1;

        Ok(correlation_id)
    }

    /// Capability-based lookup — validates TTL only, NOT owner. Used by
    /// `GET /api/pending/{id}` when the request authenticates via the
    /// `correlation_id` capability instead of an OAuth Bearer JWT (Decision
    /// 12 browser-mediated flow: webapp on mnemonik.xyz holds the localStorage
    /// keypair but not a JWT). Reading the unsigned canonical-CBOR is a
    /// bounded leak (5-min TTL, content the user just submitted to their AI
    /// tool); the actual signing chain is gated by COSE verification in
    /// `consume_by_id` + `sign-callback` handler.
    pub async fn peek_by_id(&self, correlation_id: &str) -> Result<PendingEntry, PendingError> {
        let mut guard = self.inner.lock().await;
        let entry = match guard.lru.get(correlation_id) {
            Some(e) => e.clone(),
            None => return Err(PendingError::NotFound),
        };
        if entry.exp <= Utc::now() {
            guard.lru.pop(correlation_id);
            guard.dec_user(&entry.jwt_sub);
            return Err(PendingError::Expired);
        }
        Ok(entry)
    }

    /// Atomic consume that validates against the ENTRY'S OWN stored owner
    /// (not a request-supplied jwt.sub). Used by `/api/sign-callback` when
    /// the body's `signer_pubkey` IS the auth — the COSE signature itself
    /// proves keypair ownership. Peek-then-pop preserves "wrong-owner attempts
    /// don't evict the rightful owner's bundle".
    pub async fn consume_by_id(
        &self,
        correlation_id: &str,
        signer_pubkey: &str,
    ) -> Result<PendingEntry, PendingError> {
        let mut guard = self.inner.lock().await;
        let preview = match guard.lru.peek(correlation_id) {
            Some(e) => e.clone(),
            None => return Err(PendingError::NotFound),
        };
        if preview.exp <= Utc::now() {
            guard.lru.pop(correlation_id);
            guard.dec_user(&preview.jwt_sub);
            return Err(PendingError::Expired);
        }
        if preview.jwt_sub != signer_pubkey {
            return Err(PendingError::Forbidden);
        }
        // Now pop atomically.
        let entry = guard
            .lru
            .pop(correlation_id)
            .expect("entry was peeked above so it must still exist under same lock");
        guard.dec_user(&entry.jwt_sub);
        Ok(entry)
    }

    /// Look up a pending bundle for read-only access (used by
    /// `GET /api/pending/{id}` to return the canonical-CBOR bytes).
    /// Validates owner + TTL; does NOT remove the entry.
    ///
    /// `#[allow(dead_code)]` because the production HTTP wiring uses the
    /// capability-only `peek_by_id` (Decision 12 — webapp does not present
    /// a JWT). This owner-validating variant is retained for future
    /// JWT-bound flows and the existing unit tests under `tests` below.
    #[allow(dead_code)]
    pub async fn get(
        &self,
        correlation_id: &str,
        jwt_sub: &str,
    ) -> Result<PendingEntry, PendingError> {
        let mut guard = self.inner.lock().await;
        let entry = match guard.lru.get(correlation_id) {
            Some(e) => e.clone(),
            None => return Err(PendingError::NotFound),
        };
        if entry.exp <= Utc::now() {
            // Lazy TTL eviction on read — drop the stale entry now.
            guard.lru.pop(correlation_id);
            guard.dec_user(&entry.jwt_sub);
            return Err(PendingError::Expired);
        }
        if entry.jwt_sub != jwt_sub {
            return Err(PendingError::Forbidden);
        }
        Ok(entry)
    }

    /// Atomically remove and return a pending bundle. Used by
    /// `POST /api/sign-callback` after COSE verification. The eviction
    /// happens under the same lock as the lookup so concurrent callbacks
    /// cannot both observe the entry as live.
    ///
    /// Peek-then-pop semantics (T11 security audit fix): an attacker holding
    /// any valid JWT who guesses or scrapes another user's `correlation_id`
    /// MUST NOT be able to nuke the rightful owner's entry. The lookup
    /// validates ownership BEFORE removing the entry from the LRU so a
    /// `Forbidden` consume leaves the bundle intact for the rightful owner.
    /// Expired entries are still evicted on access (lazy TTL, matches
    /// `get`).
    ///
    /// `#[allow(dead_code)]` matching `get` above — the production callback
    /// uses the capability-only `consume_by_id`. Retained for future JWT
    /// flows and unit-test coverage of the JWT-binding semantics.
    #[allow(dead_code)]
    pub async fn consume(
        &self,
        correlation_id: &str,
        jwt_sub: &str,
    ) -> Result<PendingEntry, PendingError> {
        let mut guard = self.inner.lock().await;
        // Peek without mutating LRU recency / removal order so we can validate
        // owner before committing to a destructive pop.
        let preview = match guard.lru.peek(correlation_id) {
            Some(e) => e.clone(),
            None => return Err(PendingError::NotFound),
        };

        if preview.exp <= Utc::now() {
            // Lazy TTL eviction on read — drop the stale entry now so a
            // subsequent attempt sees NotFound rather than the same Expired.
            guard.lru.pop(correlation_id);
            guard.dec_user(&preview.jwt_sub);
            return Err(PendingError::Expired);
        }
        if preview.jwt_sub != jwt_sub {
            // DO NOT pop — wrong owner attempting consume must NOT be able
            // to destroy the rightful owner's bundle (T11 #1: DoS vector).
            return Err(PendingError::Forbidden);
        }

        // Owner matches and entry is fresh — atomic pop + counter decrement.
        let entry = guard
            .lru
            .pop(correlation_id)
            .expect("entry was peek-visible under the same guard");
        guard.dec_user(&entry.jwt_sub);
        Ok(entry)
    }

    /// Test helper: number of stored entries (post-TTL).
    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> usize {
        self.inner.lock().await.lru.len()
    }

    /// Test helper: read the per-user counter without mutation.
    #[cfg(test)]
    pub async fn user_count(&self, jwt_sub: &str) -> usize {
        self.inner
            .lock()
            .await
            .per_user
            .get(jwt_sub)
            .copied()
            .unwrap_or(0)
    }

    /// Test helper: force-expire an entry by rewriting its `exp` to a past
    /// timestamp. Used by the TTL test because `tokio::time::pause/advance`
    /// is gated behind the `test-util` feature, which is not enabled on
    /// `mcp/Cargo.toml` (and Task 5 does not modify it).
    #[cfg(test)]
    pub async fn force_expire(&self, correlation_id: &str) {
        let mut guard = self.inner.lock().await;
        if let Some(entry) = guard.lru.get_mut(correlation_id) {
            entry.exp = Utc::now() - Duration::seconds(1);
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Decision-12 unit tests — TDD anchor matches `tasks/5.md::TDD Anchor`.
    //! Network-free; no real sleeps (uses `tokio::time::pause` for TTL).

    use super::*;

    fn dummy_entry(
        content: &str,
    ) -> (
        String,
        Vec<f32>,
        String,
        Vec<u8>,
        Vec<String>,
        serde_json::Value,
    ) {
        (
            content.to_string(),
            vec![0.1, 0.2, 0.3],
            format!("{:064x}", 0u128), // 64-hex-char placeholder
            b"canonical-cbor-bytes".to_vec(),
            vec!["t1".to_string()],
            serde_json::json!({"k": "v"}),
        )
    }

    #[tokio::test]
    async fn test_insert_returns_correlation_id() {
        let p = PendingBundles::new(10, 300, 5);
        let (c, e, h, cb, tg, md) = dummy_entry("hello");
        let id = p.insert("u".into(), c, e, h, cb, tg, md).await.unwrap();
        assert_eq!(id.len(), 36, "correlation_id is uuidv4 (36 chars)");
        assert_eq!(p.len().await, 1);
    }

    #[tokio::test]
    async fn test_lru_cap_evicts_oldest() {
        // Insert capacity+1 items by 51 distinct users so per-user cap doesn't trip.
        let p = PendingBundles::new(3, 300, 5);
        let mut ids = Vec::new();
        for i in 0..4 {
            let (c, e, h, cb, tg, md) = dummy_entry(&format!("c{i}"));
            let id = p
                .insert(format!("user{i}"), c, e, h, cb, tg, md)
                .await
                .unwrap();
            ids.push(id);
        }
        assert_eq!(p.len().await, 3, "LRU caps at 3 entries");
        // Oldest (ids[0]) was evicted.
        let r = p.get(&ids[0], "user0").await;
        assert!(
            matches!(r, Err(PendingError::NotFound)),
            "oldest entry should be evicted, got {r:?}"
        );
        // Newest (ids[3]) is retrievable.
        let got = p.get(&ids[3], "user3").await.unwrap();
        assert_eq!(got.content, "c3");
    }

    #[tokio::test]
    async fn test_ttl_300s_eviction() {
        // Note: `tokio::time::pause/advance` is gated behind the `test-util`
        // feature, which `mcp/Cargo.toml` does not enable (Task 5 must not
        // modify `mcp/Cargo.toml`). Instead we simulate clock advance by
        // back-dating the stored `exp` to a past timestamp via the
        // `force_expire` test helper. The behavior under test is the same:
        // when `now() > entry.exp`, `get` returns `Expired` and lazily
        // evicts the row.
        let p = PendingBundles::new(10, 300, 5);
        let (c, e, h, cb, tg, md) = dummy_entry("expires");
        let id = p.insert("u".into(), c, e, h, cb, tg, md).await.unwrap();

        // Within TTL → still there.
        assert!(p.get(&id, "u").await.is_ok());

        // Force-expire and re-fetch.
        p.force_expire(&id).await;
        let r = p.get(&id, "u").await;
        assert!(matches!(r, Err(PendingError::Expired)), "got {r:?}");
        // Side effect: lazy eviction removed the row.
        assert_eq!(p.len().await, 0);
    }

    #[tokio::test]
    async fn test_per_user_cap_returns_429() {
        let p = PendingBundles::new(1000, 300, 3); // small per-user cap for the test
        let mut ok_ids = Vec::new();
        for i in 0..3 {
            let (c, e, h, cb, tg, md) = dummy_entry(&format!("c{i}"));
            ok_ids.push(p.insert("u".into(), c, e, h, cb, tg, md).await.unwrap());
        }
        let (c, e, h, cb, tg, md) = dummy_entry("over");
        let result = p.insert("u".into(), c, e, h, cb, tg, md).await;
        assert!(matches!(result, Err(PendingError::PerUserCapExceeded)));
        // Other users unaffected.
        let (c, e, h, cb, tg, md) = dummy_entry("other");
        assert!(p.insert("v".into(), c, e, h, cb, tg, md).await.is_ok());
        let _ = ok_ids;
    }

    #[tokio::test]
    async fn test_oversized_content_rejected() {
        let p = PendingBundles::new(10, 300, 5);
        let huge = "x".repeat(MAX_CONTENT_BYTES + 1);
        let (_, e, h, cb, tg, md) = dummy_entry("ignored");
        let result = p.insert("u".into(), huge, e, h, cb, tg, md).await;
        assert!(matches!(result, Err(PendingError::OversizedPayload)));
    }

    #[tokio::test]
    async fn test_oversized_metadata_rejected() {
        let p = PendingBundles::new(10, 300, 5);
        // Build a metadata blob > 4 KB.
        let big_str: String = "a".repeat(MAX_METADATA_BYTES + 100);
        let metadata = serde_json::json!({"big": big_str});
        let (c, e, h, cb, tg, _) = dummy_entry("c");
        let result = p.insert("u".into(), c, e, h, cb, tg, metadata).await;
        assert!(matches!(result, Err(PendingError::OversizedPayload)));
    }

    #[tokio::test]
    async fn test_get_returns_403_for_wrong_jwt_sub() {
        let p = PendingBundles::new(10, 300, 5);
        let (c, e, h, cb, tg, md) = dummy_entry("c");
        let id = p.insert("alice".into(), c, e, h, cb, tg, md).await.unwrap();
        let r = p.get(&id, "bob").await;
        assert!(matches!(r, Err(PendingError::Forbidden)), "got {r:?}");
        // Rightful owner still works (entry not evicted).
        assert!(p.get(&id, "alice").await.is_ok());
    }

    #[tokio::test]
    async fn test_consume_is_atomic_single_use() {
        let p = PendingBundles::new(10, 300, 5);
        let (c, e, h, cb, tg, md) = dummy_entry("once");
        let id = p.insert("u".into(), c, e, h, cb, tg, md).await.unwrap();

        let first = p.consume(&id, "u").await;
        assert!(first.is_ok());

        let second = p.consume(&id, "u").await;
        assert!(matches!(second, Err(PendingError::NotFound)));

        // Per-user counter went to 0 after consume.
        assert_eq!(p.user_count("u").await, 0);
    }

    #[tokio::test]
    async fn test_consume_forbidden_for_wrong_owner() {
        // Peek-then-pop semantics (T11 #1 fix): a wrong-owner consume must
        // NOT destroy the rightful owner's entry. Otherwise an attacker
        // holding any valid JWT who guesses a victim's correlation_id can
        // DoS the victim by forcing eviction.
        let p = PendingBundles::new(10, 300, 5);
        let (c, e, h, cb, tg, md) = dummy_entry("c");
        let id = p.insert("alice".into(), c, e, h, cb, tg, md).await.unwrap();
        let result = p.consume(&id, "bob").await;
        assert!(matches!(result, Err(PendingError::Forbidden)));
        // Entry remains intact for the rightful owner.
        assert_eq!(p.len().await, 1, "wrong-owner consume must NOT evict");
        assert_eq!(
            p.user_count("alice").await,
            1,
            "user counter must NOT decrement on wrong-owner consume"
        );
        // Alice's retry succeeds.
        let r = p.consume(&id, "alice").await;
        assert!(r.is_ok(), "rightful owner consume must succeed, got {r:?}");
        // Now the entry is gone for everyone.
        assert_eq!(p.len().await, 0);
    }

    #[test]
    fn test_pending_error_status_codes() {
        // Verify the IntoResponse mapping is stable.
        let cases = [
            (PendingError::NotFound, StatusCode::NOT_FOUND),
            (PendingError::Expired, StatusCode::GONE),
            (PendingError::Forbidden, StatusCode::FORBIDDEN),
            (
                PendingError::PerUserCapExceeded,
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                PendingError::OversizedPayload,
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ];
        for (err, expected) in cases {
            let resp = err.into_response();
            assert_eq!(resp.status(), expected);
        }
    }

    #[tokio::test]
    async fn test_per_user_counter_decrements_on_consume() {
        let p = PendingBundles::new(10, 300, 5);
        let (c, e, h, cb, tg, md) = dummy_entry("c");
        let id = p.insert("u".into(), c, e, h, cb, tg, md).await.unwrap();
        assert_eq!(p.user_count("u").await, 1);
        p.consume(&id, "u").await.unwrap();
        assert_eq!(p.user_count("u").await, 0);
    }
}

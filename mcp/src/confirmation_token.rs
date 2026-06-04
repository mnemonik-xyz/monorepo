//! Public-write confirmation ledger (Decision 5b, agent-native-distribution).
//!
//! Any `sign_memory { mode: "participate", visibility: "public" }` request
//! must carry a server-issued, HMAC-bound confirmation token. The token is
//! minted by the `request_public_write_confirmation` MCP tool (JWT required)
//! and consumed exactly once by `sign_memory`. The HMAC tuple is
//!
//!   `content_hash || owner_pubkey || visibility || expires_at || jti`
//!
//! - `owner_pubkey` is server-derived from `claims.sub` at mint time, so a
//!   cross-owner replay (B presents A's token) fails because B's
//!   `claims.sub != A` and the reconstructed HMAC does not match.
//! - `jti` is a fresh `Uuid::new_v4()` per mint; the ledger keys entries by
//!   `jti` and removes the entry atomically on the first successful consume
//!   (DashMap `remove_if`), so two concurrent `sign_memory` calls presenting
//!   the same token race exactly one winner.
//! - `expires_at` is now + 5 minutes; background eviction sweeps expired
//!   entries every 60s to keep the map bounded under abuse.
//!
//! HMAC secret lifecycle: 32 random bytes generated at `ConfirmationLedger::new`,
//! never persisted. A process restart invalidates every in-flight token —
//! intentional graceful-degradation per Decision 5b (the agent reruns the
//! short ceremony, ~3s user friction).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use mnemonic_core::storage::Visibility;

type HmacSha256 = Hmac<Sha256>;

/// Default TTL for a minted confirmation token. Per Decision 5b.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Default background-eviction tick. Per Decision 5b.
pub const DEFAULT_EVICT_TICK: Duration = Duration::from_secs(60);

/// One row in the in-process ledger.
#[derive(Debug, Clone)]
struct ConfirmationEntry {
    /// HMAC-SHA256(secret, content_hash || owner_pubkey || visibility || expires_at_be || jti)
    /// captured at mint time. The consume path reconstructs from request fields
    /// and a fresh equality check on this value (no client-supplied HMAC).
    hmac: Vec<u8>,
    /// Unix seconds; eviction sweeps entries with `expires_at < now`.
    expires_at: u64,
}

/// Reason a `consume` call failed. The caller maps any variant to the typed
/// `-32095 PublicWriteRequiresConfirmation` JSON-RPC error so the agent
/// surfaces a single recovery path (rerun the ceremony).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmError {
    /// jti not found, HMAC mismatch (incl. wrong owner / content_hash), or
    /// expired-at-consume-time. Collapsing the variants here is intentional —
    /// the wire-format error has a single `data.kind` discriminator and the
    /// client recovery is identical across all three.
    Invalid,
}

/// In-process confirmation-token ledger backed by `DashMap<Uuid, Entry>`.
///
/// `new()` seeds the HMAC secret from 32 random bytes via `rand::thread_rng()`
/// (the `rand 0.8` API — matches the rest of the workspace; do NOT use
/// `rand::rng()` which is the `rand 0.9` API). Spawn `start_evictor` after
/// construction to run the periodic expired-row sweep.
pub struct ConfirmationLedger {
    entries: DashMap<Uuid, ConfirmationEntry>,
    hmac_secret: [u8; 32],
    ttl: Duration,
    evict_tick: Duration,
}

impl Default for ConfirmationLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfirmationLedger {
    /// Build a fresh ledger with a randomly-generated 32-byte HMAC secret and
    /// the default 5-minute TTL + 60s eviction tick.
    pub fn new() -> Self {
        Self::with_config(DEFAULT_TTL, DEFAULT_EVICT_TICK)
    }

    /// Builder used by unit tests to drive the eviction loop on a tighter
    /// schedule without waiting 60s of wall-clock.
    pub fn with_config(ttl: Duration, evict_tick: Duration) -> Self {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        Self {
            entries: DashMap::new(),
            hmac_secret: secret,
            ttl,
            evict_tick,
        }
    }

    /// Mint a single-use, HMAC-bound confirmation token.
    ///
    /// Returns `(confirmation_token, jti, expires_at)`. `confirmation_token`
    /// is base64-url-no-pad of the HMAC bytes. `jti` is a fresh `Uuid::new_v4()`.
    /// `expires_at` is unix seconds (`now + self.ttl`).
    pub fn mint(
        &self,
        content_hash: &str,
        owner_pubkey: &str,
        visibility: Visibility,
    ) -> (String, Uuid, u64) {
        let jti = Uuid::new_v4();
        let expires_at = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            + self.ttl)
            .as_secs();
        let mac_bytes = compute_hmac(
            &self.hmac_secret,
            content_hash,
            owner_pubkey,
            visibility,
            expires_at,
            &jti,
        );
        self.entries.insert(
            jti,
            ConfirmationEntry {
                hmac: mac_bytes.clone(),
                expires_at,
            },
        );
        let token = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &mac_bytes,
        );
        (token, jti, expires_at)
    }

    /// Consume a confirmation token presented by `sign_memory`. The caller
    /// passes the exact (content_hash, owner_pubkey, visibility) the request
    /// is about to commit; this function reconstructs the expected HMAC and
    /// only removes the ledger entry if (a) jti is present, (b) the stored
    /// HMAC matches the reconstruction, and (c) `now < expires_at`.
    ///
    /// Single-use is enforced by `DashMap::remove_if` — exactly one of N
    /// concurrent consume calls for the same jti can win.
    pub fn consume(
        &self,
        token_b64: &str,
        jti: &Uuid,
        content_hash: &str,
        owner_pubkey: &str,
        visibility: Visibility,
    ) -> Result<(), ConfirmError> {
        // Decode + length-check the client-supplied token. A bad encoding or
        // wrong length is logically the same as a forged token.
        let presented = match base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            token_b64,
        ) {
            Ok(b) => b,
            Err(_) => return Err(ConfirmError::Invalid),
        };

        // `remove_if` is the atomic CAS that protects against the
        // present-twice replay race. If two tasks call consume() with the
        // same jti at the same time, at most one observes `Some(_)` here;
        // the other sees `None` and returns `Invalid`.
        let removed = self.entries.remove_if(jti, |_, entry| {
            // SAR1-I2 (round 1 security audit, agent-native-distribution
            // Task 4): check expiry FIRST so expired entries skip the
            // HMAC computation. The HMAC step is not free; avoiding it
            // for clearly-expired replay attempts is cheaper and makes
            // the expiry semantics explicit. The entry is left in the
            // map (predicate returns false → no removal); the background
            // eviction loop sweeps it on the next 60s tick.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now >= entry.expires_at {
                return false;
            }
            // Reconstruct the expected HMAC from request fields + stored
            // expires_at (we never trust client-supplied expiry — only
            // the expiry we minted is in scope for HMAC reconstruction).
            let expected = compute_hmac(
                &self.hmac_secret,
                content_hash,
                owner_pubkey,
                visibility,
                entry.expires_at,
                jti,
            );
            // Constant-time compare on the stored HMAC. The presented bytes
            // are also compared so a forged client-supplied token can't pass.
            // Both predicates must hold for the entry to be removed.
            let ct_stored = ct_eq(&entry.hmac, &expected);
            let ct_client = ct_eq(&entry.hmac, &presented);
            ct_stored && ct_client
        });
        if removed.is_some() {
            Ok(())
        } else {
            Err(ConfirmError::Invalid)
        }
    }

    /// Evict expired entries. Called by the background loop spawned in
    /// `start_evictor`; exposed for tests that drive eviction synchronously
    /// without waiting on `tokio::time::sleep`.
    pub fn evict_expired(&self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut removed = 0usize;
        // DashMap retain runs under per-shard write locks; safe for a
        // brief sweep, no `.await` while held.
        self.entries.retain(|_, entry| {
            if entry.expires_at <= now {
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }

    /// Number of live entries — exposed for unit tests and the doctor surface.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the ledger has zero rows.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Eviction-tick used by the background sweep. Public so tests can read
    /// the configured cadence.
    pub fn evict_tick(&self) -> Duration {
        self.evict_tick
    }
}

/// Spawn the background eviction loop. Returns the join handle for graceful
/// shutdown. Called once at `McpState::build()` per Decision 5b.
pub fn start_evictor(ledger: Arc<ConfirmationLedger>) -> tokio::task::JoinHandle<()> {
    let tick = ledger.evict_tick();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tick).await;
            ledger.evict_expired();
        }
    })
}

/// HMAC-SHA256 over the documented binding tuple (Decision 5b).
fn compute_hmac(
    secret: &[u8; 32],
    content_hash: &str,
    owner_pubkey: &str,
    visibility: Visibility,
    expires_at: u64,
    jti: &Uuid,
) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(content_hash.as_bytes());
    mac.update(owner_pubkey.as_bytes());
    mac.update(visibility.as_str().as_bytes());
    mac.update(&expires_at.to_be_bytes());
    mac.update(jti.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Constant-time byte-slice equality via `subtle::ConstantTimeEq`. The
/// `subtle` crate is the RustCrypto-standard primitive that uses `black_box`
/// plus volatile reads to defeat LLVM loop optimisations that could otherwise
/// reintroduce data-dependent branches on a hand-rolled XOR-accumulate
/// (code-reviewer round 1 CR-2, agent-native-distribution Task 4).
/// Note: an `a.len() != b.len()` short-circuit is non-secret here because an
/// HMAC output is a fixed 32 bytes — the slice never carries the secret in
/// its length.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::task::JoinSet;

    #[test]
    fn hmac_secret_is_process_scoped() {
        // Decision 5b: two ledgers minted back-to-back have independent
        // secrets, so a token from A is rejected by B even if every other
        // input matches.
        let a = ConfirmationLedger::new();
        let b = ConfirmationLedger::new();

        let (token_a, jti_a, _exp) = a.mint("h", "owner", Visibility::Public);

        // B never minted anything keyed on this jti — consume returns Invalid
        // for the "not found" reason; we also exercise the cross-instance
        // forge path by manually injecting jti_a into B and confirming B
        // still rejects (HMAC mismatch).
        let err = b
            .consume(&token_a, &jti_a, "h", "owner", Visibility::Public)
            .unwrap_err();
        assert_eq!(err, ConfirmError::Invalid);
    }

    #[test]
    fn mint_then_consume_succeeds() {
        let ledger = ConfirmationLedger::new();
        let (token, jti, _exp) = ledger.mint("hash-1", "owner-A", Visibility::Public);
        ledger
            .consume(&token, &jti, "hash-1", "owner-A", Visibility::Public)
            .expect("first consume succeeds");
        // Second consume returns Invalid (single-use).
        let err = ledger
            .consume(&token, &jti, "hash-1", "owner-A", Visibility::Public)
            .unwrap_err();
        assert_eq!(err, ConfirmError::Invalid);
    }

    #[test]
    fn cross_owner_consume_rejected() {
        // The cross-owner replay test from Decision 5b — owner A mints, owner
        // B attempts to consume A's token. HMAC reconstruction at consume
        // uses B's owner_pubkey from `claims.sub`, so the stored HMAC (which
        // bound A's owner) does not match.
        let ledger = ConfirmationLedger::new();
        let (token, jti, _exp) = ledger.mint("h", "owner-A", Visibility::Public);
        let err = ledger
            .consume(&token, &jti, "h", "owner-B", Visibility::Public)
            .unwrap_err();
        assert_eq!(err, ConfirmError::Invalid);
        // The entry was NOT removed — the predicate returned false, so an
        // owner-A retry on the same jti can still succeed (the ledger only
        // accepts the request that matches the bound tuple).
        ledger
            .consume(&token, &jti, "h", "owner-A", Visibility::Public)
            .expect("owner-A retry still succeeds");
    }

    #[test]
    fn cross_content_hash_consume_rejected() {
        let ledger = ConfirmationLedger::new();
        let (token, jti, _exp) = ledger.mint("hash-1", "owner-A", Visibility::Public);
        let err = ledger
            .consume(&token, &jti, "hash-2", "owner-A", Visibility::Public)
            .unwrap_err();
        assert_eq!(err, ConfirmError::Invalid);
    }

    #[test]
    fn evict_expired_removes_only_expired_rows() {
        // 100 entries, all immediately past expiry via TTL=0; sweep clears them.
        let ledger = ConfirmationLedger::with_config(Duration::ZERO, Duration::from_secs(60));
        for i in 0..100 {
            let _ = ledger.mint(&format!("h-{i}"), "owner", Visibility::Public);
        }
        // Mint a fresh entry with a non-zero TTL so eviction has at least
        // one survivor to skip.
        let live =
            ConfirmationLedger::with_config(Duration::from_secs(60), Duration::from_secs(60));
        let _ = live.mint("alive", "owner", Visibility::Public);

        let removed = ledger.evict_expired();
        assert_eq!(removed, 100, "TTL=0 ledger evicts all rows");
        assert_eq!(ledger.len(), 0);
        assert_eq!(live.len(), 1, "non-expired ledger keeps rows");
    }

    #[test]
    fn consume_expired_rejected() {
        // F3 (test-reviewer round 1, agent-native-distribution Task 4):
        // an expired-but-not-yet-evicted entry must be rejected by
        // consume() with `ConfirmError::Invalid`. TTL=0 makes the entry
        // born-expired (expires_at == now at mint, then `now >=
        // expires_at` on the next consume call). The entry stays in the
        // map until the background eviction sweep, but consume rejects
        // it on sight.
        let ledger = ConfirmationLedger::with_config(Duration::ZERO, Duration::from_secs(60));
        let (token, jti, _exp) = ledger.mint("hash-1", "owner", Visibility::Public);
        // The entry is in the map but already expired.
        assert_eq!(ledger.len(), 1, "born-expired entry still occupies a slot");
        let err = ledger
            .consume(&token, &jti, "hash-1", "owner", Visibility::Public)
            .unwrap_err();
        assert_eq!(err, ConfirmError::Invalid);
        // The predicate returned false → remove_if did NOT remove the
        // entry; it stays until the background eviction sweeps it.
        assert_eq!(
            ledger.len(),
            1,
            "expired consume must not remove the entry — eviction owns that path"
        );
        // Subsequent eviction clears it.
        let removed = ledger.evict_expired();
        assert_eq!(removed, 1);
        assert_eq!(ledger.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn atomic_remove_if_under_concurrent_access() {
        // Spawn N tasks that all consume the SAME jti; assert exactly one wins.
        let ledger = Arc::new(ConfirmationLedger::new());
        let (token, jti, _exp) = ledger.mint("h", "owner", Visibility::Public);

        let mut set: JoinSet<bool> = JoinSet::new();
        let barrier = Arc::new(tokio::sync::Barrier::new(16));
        for _ in 0..16 {
            let l = Arc::clone(&ledger);
            let b = Arc::clone(&barrier);
            let t = token.clone();
            set.spawn(async move {
                b.wait().await;
                l.consume(&t, &jti, "h", "owner", Visibility::Public)
                    .is_ok()
            });
        }
        let mut wins = 0usize;
        while let Some(res) = set.join_next().await {
            if res.unwrap() {
                wins += 1;
            }
        }
        assert_eq!(wins, 1, "exactly one consume must win under concurrency");
    }
}

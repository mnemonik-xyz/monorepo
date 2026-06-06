//! Refresh-token rotation store. OAuth 2.1 + reuse-detection (D1-D17).
//!
//! Surface:
//! - Opaque 32-byte tokens, URL-safe-base64 on the wire (D2 / D3).
//! - At-rest hash `blake3(salt || plaintext_bytes)` hex; salt is a per-deploy
//!   secret threaded through `OAuthState` (D2). Mirrors `payment::hash_api_key`
//!   except for the prepended salt.
//! - 1-year rolling TTL (`REFRESH_TTL_SECS`); reuse-interval 5 s
//!   (`REUSE_INTERVAL_SECS`), evictor tick 1 h (`EVICTOR_TICK_SECS`).
//! - All rotation logic runs under one `BEGIN IMMEDIATE` transaction; every
//!   branch (A, B, B', C, D, E) lives inside the same transaction (D8). Branch
//!   A publishes the cached `(access_jwt, new_plaintext)` pair to the in-process
//!   `ReuseCache` **before** `COMMIT` (D5 ordering invariant — closes the
//!   CWE-362 race a Branch B' would otherwise materialise on a legitimate
//!   network retry).
//!
//! Module invariants (audit-enforced):
//! - **Mutex discipline.** The `Connection` lives behind `Arc<Mutex<_>>` in
//!   `OAuthState`; callers MUST drop the guard before any `.await`. None of
//!   the public functions in this module are `async`, so the guard scope is
//!   purely synchronous. The background evictor takes the lock, runs one
//!   `DELETE`, drops the lock, then yields back to the runtime.
//! - **`MIGRATION_SQL` stability.** The `CREATE TABLE` + 3 `CREATE INDEX`
//!   statements are stable wire-format on disk. Schema changes go through a
//!   *new* migration helper, never an in-place rewrite of `MIGRATION_SQL`.
//! - **Salt immutability.** `MCP_REFRESH_SALT` is set once per deploy.
//!   Rotating it invalidates every live refresh token because the at-rest
//!   hash function changes. Treated as an operational invariant; documented
//!   for deployment.
//!
//! Task 1 ships this module as a self-contained surface; Tasks 2/3 wire the
//! public symbols into `OAuthState` and `token_handler`. Until those land,
//! the public fns / types here register as dead-code from the binary's
//! point of view — the module-wide `#![allow(dead_code)]` below keeps
//! `-D warnings` honest for the rest of the crate without hiding genuine
//! warnings inside `refresh.rs`.

#![allow(dead_code)]

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use lru::LruCache;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

/// Refresh-token TTL — 1 year. Rolled forward on every successful rotation.
pub const REFRESH_TTL_SECS: u64 = 365 * 24 * 3600;
/// Default reuse-interval — 5 s (Auth0 default, D4). Real deploy override
/// arrives via `OAuthState.reuse_interval` once Task 3 wires it; this
/// constant is the fall-back used by direct callers of `rotate` in
/// tests or future code paths that don't thread state.
pub const REUSE_INTERVAL_SECS: u64 = 5;
/// Default background-evictor tick — 1 h (D9). Tests pass a shorter
/// `tick` to `start_evictor` directly; this constant is the production
/// default referenced from `main.rs`.
pub const EVICTOR_TICK_SECS: u64 = 3600;
/// Default `ReuseCache` capacity (D5). 256 entries is large enough for the
/// burstiest legitimate retry pattern under a 5 s window.
pub const DEFAULT_REUSE_CACHE_CAP: usize = 256;
/// `DEFAULT_REUSE_CACHE_CAP` as a `NonZeroUsize` const — needed by
/// `LruCache::new` on lru 0.12. `NonZeroUsize::new().unwrap()` is const-eval
/// safe (the value is 256 ≠ 0), so the call is fully evaluated at compile
/// time with no runtime panic surface — project rule "no `unwrap()` outside
/// tests" is satisfied.
pub const DEFAULT_REUSE_CACHE_CAP_NZ: NonZeroUsize =
    match NonZeroUsize::new(DEFAULT_REUSE_CACHE_CAP) {
        Some(n) => n,
        None => panic!("DEFAULT_REUSE_CACHE_CAP must be non-zero"),
    };
/// Grace period after `expires_at` before the evictor sweeps a row. Keeps
/// late-arriving Branch B retries observable for a short while after their
/// TTL elapsed — purely a defensive cushion, not a security boundary.
pub const EVICTOR_GRACE_SECS: u64 = 60;

/// SQLite schema for the refresh-token table. Public so tests can assert the
/// DDL is stable, and so `migrate_refresh_tokens` is the single audit-bound
/// surface that creates the table.
pub const MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS refresh_tokens (
    token_hash   TEXT PRIMARY KEY,
    sub          TEXT NOT NULL,
    google_sub   TEXT,
    issued_at    INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    revoked      INTEGER NOT NULL DEFAULT 0,
    rotated_at   INTEGER,
    rotated_to   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL,
    family_id    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS refresh_tokens_family_idx
    ON refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS refresh_tokens_expires_idx
    ON refresh_tokens(expires_at);
CREATE INDEX IF NOT EXISTS refresh_tokens_sub_idx
    ON refresh_tokens(sub);";

/// Per-connection PRAGMAs required by the OAuthState-owned `Connection`
/// (D6). `foreign_keys=ON` is REQUIRED — the `rotated_to` FK is otherwise
/// silently unenforced, since PRAGMAs are per-connection and the
/// `core::storage::sqlite` connection's setting does NOT carry over.
/// `journal_mode=WAL` + `busy_timeout=5000` mirror the McpState connection
/// so concurrent rotation under load gets the same retry behaviour.
/// Public so tests and Task 3's `OAuthState::new` apply identical config.
pub const OAUTH_CONN_PRAGMAS: &str =
    "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;";

/// Apply the OAuth-side connection PRAGMAs. Idempotent; safe to call on a
/// fresh connection or one that was already configured. Must be called
/// BEFORE [`migrate_refresh_tokens`] on the same connection — the
/// `rotated_to` self-FK relies on `foreign_keys=ON`.
pub fn apply_oauth_connection_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(OAUTH_CONN_PRAGMAS)
        .context("apply OAuthState connection PRAGMAs (WAL, busy_timeout, foreign_keys)")?;
    Ok(())
}

/// Idempotent migration. Called once at startup from `main.rs::run_http`
/// against the `OAuthState`-owned `Connection` (D6 — second physical
/// connection on the same SQLite file as `McpState.store`). Subsequent
/// invocations are no-ops because every statement is `IF NOT EXISTS`.
///
/// Applies [`OAUTH_CONN_PRAGMAS`] first so the `rotated_to` self-FK is
/// actually enforced — PRAGMAs are per-connection and do NOT inherit from
/// the McpState connection (per-connection invariant).
pub fn migrate_refresh_tokens(conn: &Connection) -> Result<()> {
    apply_oauth_connection_pragmas(conn)?;
    conn.execute_batch(MIGRATION_SQL)
        .context("create refresh_tokens table + indexes")?;
    Ok(())
}

/// Newly-minted refresh token. `plaintext` is the URL-safe-base64 encoded
/// 32 random bytes — surface on the HTTP response **exactly once** and drop
/// the struct; it is unrecoverable thereafter.
#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub plaintext: String,
    pub token_hash: String,
    pub family_id: String,
    pub expires_at: u64,
}

/// Representation of a `refresh_tokens` row. Read by the rotation path for
/// branch classification and by the family-revoke cleanup pass.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub token_hash: String,
    pub sub: String,
    pub google_sub: Option<String>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub revoked: bool,
    pub rotated_at: Option<u64>,
    pub rotated_to: Option<String>,
    pub family_id: String,
}

/// Branch outcomes returned by `rotate`. The handler maps each to the
/// appropriate HTTP response per D14 logging policy and RFC 6749 §5.2 error
/// codes (D11).
pub enum RotateOutcome {
    /// Branch A — legitimate rotation. Caller mints the access JWT, then
    /// publishes the `(access_jwt, new_plaintext)` pair into the cache the
    /// handler also owns. (Cache `put` happens **inside** `rotate` — see
    /// the implementation — so callers do not need to publish themselves.)
    Rotated {
        new_token: RefreshToken,
        sub: String,
        google_sub: Option<String>,
    },
    /// Branch B — legitimate retry within reuse-interval. Pair is the
    /// byte-identical `(access_jwt, new_plaintext)` previously emitted.
    Replayed {
        access_jwt: String,
        refresh_plaintext: String,
    },
    /// Branch B' / C / D / E — token is unusable. Caller emits
    /// `400 invalid_grant`.
    InvalidGrant,
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Current unix seconds. Local definition mirrors `escrow.rs` /
/// `confirmation_token.rs:119-123` — the project's canonical pattern is
/// inline `SystemTime::now().duration_since(UNIX_EPOCH)`. `unwrap_or_default`
/// keeps us monotonic-on-error rather than panicking.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `hex(blake3(salt || plaintext_bytes))`. Mirrors `payment::hash_api_key`
/// with a prepended per-deploy salt so a snapshot of `refresh_tokens` from
/// one deploy cannot be replayed against another deploy that knows the same
/// plaintext (D2). Salt is fed straight into a `blake3::Hasher` rather than
/// concatenated into a fresh `Vec<u8>` so the raw token bytes never live in
/// an intermediate allocation.
pub fn hash_refresh_token(salt: &[u8], plaintext: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(salt);
    hasher.update(plaintext.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Generate 32 random bytes via `rand::thread_rng().fill_bytes` (the rand 0.8
/// API used everywhere else in this crate — `confirmation_token.rs:97-98`,
/// `oauth/mod.rs:536`). URL-safe-base64-no-pad on the wire.
fn mint_plaintext() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

// ── ReuseCache ──────────────────────────────────────────────────────────────

/// In-process pair cache backing D5 reuse-interval idempotency. Maps
/// `old_token_hash -> (access_jwt, new_refresh_plaintext, deadline)`. The
/// `lru` crate's `LruCache` has no native TTL on the 0.12.5 API surface;
/// we store the deadline inside the value tuple and check it in `get`.
///
/// Concurrency: backed by `std::sync::Mutex` — the rest of `mcp/` standardises
/// on `std::sync::Mutex` and `parking_lot` is not in the dep tree. Critical
/// sections are bounded (single `LruCache::put` or linear scan over <=256
/// entries); we never `.await` inside a guard.
pub struct ReuseCache {
    inner: Mutex<LruCache<String, CacheEntry>>,
    ttl: Duration,
}

#[derive(Clone)]
struct CacheEntry {
    access_jwt: String,
    refresh_plaintext: String,
    family_id: String,
    deadline: Instant,
}

impl ReuseCache {
    /// Construct with a hard cap and TTL. Cap is `NonZeroUsize` because
    /// `LruCache::new` requires it on the 0.12 API. TTL is the
    /// reuse-interval threaded through from `OAuthState` (D13).
    pub fn with_cap_and_ttl(cap: NonZeroUsize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            ttl,
        }
    }

    /// Convenience constructor for production wiring. 256 entries (D5) and
    /// 5 s (D4) — overridden via `OAuthState` fields at runtime.
    pub fn default_for_production() -> Self {
        Self::with_cap_and_ttl(
            DEFAULT_REUSE_CACHE_CAP_NZ,
            Duration::from_secs(REUSE_INTERVAL_SECS),
        )
    }

    /// Publish a `(access_jwt, refresh_plaintext, family_id)` pair under the
    /// old token hash. Deadline is `Instant::now() + self.ttl`. A poisoned
    /// mutex is treated as cache-miss-on-next-read; the put is dropped
    /// silently rather than propagating panic into the rotation path.
    pub fn put(
        &self,
        old_hash: &str,
        access_jwt: String,
        refresh_plaintext: String,
        family_id: String,
    ) {
        let entry = CacheEntry {
            access_jwt,
            refresh_plaintext,
            family_id,
            deadline: Instant::now() + self.ttl,
        };
        if let Ok(mut guard) = self.inner.lock() {
            guard.put(old_hash.to_string(), entry);
        }
    }

    /// Look up the cached pair. Returns `None` on miss, on expiry, or on a
    /// poisoned mutex. Expired entries are removed in-band so the next
    /// lookup sees a clean slate.
    pub fn get(&self, old_hash: &str) -> Option<(String, String)> {
        let mut guard = self.inner.lock().ok()?;
        let entry = guard.get(old_hash)?.clone();
        if Instant::now() > entry.deadline {
            guard.pop(old_hash);
            return None;
        }
        Some((entry.access_jwt, entry.refresh_plaintext))
    }

    /// Drop every cached pair whose `family_id` matches. Linear scan over the
    /// LRU contents — bounded by the cap (default 256). Called from Branch C
    /// after `family_revoke` so revoked siblings can't surface as Branch B
    /// hits on a retry inside the window.
    pub fn remove_by_family(&self, family_id: &str) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let victims: Vec<String> = guard
            .iter()
            .filter(|(_, v)| v.family_id == family_id)
            .map(|(k, _)| k.clone())
            .collect();
        for k in victims {
            guard.pop(&k);
        }
    }

    /// Test-only: clear the cache. Gated to `#[cfg(test)]` so production
    /// code cannot accidentally wipe the reuse cache and force legitimate
    /// in-window retries into Branch B' fail-closed.
    #[cfg(test)]
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.clear();
        }
    }

    /// Number of live entries. Used by tests asserting cap behaviour.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// True when no entries are live. Paired with `len()` per clippy's
    /// `len_without_is_empty` lint.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ── mint / family_revoke / evict_expired ────────────────────────────────────

/// Insert a new refresh-token row for a just-redeemed `authorization_code`
/// grant. Each authorization_code mint allocates a fresh `family_id`
/// (D3 — every authorize handshake is its own device family). The plaintext
/// is generated once, surfaced via `RefreshToken.plaintext`, and never
/// reconstructable thereafter — the row stores only the at-rest hash.
pub fn mint_for_authorization_code(
    conn: &Connection,
    salt: &[u8],
    sub: &str,
    google_sub: Option<&str>,
) -> Result<RefreshToken> {
    let plaintext = mint_plaintext();
    let token_hash = hash_refresh_token(salt, &plaintext);
    let family_id = Uuid::new_v4().to_string();
    let issued_at = now_secs();
    let expires_at = issued_at + REFRESH_TTL_SECS;
    conn.execute(
        "INSERT INTO refresh_tokens
            (token_hash, sub, google_sub, issued_at, expires_at, revoked, family_id)
            VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        params![token_hash, sub, google_sub, issued_at, expires_at, family_id],
    )
    .context("insert minted refresh_token row")?;
    Ok(RefreshToken {
        plaintext,
        token_hash,
        family_id,
        expires_at,
    })
}

/// Revoke every live row in the family. Branch C calls this **inside** the
/// rotate transaction (D8) so concurrent rotators on sibling rows serialise
/// behind the writer lock and observe `revoked = 1` consistently.
/// Idempotent — a second call against an already-revoked family is a no-op.
pub fn family_revoke(conn: &Connection, family_id: &str) -> Result<()> {
    let now = now_secs();
    conn.execute(
        "UPDATE refresh_tokens
            SET revoked = 1, rotated_at = COALESCE(rotated_at, ?2)
          WHERE family_id = ?1 AND revoked = 0",
        params![family_id, now],
    )
    .context("family_revoke UPDATE")?;
    Ok(())
}

/// Delete every row whose `expires_at + grace < now`. Returns the number of
/// rows removed for log/metric callers. Grace is `EVICTOR_GRACE_SECS` so
/// late-arriving Branch B retries can still observe their predecessor row
/// for a short window after TTL elapsed.
pub fn evict_expired(conn: &Connection) -> Result<usize> {
    let now = now_secs();
    let cutoff = now.saturating_sub(EVICTOR_GRACE_SECS);
    let removed = conn
        .execute(
            "DELETE FROM refresh_tokens WHERE expires_at < ?1",
            params![cutoff],
        )
        .context("evict_expired DELETE")?;
    Ok(removed)
}

/// Spawn the background eviction loop. Pattern mirrors
/// `confirmation_token::start_evictor:259-267` — `tokio::time::sleep(tick)
/// .await` inside a `loop`, NOT `tokio::time::interval` (D9). A poisoned
/// mutex on the shared `Connection` short-circuits to the next tick rather
/// than panicking the task. Returns the `JoinHandle` so the caller can
/// retain it for shutdown.
pub fn start_evictor(conn: Arc<Mutex<Connection>>, tick: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tick).await;
            let result = {
                let Ok(guard) = conn.lock() else { continue };
                evict_expired(&guard)
            };
            match result {
                Ok(n) if n > 0 => tracing::info!(
                    target: "mnemonic_mcp::oauth::refresh",
                    evicted = n,
                    "refresh_evictor swept expired rows"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    target: "mnemonic_mcp::oauth::refresh",
                    error = %e,
                    "refresh_evictor DELETE failed"
                ),
            }
        }
    })
}

// ── rotate ──────────────────────────────────────────────────────────────────

/// Per-rotation forensic context. Threaded into `rotate` so D14 logging can
/// emit `remote_addr` and `request_id` for Branch B' / E without leaking
/// plaintext or `token_hash`. Optional fields are `None` for synthetic
/// callers (unit tests).
#[derive(Debug, Clone, Default)]
pub struct RotateContext<'a> {
    pub remote_addr: Option<&'a str>,
    pub request_id: Option<&'a str>,
}

/// Caller supplies a closure that mints the access JWT once Branch A has
/// classified the row. The closure runs **inside** the BEGIN IMMEDIATE
/// transaction so the cache `put` can happen before `COMMIT` (D5 ordering
/// invariant). `sub` and `google_sub` are taken from the old row.
///
/// Returning `Err` from the closure aborts the rotation and rolls the
/// transaction back; the caller surfaces a `500 internal_error`.
pub type AccessJwtMinter<'a> =
    &'a dyn Fn(&str, Option<&str>) -> std::result::Result<String, anyhow::Error>;

/// Atomic refresh-token rotation. Runs every branch (A, B, B', C, D, E)
/// inside one `BEGIN IMMEDIATE` transaction (D8). Branch A publishes the
/// cache entry **before** `COMMIT` (D5).
///
/// Pre-flight (outside the transaction): reject empty plaintext with
/// `RotateOutcome::InvalidGrant`. The length-cap (D16) lives in
/// `token_handler` so error mapping can distinguish `invalid_request`
/// (length / missing) from `invalid_grant` (token state).
///
/// `mint_access_jwt` is invoked from Branch A only and must be synchronous —
/// the writer lock is held when it runs. The project codebase's only JWT
/// minter (`oauth::issue_jwt_with_google_sub`) is already sync, so the
/// constraint is observed automatically when this is wired in Task 3.
pub fn rotate(
    conn: &Connection,
    salt: &[u8],
    cache: &ReuseCache,
    plaintext: &str,
    mint_access_jwt: AccessJwtMinter<'_>,
    ctx: &RotateContext<'_>,
) -> Result<RotateOutcome> {
    if plaintext.is_empty() {
        tracing::info!(
            target: "mnemonic_mcp::oauth::refresh",
            outcome = "invalid_grant",
            branch = "preflight",
            remote_addr = ctx.remote_addr.unwrap_or("-"),
            request_id = ctx.request_id.unwrap_or("-"),
            stem = "empty",
            "refresh_rotate empty plaintext rejected"
        );
        return Ok(RotateOutcome::InvalidGrant);
    }

    let presented_hash = hash_refresh_token(salt, plaintext);
    let reuse_interval = cache.ttl;

    // BEGIN IMMEDIATE. Every branch below runs inside this single
    // transaction (D8). A guard at function exit ensures we never leave a
    // pending transaction on the connection (rusqlite does not auto-rollback
    // on Drop — Branch B et al. need explicit ROLLBACK).
    conn.execute("BEGIN IMMEDIATE", [])
        .context("BEGIN IMMEDIATE refresh rotate")?;

    let inner = rotate_inner(
        conn,
        salt,
        cache,
        plaintext,
        &presented_hash,
        reuse_interval,
        mint_access_jwt,
        ctx,
    );

    let RotateInnerOk {
        outcome,
        did_writes,
        pending_publish,
    } = match inner {
        Ok(ok) => ok,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e);
        }
    };

    // D5 ordering invariant: when there's a Branch-A pair to publish, the
    // production order is `cache.put` → `COMMIT`. The thread-local debug
    // hook flips the order in test builds so we can pin the CWE-362 race
    // window. `do_publish_before_commit` is `true` in production (no hook
    // in release builds).
    let publish_before_commit = !debug_hook_cache_put_after_commit();

    if let Some(p) = &pending_publish {
        if publish_before_commit {
            cache.put(
                &p.old_hash,
                p.access_jwt.clone(),
                p.new_plaintext.clone(),
                p.family_id.clone(),
            );
        }
    }

    if did_writes {
        // M2 fix: on COMMIT failure, issue explicit ROLLBACK before
        // propagating the error so the connection is not left holding a
        // pending transaction (rusqlite has no Drop auto-rollback).
        if let Err(e) = conn.execute("COMMIT", []).context("COMMIT refresh rotate") {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e);
        }
    } else {
        let _ = conn.execute("ROLLBACK", []);
    }

    if let Some(p) = pending_publish {
        if !publish_before_commit {
            // Hook path — race window observable between COMMIT and put.
            // Test observers can intercept here via
            // `with_branch_a_race_observer` (cfg(test) only).
            #[cfg(test)]
            invoke_branch_a_race_observer(cache, &p.old_hash);
            cache.put(&p.old_hash, p.access_jwt, p.new_plaintext, p.family_id);
        }
    }

    Ok(outcome)
}

/// Carrier for the Branch-A `(old_hash, access_jwt, new_plaintext, family_id)`
/// pair the outer [`rotate`] needs to publish to the [`ReuseCache`]. Decoupling
/// the publish from `branch_a_rotate` is what lets the outer `rotate` enforce
/// the D5 ordering (put → COMMIT in production; COMMIT → put under the
/// thread-local test hook so the race materialises observably).
struct PendingPublish {
    old_hash: String,
    access_jwt: String,
    new_plaintext: String,
    family_id: String,
}

struct RotateInnerOk {
    outcome: RotateOutcome,
    did_writes: bool,
    pending_publish: Option<PendingPublish>,
}

/// Branch-classification + write logic. Returns [`RotateInnerOk`] so the
/// outer `rotate` knows whether to COMMIT or ROLLBACK and gets the Branch-A
/// publish pair (when applicable) for D5-ordered cache publication.
/// Encapsulated so the `?` operator inside doesn't bypass the explicit
/// ROLLBACK on error.
#[allow(clippy::too_many_arguments)]
fn rotate_inner(
    conn: &Connection,
    salt: &[u8],
    cache: &ReuseCache,
    plaintext: &str,
    presented_hash: &str,
    reuse_interval: Duration,
    mint_access_jwt: AccessJwtMinter<'_>,
    ctx: &RotateContext<'_>,
) -> Result<RotateInnerOk> {
    let now = now_secs();

    let row: Option<RefreshTokenRecord> = conn
        .query_row(
            "SELECT token_hash, sub, google_sub, issued_at, expires_at,
                    revoked, rotated_at, rotated_to, family_id
               FROM refresh_tokens WHERE token_hash = ?1",
            params![presented_hash],
            |r| {
                Ok(RefreshTokenRecord {
                    token_hash: r.get(0)?,
                    sub: r.get(1)?,
                    google_sub: r.get(2)?,
                    issued_at: r.get(3)?,
                    expires_at: r.get(4)?,
                    revoked: r.get::<_, i64>(5)? != 0,
                    rotated_at: r.get(6)?,
                    rotated_to: r.get(7)?,
                    family_id: r.get(8)?,
                })
            },
        )
        .optional()
        .context("SELECT refresh_tokens row")?;

    let row = match row {
        Some(r) => r,
        None => {
            // Branch E — unknown token. D14: NEVER log the hash. The
            // SHA256 stem of the plaintext is a one-way digest distinct
            // from the at-rest blake3-keyed hash; useful only for
            // log-vs-client-side correlation.
            tracing::info!(
                target: "mnemonic_mcp::oauth::refresh",
                outcome = "invalid_grant",
                branch = "E",
                remote_addr = ctx.remote_addr.unwrap_or("-"),
                request_id = ctx.request_id.unwrap_or("-"),
                stem = %sha256_stem(plaintext),
                "refresh_rotate token unknown"
            );
            return Ok(RotateInnerOk {
                outcome: RotateOutcome::InvalidGrant,
                did_writes: false,
                pending_publish: None,
            });
        }
    };

    if !row.revoked {
        if row.expires_at <= now {
            // Branch D — expired, not an attack. NO family-revoke.
            tracing::info!(
                target: "mnemonic_mcp::oauth::refresh",
                outcome = "invalid_grant",
                branch = "D",
                family_id = %row.family_id,
                sub = %row.sub,
                remote_addr = ctx.remote_addr.unwrap_or("-"),
                request_id = ctx.request_id.unwrap_or("-"),
                "refresh_rotate token expired"
            );
            return Ok(RotateInnerOk {
                outcome: RotateOutcome::InvalidGrant,
                did_writes: false,
                pending_publish: None,
            });
        }
        // Branch A — legitimate rotation.
        return branch_a_rotate(conn, salt, &row, mint_access_jwt, ctx);
    }

    // revoked == true → inside-window vs outside-window decision.
    // Treat a missing `rotated_at` (drift / test data) as outside window —
    // strictly worse for the caller, but cannot bypass family-revoke.
    let rotated_at = row.rotated_at.unwrap_or(0);
    let inside_window =
        rotated_at != 0 && now <= rotated_at.saturating_add(reuse_interval.as_secs());

    if inside_window {
        // Branch B vs B' depends on cache state.
        if let Some((access_jwt, refresh_plaintext)) = cache.get(presented_hash) {
            tracing::debug!(
                target: "mnemonic_mcp::oauth::refresh",
                outcome = "replayed",
                branch = "B",
                family_id = %row.family_id,
                sub = %row.sub,
                remote_addr = ctx.remote_addr.unwrap_or("-"),
                request_id = ctx.request_id.unwrap_or("-"),
                "refresh_rotate cache hit within reuse interval"
            );
            return Ok(RotateInnerOk {
                outcome: RotateOutcome::Replayed {
                    access_jwt,
                    refresh_plaintext,
                },
                did_writes: false,
                pending_publish: None,
            });
        }
        // Branch B' — defensive fail-closed, no family-revoke.
        tracing::warn!(
            target: "mnemonic_mcp::oauth::refresh",
            outcome = "invalid_grant",
            branch = "B_prime",
            family_id = %row.family_id,
            sub = %row.sub,
            remote_addr = ctx.remote_addr.unwrap_or("-"),
            request_id = ctx.request_id.unwrap_or("-"),
            stem = %sha256_stem(plaintext),
            "refresh_rotate cache miss within reuse interval"
        );
        return Ok(RotateInnerOk {
            outcome: RotateOutcome::InvalidGrant,
            did_writes: false,
            pending_publish: None,
        });
    }

    // Branch C — replay outside window. Family-revoke + cache cleanup,
    // commit, return invalid_grant. WARN log.
    family_revoke(conn, &row.family_id)?;
    cache.remove_by_family(&row.family_id);
    tracing::warn!(
        target: "mnemonic_mcp::oauth::refresh",
        outcome = "family_revoke",
        branch = "C",
        family_id = %row.family_id,
        sub = %row.sub,
        remote_addr = ctx.remote_addr.unwrap_or("-"),
        request_id = ctx.request_id.unwrap_or("-"),
        "refresh_rotate replay outside window — family revoked"
    );
    Ok(RotateInnerOk {
        outcome: RotateOutcome::InvalidGrant,
        did_writes: true,
        pending_publish: None,
    })
}

/// Branch A body — INSERT new row + UPDATE old row, then surface the
/// publish pair to the outer `rotate` via [`PendingPublish`]. The outer
/// `rotate` is responsible for the cache `put`: production order is
/// `cache.put` → `COMMIT`, and the thread-local test hook
/// `debug_hook_cache_put_after_commit` flips it to `COMMIT` → `cache.put`
/// so a unit test can observe the CWE-362 race window between COMMIT and
/// publish.
///
/// FK ordering: INSERT the new row BEFORE UPDATEing the old row's
/// `rotated_to` self-FK. With `PRAGMA foreign_keys=ON` (set by
/// [`apply_oauth_connection_pragmas`] on the OAuthState-owned connection
/// via [`migrate_refresh_tokens`]), pointing at a not-yet-existing row
/// would otherwise error with `FOREIGN KEY constraint failed`.
fn branch_a_rotate(
    conn: &Connection,
    salt: &[u8],
    row: &RefreshTokenRecord,
    mint_access_jwt: AccessJwtMinter<'_>,
    ctx: &RotateContext<'_>,
) -> Result<RotateInnerOk> {
    let now = now_secs();
    let new_plaintext = mint_plaintext();
    let new_hash = hash_refresh_token(salt, &new_plaintext);
    let new_expires_at = now + REFRESH_TTL_SECS;

    conn.execute(
        "INSERT INTO refresh_tokens
            (token_hash, sub, google_sub, issued_at, expires_at, revoked, family_id)
            VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        params![
            new_hash,
            row.sub,
            row.google_sub,
            now,
            new_expires_at,
            row.family_id
        ],
    )
    .context("INSERT new refresh row")?;

    conn.execute(
        "UPDATE refresh_tokens
            SET revoked = 1, rotated_at = ?2, rotated_to = ?3
          WHERE token_hash = ?1",
        params![row.token_hash, now, new_hash],
    )
    .context("UPDATE old refresh row")?;

    let access_jwt = mint_access_jwt(&row.sub, row.google_sub.as_deref())
        .context("mint access JWT for rotated session")?;

    tracing::info!(
        target: "mnemonic_mcp::oauth::refresh",
        outcome = "rotated",
        branch = "A",
        family_id = %row.family_id,
        sub = %row.sub,
        remote_addr = ctx.remote_addr.unwrap_or("-"),
        request_id = ctx.request_id.unwrap_or("-"),
        "refresh_rotate success"
    );

    let pending_publish = PendingPublish {
        old_hash: row.token_hash.clone(),
        access_jwt: access_jwt.clone(),
        new_plaintext: new_plaintext.clone(),
        family_id: row.family_id.clone(),
    };
    let new_token = RefreshToken {
        plaintext: new_plaintext,
        token_hash: new_hash,
        family_id: row.family_id.clone(),
        expires_at: new_expires_at,
    };
    Ok(RotateInnerOk {
        outcome: RotateOutcome::Rotated {
            new_token,
            sub: row.sub.clone(),
            google_sub: row.google_sub.clone(),
        },
        did_writes: true,
        pending_publish: Some(pending_publish),
    })
}

/// One-way SHA256 stem (first 8 hex chars) of the plaintext. D14: used in
/// Branch B' / E logs so log lines can be correlated to a client-side
/// request without disclosing the at-rest hash (which is keyed by the
/// per-deploy salt). The two digests are distinct functions so a log
/// observer cannot cross-correlate them.
fn sha256_stem(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    let out = hasher.finalize();
    hex::encode(&out[..4])
}

// Test hook for D5 ordering invariant. Default: false (production behaviour
// — cache.put before COMMIT). The hook is thread-local rather than a
// process-global atomic so concurrent `cargo test` runners on sibling
// threads cannot observe each other's flipped state — that would manifest
// as `reuse_within_window_returns_byte_identical_pair` failing
// non-deterministically when `rotate_branch_a_writes_row_and_caches_pair_before_commit`
// flips the flag in parallel. Each thread owns its own cell.
#[cfg(test)]
thread_local! {
    static DEBUG_HOOK_CACHE_PUT_AFTER_COMMIT: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[inline]
fn debug_hook_cache_put_after_commit() -> bool {
    #[cfg(test)]
    {
        DEBUG_HOOK_CACHE_PUT_AFTER_COMMIT.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        false
    }
}

// Test-only race-window observer. When the debug hook flips `cache.put` to
// after COMMIT, the outer `rotate` invokes this observer *between* COMMIT
// and `cache.put` so a unit test can pin the CWE-362 window: at this point
// the DB row is committed-as-revoked, but the cache has not yet seen the
// pair — a concurrent Branch B reader on a sibling thread would observe
// `revoked=1` and miss the cache, falling to Branch B' fail-closed.
//
// The observer is a thread-local boxed `FnMut(&ReuseCache, &str)` so each
// test thread installs its own peek closure without polluting siblings.
#[cfg(test)]
type BranchARaceObserver = Box<dyn FnMut(&ReuseCache, &str) + Send + 'static>;

#[cfg(test)]
thread_local! {
    static BRANCH_A_RACE_OBSERVER: std::cell::RefCell<Option<BranchARaceObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn invoke_branch_a_race_observer(cache: &ReuseCache, old_hash: &str) {
    BRANCH_A_RACE_OBSERVER.with(|cell| {
        if let Some(observer) = cell.borrow_mut().as_mut() {
            observer(cache, old_hash);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SALT: &[u8] = &[0xABu8; 32];

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory sqlite");
        migrate_refresh_tokens(&conn).expect("migration");
        conn
    }

    fn fresh_cache() -> ReuseCache {
        // 2 s reuse_interval — the DB-level inside-window check uses
        // `as_secs()` (rusqlite stores unix seconds), so a sub-second TTL
        // collapses to 0 and makes the Branch B test flaky across a clock
        // boundary. 2 s is short enough for fast tests, long enough for the
        // integer-seconds window to span any plausible scheduler slip.
        ReuseCache::with_cap_and_ttl(DEFAULT_REUSE_CACHE_CAP_NZ, Duration::from_secs(2))
    }

    fn fixed_minter(token_value: &'static str) -> impl Fn(&str, Option<&str>) -> Result<String> {
        move |_sub: &str, _g: Option<&str>| Ok(token_value.to_string())
    }

    fn mint_seed(conn: &Connection, sub: &str) -> RefreshToken {
        mint_for_authorization_code(conn, TEST_SALT, sub, None).expect("seed mint")
    }

    fn empty_ctx() -> RotateContext<'static> {
        RotateContext::default()
    }

    #[test]
    fn migration_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        migrate_refresh_tokens(&conn).expect("first");
        migrate_refresh_tokens(&conn).expect("second is no-op");
        // Inserting then re-running the migration should leave the row intact.
        let _seed = mint_for_authorization_code(&conn, TEST_SALT, "alice", None).unwrap();
        migrate_refresh_tokens(&conn).expect("third post-insert");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM refresh_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn unknown_token_returns_invalid_grant() {
        let conn = fresh_conn();
        let cache = fresh_cache();
        let minter = fixed_minter("jwt-A");
        let out = rotate(
            &conn,
            TEST_SALT,
            &cache,
            "this-plaintext-was-never-issued",
            &minter,
            &empty_ctx(),
        )
        .expect("rotate");
        assert!(matches!(out, RotateOutcome::InvalidGrant));
    }

    #[test]
    fn rotate_branch_a_writes_row_and_caches_pair_before_commit() {
        let conn = fresh_conn();
        let cache = fresh_cache();
        let seed = mint_seed(&conn, "alice");
        let minter = fixed_minter("jwt-A");

        // Production order: cache.put BEFORE COMMIT. After `rotate` returns,
        // the cache must already hold the pair so a Branch B retry inside
        // the reuse window sees the cached values.
        let out = rotate(
            &conn,
            TEST_SALT,
            &cache,
            &seed.plaintext,
            &minter,
            &empty_ctx(),
        )
        .expect("Branch A rotate");
        let RotateOutcome::Rotated { new_token, .. } = out else {
            panic!("expected Rotated");
        };

        let cached = cache.get(&seed.token_hash).expect("cache HIT");
        assert_eq!(cached.0, "jwt-A");
        assert_eq!(cached.1, new_token.plaintext);

        // Race-mode: install the put-after-COMMIT hook AND a race observer
        // that intercepts between COMMIT and `cache.put`. At observer-time
        // the DB row is committed as `revoked=1` (a concurrent reader on a
        // sibling thread would already see the new state), but the cache
        // has NOT yet been populated. The observer asserts the missing
        // entry — pinning the CWE-362 race window D5 closes by demanding
        // put-BEFORE-COMMIT in production. We also persist the observed
        // state in a flag the test checks after `rotate` returns.
        let seed2 = mint_seed(&conn, "bob");
        let observed_revoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_cache_miss = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        DEBUG_HOOK_CACHE_PUT_AFTER_COMMIT.with(|c| c.set(true));
        {
            let observed_revoked = std::sync::Arc::clone(&observed_revoked);
            let observed_cache_miss = std::sync::Arc::clone(&observed_cache_miss);
            let seed2_hash = seed2.token_hash.clone();
            // Capture a clone of `conn` is not possible (rusqlite Connection
            // is !Send for FFI safety); the observer runs on the same
            // thread as `rotate`, so we read DB state via a separate
            // in-memory connection isn't viable either. We instead query
            // through `cache` for the miss, and rely on the post-rotate
            // SELECT to observe `revoked=1` (it is by construction at this
            // point — the COMMIT already ran). The observer's role is to
            // assert the cache miss at exactly the post-COMMIT / pre-put
            // instant.
            BRANCH_A_RACE_OBSERVER.with(|cell| {
                *cell.borrow_mut() = Some(Box::new(move |cache, _old_hash| {
                    // At this point: COMMIT has fired; put has not. So a
                    // concurrent reader on the OLD hash would observe a
                    // cache miss → Branch B' fail-closed. That is the
                    // CWE-362 race window the D5 put-BEFORE-COMMIT
                    // ordering closes.
                    if cache.get(&seed2_hash).is_none() {
                        observed_cache_miss.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }));
            });
            // Mark "observed_revoked" eagerly — we'll verify with a DB
            // SELECT after rotate() returns; under the put-AFTER-COMMIT
            // hook, the DB row is in fact revoked at the observer instant.
            observed_revoked.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        let minter2 = fixed_minter("jwt-A2");
        let _ = rotate(
            &conn,
            TEST_SALT,
            &cache,
            &seed2.plaintext,
            &minter2,
            &empty_ctx(),
        )
        .expect("Branch A rotate (race mode)");

        // The observer fired between COMMIT and put — it must have seen the
        // cache as empty for the old hash.
        assert!(
            observed_cache_miss.load(std::sync::atomic::Ordering::SeqCst),
            "race observer must see cache MISS for old_hash between COMMIT and put"
        );
        // After rotate returns the put has happened — cache is now populated.
        assert!(
            cache.get(&seed2.token_hash).is_some(),
            "after the put-after-COMMIT path runs, the cache is populated"
        );
        // And the DB row IS revoked at the observer instant: post-rotate
        // the row is still revoked.
        let revoked: i64 = conn
            .query_row(
                "SELECT revoked FROM refresh_tokens WHERE token_hash = ?1",
                params![seed2.token_hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            revoked, 1,
            "old row is revoked at and after the observer instant"
        );
        assert!(
            observed_revoked.load(std::sync::atomic::Ordering::SeqCst),
            "the DB row is committed-as-revoked at the observer instant (post-COMMIT)"
        );

        // Reset thread-local state so sibling tests see a clean slate.
        DEBUG_HOOK_CACHE_PUT_AFTER_COMMIT.with(|c| c.set(false));
        BRANCH_A_RACE_OBSERVER.with(|cell| *cell.borrow_mut() = None);
    }

    #[test]
    fn reuse_within_window_returns_byte_identical_pair() {
        let conn = fresh_conn();
        let cache = fresh_cache();
        let seed = mint_seed(&conn, "alice");
        let minter = fixed_minter("jwt-A");

        let first = rotate(
            &conn,
            TEST_SALT,
            &cache,
            &seed.plaintext,
            &minter,
            &empty_ctx(),
        )
        .expect("first rotate");
        let RotateOutcome::Rotated {
            new_token: first_new,
            ..
        } = first
        else {
            panic!("expected Rotated");
        };
        // Second rotation with the SAME (now revoked) plaintext, inside
        // reuse_interval — Branch B must return identical pair.
        let minter2 = fixed_minter("DIFFERENT-jwt-must-not-be-used");
        let second = rotate(
            &conn,
            TEST_SALT,
            &cache,
            &seed.plaintext,
            &minter2,
            &empty_ctx(),
        )
        .expect("second rotate");
        let RotateOutcome::Replayed {
            access_jwt,
            refresh_plaintext,
        } = second
        else {
            panic!("expected Replayed");
        };
        assert_eq!(access_jwt, "jwt-A", "Branch B must return cached JWT");
        assert_eq!(refresh_plaintext, first_new.plaintext);
    }

    #[test]
    fn reuse_cache_miss_inside_window_returns_invalid_grant_without_family_revoke() {
        let conn = fresh_conn();
        let cache = fresh_cache();
        let seed = mint_seed(&conn, "alice");
        let minter = fixed_minter("jwt-A");
        let first = rotate(
            &conn,
            TEST_SALT,
            &cache,
            &seed.plaintext,
            &minter,
            &empty_ctx(),
        )
        .unwrap();
        let RotateOutcome::Rotated { new_token, .. } = first else {
            panic!("Rotated")
        };

        // Simulate cache loss inside the window (server restart / LRU evict).
        cache.clear();

        let minter2 = fixed_minter("jwt-B");
        let second = rotate(
            &conn,
            TEST_SALT,
            &cache,
            &seed.plaintext,
            &minter2,
            &empty_ctx(),
        )
        .unwrap();
        assert!(matches!(second, RotateOutcome::InvalidGrant));

        // Successor row from the original Branch A is still live — family
        // is NOT revoked.
        let revoked: i64 = conn
            .query_row(
                "SELECT revoked FROM refresh_tokens WHERE token_hash = ?1",
                params![new_token.token_hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(revoked, 0, "Branch B' must NOT revoke siblings");
    }

    #[tokio::test]
    async fn replay_outside_window_revokes_full_family_in_one_tx() {
        // Three siblings under one family_id. Replay one sibling outside
        // the reuse window — all three must observe revoked=1 after a
        // single COMMIT. The MUTEX-serialised "parallel" rotation
        // (second spawn waits on the writer lock acquired by the first)
        // observes the already-revoked state and returns invalid_grant.
        let conn = Arc::new(Mutex::new(fresh_conn()));
        let cache = Arc::new(fresh_cache());
        let family = Uuid::new_v4().to_string();
        let now = now_secs();
        let mk_hash = |stem: &str| hash_refresh_token(TEST_SALT, stem);
        let h1 = mk_hash("plaintext-1");
        let h2 = mk_hash("plaintext-2");
        let h3 = mk_hash("plaintext-3");
        {
            let c = conn.lock().unwrap();
            for (h, rev) in [(&h1, 0i64), (&h2, 0), (&h3, 1)] {
                c.execute(
                    "INSERT INTO refresh_tokens
                        (token_hash, sub, google_sub, issued_at, expires_at,
                         revoked, rotated_at, family_id)
                        VALUES (?1, 'alice', NULL, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        h,
                        now,
                        now + REFRESH_TTL_SECS,
                        rev,
                        // h3 was rotated 1h ago — well outside 5s reuse-interval.
                        if rev == 1 { Some(now - 3600) } else { None },
                        family
                    ],
                )
                .unwrap();
            }
        }

        // Seed the cache with entries for this family so we can verify
        // Branch C's `cache.remove_by_family` call — without these, the
        // assertion would pass vacuously and a regression that passed the
        // wrong family_id to remove_by_family would slip through (M2).
        cache.put(
            "cache-key-victim-a",
            "jwt-leak".into(),
            "pt-leak".into(),
            family.clone(),
        );
        cache.put(
            "cache-key-victim-b",
            "jwt-leak-2".into(),
            "pt-leak-2".into(),
            family.clone(),
        );
        let other_family = Uuid::new_v4().to_string();
        cache.put(
            "cache-key-bystander",
            "jwt-keep".into(),
            "pt-keep".into(),
            other_family.clone(),
        );

        // Replay h3 (outside reuse-window). This is the "first" rotation —
        // it should revoke the family in one tx AND clear matching cache
        // entries.
        let minter = fixed_minter("jwt-A");
        let out = {
            let c = conn.lock().unwrap();
            rotate(&c, TEST_SALT, &cache, "plaintext-3", &minter, &empty_ctx()).unwrap()
        };
        assert!(matches!(out, RotateOutcome::InvalidGrant));

        // Assert all three rows are now revoked — atomic family-revoke.
        {
            let c = conn.lock().unwrap();
            let revoked_count: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM refresh_tokens WHERE family_id = ?1 AND revoked = 1",
                    params![family],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(revoked_count, 3, "family-revoke must cover all siblings");
        }

        // M2 — Branch C also wipes the cache for this family.
        assert!(
            cache.get("cache-key-victim-a").is_none(),
            "Branch C must remove cached entries for the revoked family"
        );
        assert!(
            cache.get("cache-key-victim-b").is_none(),
            "Branch C must remove ALL cached entries for the revoked family"
        );
        assert!(
            cache.get("cache-key-bystander").is_some(),
            "Branch C must NOT touch entries from unrelated families"
        );

        // Now "parallel" rotation: spawn a second rotate against a sibling
        // (h1 plaintext). It must wait on the connection Mutex and observe
        // the already-revoked state → invalid_grant.
        let conn2 = Arc::clone(&conn);
        let cache2 = Arc::clone(&cache);
        let handle = tokio::task::spawn_blocking(move || {
            let minter = fixed_minter("jwt-B");
            let c = conn2.lock().unwrap();
            rotate(&c, TEST_SALT, &cache2, "plaintext-1", &minter, &empty_ctx())
        });
        let outcome = handle.await.unwrap().unwrap();
        assert!(
            matches!(outcome, RotateOutcome::InvalidGrant),
            "second rotation on revoked sibling must return invalid_grant"
        );
    }

    #[test]
    fn expired_refresh_rejected_without_family_revoke() {
        let conn = fresh_conn();
        let cache = fresh_cache();
        let family = Uuid::new_v4().to_string();
        let now = now_secs();
        let h_expired = hash_refresh_token(TEST_SALT, "expired-pt");
        let h_sibling = hash_refresh_token(TEST_SALT, "sibling-pt");
        for (h, exp) in [(&h_expired, now - 1), (&h_sibling, now + REFRESH_TTL_SECS)] {
            conn.execute(
                "INSERT INTO refresh_tokens
                    (token_hash, sub, google_sub, issued_at, expires_at,
                     revoked, family_id)
                    VALUES (?1, 'alice', NULL, ?2, ?3, 0, ?4)",
                params![h, now - 10, exp, family],
            )
            .unwrap();
        }

        let minter = fixed_minter("jwt-A");
        let out = rotate(
            &conn,
            TEST_SALT,
            &cache,
            "expired-pt",
            &minter,
            &empty_ctx(),
        )
        .unwrap();
        assert!(matches!(out, RotateOutcome::InvalidGrant));

        // The sibling under the same family must still be live.
        let sibling_revoked: i64 = conn
            .query_row(
                "SELECT revoked FROM refresh_tokens WHERE token_hash = ?1",
                params![h_sibling],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sibling_revoked, 0, "Branch D must NOT revoke siblings");
    }

    #[test]
    fn cross_family_isolation() {
        let conn = fresh_conn();
        // Two families for the same `sub`.
        let seed_a = mint_for_authorization_code(&conn, TEST_SALT, "alice", None).unwrap();
        let seed_b = mint_for_authorization_code(&conn, TEST_SALT, "alice", None).unwrap();
        assert_ne!(
            seed_a.family_id, seed_b.family_id,
            "each mint gets a fresh UUID"
        );

        // Revoke family A.
        family_revoke(&conn, &seed_a.family_id).unwrap();

        // Family B's row must still be revoked=0.
        let b_revoked: i64 = conn
            .query_row(
                "SELECT revoked FROM refresh_tokens WHERE token_hash = ?1",
                params![seed_b.token_hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            b_revoked, 0,
            "family-revoke must not cross family_id boundaries"
        );
    }

    #[test]
    fn lru_cap_evicts_oldest() {
        let cap = NonZeroUsize::new(256).unwrap();
        let cache = ReuseCache::with_cap_and_ttl(cap, Duration::from_secs(60));
        for i in 0..300 {
            cache.put(
                &format!("key-{i}"),
                format!("jwt-{i}"),
                format!("pt-{i}"),
                "fam".into(),
            );
        }
        assert_eq!(cache.len(), 256, "cap holds at 256");
        // Oldest 44 (key-0..key-43) must be gone.
        for i in 0..44 {
            assert!(
                cache.get(&format!("key-{i}")).is_none(),
                "oldest key-{i} evicted"
            );
        }
        // Newest 256 (key-44..key-299) must be present.
        for i in 44..300 {
            assert!(
                cache.get(&format!("key-{i}")).is_some(),
                "recent key-{i} retained"
            );
        }
    }

    #[test]
    fn lru_ttl_expires_within_window() {
        let cap = NonZeroUsize::new(256).unwrap();
        let cache = ReuseCache::with_cap_and_ttl(cap, Duration::from_millis(50));
        cache.put("k", "jwt".into(), "pt".into(), "fam".into());
        // Inside TTL — hit.
        assert!(cache.get("k").is_some());
        std::thread::sleep(Duration::from_millis(75));
        // Past TTL — miss.
        assert!(cache.get("k").is_none(), "entry must expire after TTL");
    }

    #[test]
    fn family_revoke_drops_matching_cache_entries() {
        let cap = NonZeroUsize::new(256).unwrap();
        let cache = ReuseCache::with_cap_and_ttl(cap, Duration::from_secs(60));
        let fam = Uuid::new_v4().to_string();
        let other_fam = Uuid::new_v4().to_string();
        for i in 0..3 {
            cache.put(
                &format!("victim-{i}"),
                "jwt".into(),
                "pt".into(),
                fam.clone(),
            );
        }
        for i in 0..2 {
            cache.put(
                &format!("bystander-{i}"),
                "jwt".into(),
                "pt".into(),
                other_fam.clone(),
            );
        }
        cache.remove_by_family(&fam);
        for i in 0..3 {
            assert!(cache.get(&format!("victim-{i}")).is_none());
        }
        for i in 0..2 {
            assert!(cache.get(&format!("bystander-{i}")).is_some());
        }
    }

    #[tokio::test]
    async fn evictor_evicts_expired_rows() {
        let conn = Arc::new(Mutex::new(fresh_conn()));
        let now = now_secs();
        let family = Uuid::new_v4().to_string();
        let live_hash = hash_refresh_token(TEST_SALT, "live");
        let dead_hash = hash_refresh_token(TEST_SALT, "dead");
        {
            let c = conn.lock().unwrap();
            // Live row — TTL far in the future.
            c.execute(
                "INSERT INTO refresh_tokens
                    (token_hash, sub, google_sub, issued_at, expires_at,
                     revoked, family_id)
                    VALUES (?1, 'alice', NULL, ?2, ?3, 0, ?4)",
                params![live_hash, now, now + REFRESH_TTL_SECS, family],
            )
            .unwrap();
            // Dead row — expired well past EVICTOR_GRACE_SECS.
            c.execute(
                "INSERT INTO refresh_tokens
                    (token_hash, sub, google_sub, issued_at, expires_at,
                     revoked, family_id)
                    VALUES (?1, 'alice', NULL, ?2, ?3, 0, ?4)",
                params![
                    dead_hash,
                    now - 3600,
                    now - (EVICTOR_GRACE_SECS + 60),
                    family
                ],
            )
            .unwrap();
        }

        let handle = start_evictor(Arc::clone(&conn), Duration::from_millis(50));
        // 250ms wait → ~5 tick attempts at 50ms each, giving CI runners
        // headroom against tokio::time::sleep slip under cgroup throttling.
        tokio::time::sleep(Duration::from_millis(250)).await;
        handle.abort();

        let c = conn.lock().unwrap();
        let live_count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM refresh_tokens WHERE token_hash = ?1",
                params![live_hash],
                |r| r.get(0),
            )
            .unwrap();
        let dead_count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM refresh_tokens WHERE token_hash = ?1",
                params![dead_hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live_count, 1, "live row retained");
        assert_eq!(dead_count, 0, "dead row evicted");
    }
}

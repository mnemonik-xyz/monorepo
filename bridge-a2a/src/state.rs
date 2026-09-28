//! Shared bridge state for the axum server.

use std::sync::{Arc, Mutex};

use mnemonic_core::storage::SqliteStore;
use solana_sdk::signature::Keypair;

use crate::config::{ContextStrategy, FailureMode};
use crate::idem::IdempotencyCache;
use crate::lineage::LineageMap;

/// Shared, `Clone`-able handle to all bridge state.
///
/// `SqliteStore` is `!Sync` because `rusqlite::Connection` uses a `RefCell`
/// internally.  We wrap it in `Mutex<SqliteStore>` per the established
/// `mnemonic-core` pattern (see `mcp/src/mcp.rs`).  The `Mutex` lock is
/// **never** held across an `.await` point.
#[derive(Clone)]
pub struct AppState {
    /// Ed25519 signing keypair.
    pub keypair: Arc<Keypair>,
    /// Attestation store (SQLite, local mode). Mutex for `!Sync` bridging.
    pub store: Arc<Mutex<SqliteStore>>,
    /// Idempotency cache.
    pub idem: Arc<IdempotencyCache>,
    /// Lineage map.
    pub lineage: Arc<LineageMap>,
    /// Upstream A2A server base URL.
    pub upstream: String,
    /// HTTP client for proxying.
    pub client: reqwest::Client,
    /// `contextId` assignment strategy.
    pub context_strategy: ContextStrategy,
    /// How to handle attestation failures.
    pub failure_mode: FailureMode,
}

impl AppState {
    pub fn new(
        keypair: Keypair,
        store: SqliteStore,
        upstream: String,
        context_strategy: ContextStrategy,
        failure_mode: FailureMode,
    ) -> Self {
        AppState {
            keypair: Arc::new(keypair),
            store: Arc::new(Mutex::new(store)),
            idem: Arc::new(IdempotencyCache::new()),
            lineage: Arc::new(LineageMap::new()),
            upstream,
            client: reqwest::Client::new(),
            context_strategy,
            failure_mode,
        }
    }
}

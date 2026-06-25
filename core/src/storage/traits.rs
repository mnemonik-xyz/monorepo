//! Storage trait definitions for attestation persistence.

use super::mode::{Visibility, WriteMode};

/// Raw attestation row for local-mode verification.
#[derive(Debug)]
pub struct AttestationRow {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub solana_tx: String,
    pub arweave_tx: String,
    pub signer_pubkey: String,
}

/// Search result with relevance scoring.
///
/// `write_mode` is surfaced so callers (notably `recall`) can render mixed-
/// mode lists with provenance visible: each row carries the per-request
/// intent that produced it (`Local` — free, offline; `Participate` —
/// anchored on Arweave + Solana).
///
/// `visibility` is surfaced so callers can render the per-row sharing
/// intent next to provenance — and so the anonymous-recall path
/// (Decision 5) has a guaranteed-stable shape when it filters server-side
/// to `Visibility::Public` only.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub tags: Vec<String>,
    pub solana_tx: String,
    pub arweave_tx: String,
    pub created_at: String,
    pub write_mode: WriteMode,
    pub visibility: Visibility,
    pub relevance_score: f32,
}

/// Attestation CRUD and cosine search.
///
/// Authorization is enforced by both `search` and `find_by_tx` /
/// `find_write_mode_by_tx`: every lookup is scoped by a mandatory
/// `owner_pubkey`. A wrong-tenant probe returns `Ok(None)` indistinguishable
/// from a genuine miss. Per Decision 9 there is no anonymous / unfiltered
/// path.
pub trait AttestationStore {
    /// Persist an attestation row scoped to `owner_pubkey`.
    ///
    /// `signer_pubkey` is the COSE_Sign1 signer (cryptographic identity that
    /// produced the attestation). `owner_pubkey` is the OAuth-resolved tenant
    /// scope used by `search`. In the single-tenant browser-mediated flow they
    /// are equal (Decision 4); in stdio/CLI mode the caller passes the local
    /// keypair pubkey for both.
    /// `write_mode` is the per-request user intent (`Local` / `Participate`)
    /// resolved at the MCP entry point. It is persisted on the row so `verify`
    /// can route by stored intent (T4) and `recall` can surface it back to the
    /// caller. Internal/legacy callsites that pre-date this parameter pass
    /// `WriteMode::Participate` to preserve their previous "real anchor"
    /// semantics.
    /// `visibility` is the per-request sharing intent (`Private` / `Public`)
    /// resolved at the MCP entry point. Local-mode writes are always
    /// `Private` (the resolver rejects explicit visibility on local mode —
    /// AC14). Anonymous recall (Decision 5) filters server-side to
    /// `Visibility::Public`; authenticated callers see all their own rows.
    #[allow(clippy::too_many_arguments)]
    fn save_attestation(
        &self,
        attestation_id: &str,
        content: &str,
        content_hash: &str,
        tags: &[String],
        solana_tx: &str,
        arweave_tx: &str,
        signer_pubkey: &str,
        owner_pubkey: &str,
        created_at: &str,
        write_mode: WriteMode,
        visibility: Visibility,
        embedding: &[f32],
    ) -> anyhow::Result<()>;

    /// Look up a row by `solana_tx` OR `arweave_tx`, scoped to the caller's
    /// `owner_pubkey`. A row belonging to a different tenant returns
    /// `Ok(None)` — same shape as a genuine miss, no side-channel leak of
    /// existence, hash, signer, or content. Closes the pre-existing tenant-
    /// isolation gap surfaced by per-request mode coexistence (Decision 9 /
    /// T4).
    fn find_by_tx(&self, tx_id: &str, owner_pubkey: &str)
        -> anyhow::Result<Option<AttestationRow>>;

    /// Look up the stored `write_mode` for a row by `solana_tx` OR
    /// `arweave_tx`, scoped to the caller's `owner_pubkey`. Used by
    /// `verify` routing in `mcp/` to dispatch by stored intent without
    /// re-loading the full row. Wrong-tenant lookups return `Ok(None)`
    /// identical to a genuine miss.
    fn find_write_mode_by_tx(
        &self,
        tx_id: &str,
        owner_pubkey: &str,
    ) -> anyhow::Result<Option<WriteMode>>;

    fn count(&self, signer: &str) -> anyhow::Result<i64>;

    /// Wipe the previously-seeded protocol-knowledge corpus for `owner_pubkey`.
    ///
    /// Called by `mcp/src/seed.rs::run` before a forced reseed (`MNEMONIC_FORCE_RESEED=1`
    /// or detected stale artifact) so the new chunks don't pile up alongside an
    /// outdated copy. Matches rows where `owner_pubkey = ?` AND the JSON `tags`
    /// column contains `"protocol-knowledge"` — the canonical tag every seeded
    /// chunk carries (`mcp/src/seed.rs:313`).
    ///
    /// User-signed attestations (`owner_pubkey` = some OAuth pubkey, NOT the
    /// server's signing pubkey) are never matched: those rows have an OAuth
    /// `owner_pubkey`, while the seeded corpus has `owner_pubkey = server_pubkey`.
    /// Returns the number of rows deleted; default impl returns `0` for stores
    /// that don't seed (e.g. test fakes).
    fn delete_protocol_knowledge_for_owner(&self, _owner_pubkey: &str) -> anyhow::Result<usize> {
        Ok(0)
    }

    /// Cosine-similarity search.
    ///
    /// `owner_pubkey` — `Some(pk)` scopes results to a single owner (the
    /// authenticated-recall path, Decision 9 tenant boundary). `None` drops
    /// the owner predicate entirely so results span every owner; this is the
    /// cross-owner anonymous-public path (agent-native-distribution Task 4 /
    /// SAR1-M1). Callers MUST pair `None` with
    /// `visibility_filter = Some(Visibility::Public)` — passing `None`
    /// without the visibility filter would expose every row in the database
    /// to an anonymous caller.
    ///
    /// `visibility_filter` — when `Some(v)`, restricts results to rows whose
    /// stored `visibility` equals `v`. Authenticated callers (with
    /// `owner_pubkey = Some(pk)`) pass `None` to see all their own rows;
    /// anonymous callers (with `owner_pubkey = None`) pass
    /// `Some(Visibility::Public)` to discover the cross-owner public pool.
    fn search(
        &self,
        query_embedding: &[f32],
        owner_pubkey: Option<&str>,
        visibility_filter: Option<Visibility>,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>>;
}

/// Minimal storage-level lineage operations.
///
/// Authorization is the caller's responsibility. These methods operate on the
/// full database and do not enforce per-signer scoping.
pub trait LineageStore {
    fn save_edge(&self, parent_id: &str, child_id: &str, depth: i64) -> anyhow::Result<()>;

    fn get_edges(&self, child_id: &str) -> anyhow::Result<Vec<(String, i64)>>;

    fn clear_edges(&self, artifact_id: &str) -> anyhow::Result<()>;

    // TODO(task-9): add get_lineage, get_ancestry, validate_chain methods after lineage types are in core
}

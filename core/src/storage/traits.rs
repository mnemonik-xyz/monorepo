//! Storage trait definitions for attestation persistence.

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
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub tags: Vec<String>,
    pub solana_tx: String,
    pub arweave_tx: String,
    pub created_at: String,
    pub relevance_score: f32,
}

/// Attestation CRUD and cosine search.
///
/// Authorization is the caller's responsibility. These methods operate on the
/// full database and do not enforce per-signer scoping.
pub trait AttestationStore {
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
        created_at: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()>;

    fn find_by_tx(&self, tx_id: &str) -> anyhow::Result<Option<AttestationRow>>;

    fn count(&self, signer: &str) -> anyhow::Result<i64>;

    fn search(
        &self,
        query_embedding: &[f32],
        signer: &str,
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

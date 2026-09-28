//! Recall A2A attestations by context ID.
//!
//! This module is a thin public-API wrapper around
//! `AttestationStore::recall_by_context` so downstream consumers never need to
//! import `mnemonic_core::storage` directly.

use mnemonic_core::storage::traits::AttestationStore;

use crate::Attestation;

/// Return all attestation rows whose `context_id` matches `ctx_id`, newest
/// first.
///
/// # Arguments
/// * `store` — Any [`mnemonic_core::storage::traits::AttestationStore`]
///   implementation.
/// * `ctx_id` — The A2A `contextId` string to query.
/// * `limit` — Optional cap on the number of returned rows; `None` returns
///   all matching rows.
///
/// # Returns
/// Rows ordered by `created_at DESC` (newest first). Returns an empty `Vec`
/// when no rows match — this is not an error.
pub fn recall_by_context<S: AttestationStore>(
    store: &S,
    ctx_id: &str,
    limit: Option<usize>,
) -> anyhow::Result<Vec<Attestation>> {
    store.recall_by_context(ctx_id, limit)
}

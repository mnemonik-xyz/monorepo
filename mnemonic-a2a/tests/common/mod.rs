//! Shared test helpers for `mnemonic-a2a` integration tests.
//!
//! Provides `InMemoryA2aStore` — an in-memory implementation of both
//! `AttestationStore` and `A2aStore` for use in integration tests.

use std::collections::HashMap;
use std::sync::Mutex;

use mnemonic_core::storage::traits::{AttestationRow, AttestationStore, ReconstructionInputs};
use mnemonic_core::storage::{Visibility, WriteMode};
use mnemonic_core::storage::SearchResult;
use mnemonic_a2a::A2aStore;

/// Row stored in `InMemoryA2aStore`.
#[derive(Clone)]
struct Row {
    attestation_id: String,
    content: String,
    content_hash: String,
    solana_tx: String,
    arweave_tx: String,
    signer_pubkey: String,
    context_id: Option<String>,
    created_at: String,
}

/// Thread-safe in-memory attestation store used by unit tests.
///
/// Implements `AttestationStore` (minimal surface) and `A2aStore`
/// (`set_context_id`). `recall_by_context` scans the in-memory list for rows
/// whose `context_id` matches.
pub struct InMemoryA2aStore {
    rows: Mutex<HashMap<String, Row>>,
}

impl InMemoryA2aStore {
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryA2aStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AttestationStore for InMemoryA2aStore {
    fn save_attestation(
        &self,
        attestation_id: &str,
        content: &str,
        content_hash: &str,
        _tags: &[String],
        solana_tx: &str,
        arweave_tx: &str,
        signer_pubkey: &str,
        _owner_pubkey: &str,
        created_at: &str,
        _write_mode: WriteMode,
        _visibility: Visibility,
        _embedding: &[f32],
    ) -> anyhow::Result<()> {
        let mut map = self.rows.lock().unwrap();
        // Preserve existing context_id on UPDATE (INSERT OR REPLACE behaviour).
        let existing_ctx = map
            .get(attestation_id)
            .and_then(|r| r.context_id.clone());
        map.insert(
            attestation_id.to_string(),
            Row {
                attestation_id: attestation_id.to_string(),
                content: content.to_string(),
                content_hash: content_hash.to_string(),
                solana_tx: solana_tx.to_string(),
                arweave_tx: arweave_tx.to_string(),
                signer_pubkey: signer_pubkey.to_string(),
                context_id: existing_ctx,
                created_at: created_at.to_string(),
            },
        );
        Ok(())
    }

    fn find_by_tx(
        &self,
        tx_id: &str,
        _owner_pubkey: &str,
    ) -> anyhow::Result<Option<AttestationRow>> {
        let map = self.rows.lock().unwrap();
        for row in map.values() {
            if row.solana_tx == tx_id || row.arweave_tx == tx_id {
                return Ok(Some(AttestationRow {
                    attestation_id: row.attestation_id.clone(),
                    content: row.content.clone(),
                    content_hash: row.content_hash.clone(),
                    solana_tx: row.solana_tx.clone(),
                    arweave_tx: row.arweave_tx.clone(),
                    signer_pubkey: row.signer_pubkey.clone(),
                }));
            }
        }
        Ok(None)
    }

    fn reconstruction_inputs_by_tx(
        &self,
        _tx_id: &str,
        _owner_pubkey: &str,
    ) -> anyhow::Result<Option<ReconstructionInputs>> {
        Ok(None)
    }

    fn find_write_mode_by_tx(
        &self,
        _tx_id: &str,
        _owner_pubkey: &str,
    ) -> anyhow::Result<Option<WriteMode>> {
        Ok(None)
    }

    fn count(&self, _signer: &str) -> anyhow::Result<i64> {
        Ok(self.rows.lock().unwrap().len() as i64)
    }

    fn search(
        &self,
        _query_embedding: &[f32],
        _owner_pubkey: Option<&str>,
        _visibility_filter: Option<Visibility>,
        _limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        Ok(Vec::new())
    }

    fn recall_by_context(
        &self,
        context_id: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<AttestationRow>> {
        let map = self.rows.lock().unwrap();
        let mut out: Vec<&Row> = map
            .values()
            .filter(|r| r.context_id.as_deref() == Some(context_id))
            .collect();

        // Sort newest-first by created_at (lexicographic ISO-8601 sort is correct).
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        if let Some(n) = limit {
            out.truncate(n);
        }

        Ok(out
            .into_iter()
            .map(|r| AttestationRow {
                attestation_id: r.attestation_id.clone(),
                content: r.content.clone(),
                content_hash: r.content_hash.clone(),
                solana_tx: r.solana_tx.clone(),
                arweave_tx: r.arweave_tx.clone(),
                signer_pubkey: r.signer_pubkey.clone(),
            })
            .collect())
    }
}

impl A2aStore for InMemoryA2aStore {
    fn set_context_id(&self, attestation_id: &str, context_id: &str) -> anyhow::Result<()> {
        let mut map = self.rows.lock().unwrap();
        if let Some(row) = map.get_mut(attestation_id) {
            row.context_id = Some(context_id.to_string());
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "set_context_id: attestation_id '{}' not found",
                attestation_id
            ))
        }
    }
}

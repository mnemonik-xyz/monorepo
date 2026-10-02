//! Shared test helpers for bridge-a2a integration tests.

use std::collections::HashMap;
use std::sync::Mutex;

use mnemonic_a2a::A2aStore;
use mnemonic_core::storage::traits::{AttestationRow, AttestationStore, ReconstructionInputs};
use mnemonic_core::storage::{SearchResult, Visibility, WriteMode};

/// Thread-safe in-memory attestation store for tests.
#[derive(Default)]
pub struct InMemoryA2aStore {
    rows: Mutex<HashMap<String, InMemRow>>,
    /// When set to true, `save_attestation` returns an error.
    pub fail_writes: Mutex<bool>,
}

#[derive(Clone)]
struct InMemRow {
    attestation_id: String,
    content: String,
    content_hash: String,
    solana_tx: String,
    arweave_tx: String,
    signer_pubkey: String,
    context_id: Option<String>,
    created_at: String,
}

impl InMemoryA2aStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fail_writes(&self, fail: bool) {
        *self.fail_writes.lock().unwrap() = fail;
    }

    pub fn row_count(&self) -> usize {
        self.rows.lock().unwrap().len()
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
        if *self.fail_writes.lock().unwrap() {
            return Err(anyhow::anyhow!("injected write failure"));
        }
        let mut map = self.rows.lock().unwrap();
        let existing_ctx = map.get(attestation_id).and_then(|r| r.context_id.clone());
        map.insert(
            attestation_id.to_string(),
            InMemRow {
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
        let mut out: Vec<&InMemRow> = map
            .values()
            .filter(|r| r.context_id.as_deref() == Some(context_id))
            .collect();
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
        if *self.fail_writes.lock().unwrap() {
            return Err(anyhow::anyhow!("injected write failure (set_context_id)"));
        }
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

//! Non-custodial ingestion and recipient-scoped recall.
use anyhow::{ensure, Result};
use mnemonic_core::codec::a2a::signed::verify_signed_a2a;
use mnemonic_core::storage::SqliteStore;
use serde_json::Value;

pub fn ingest_signed_a2a(
    store: &SqliteStore,
    signed: &[u8],
    owner: &str,
    kind: &str,
    context: &str,
    sealed: bool,
    prev_id: Option<&str>,
) -> Result<Value> {
    let verified = validate_signed_a2a(signed, owner, kind, context, sealed, prev_id)?;
    store.save_signed_a2a(signed, &verified)
}

pub fn validate_signed_a2a(
    signed: &[u8],
    owner: &str,
    kind: &str,
    context: &str,
    sealed: bool,
    prev_id: Option<&str>,
) -> Result<mnemonic_core::codec::a2a::signed::VerifiedBinding> {
    let verified = verify_signed_a2a(signed, Some(owner))?;
    ensure!(
        verified.binding.kind == kind
            && verified.binding.context_id == context
            && verified.binding.sealed == sealed
            && verified.binding.prev_id.as_deref() == prev_id,
        "tool arguments do not match signed binding"
    );
    Ok(verified)
}

pub fn recall_signed_a2a(
    store: &SqliteStore,
    owner: &str,
    context: &str,
    kind: Option<&str>,
    sealed: Option<bool>,
    limit: usize,
) -> Result<Vec<Value>> {
    store.recall_signed_a2a(owner, context, kind, sealed, limit)
}

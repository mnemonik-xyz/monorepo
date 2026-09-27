//! Restore a recall index from Arweave (work/arweave-as-source-of-truth).
//!
//! This is the other half of [`crate::rebuild`]. `rebuild` turns one signed
//! artifact back into a row; this module finds *which* artifacts belong to an
//! identity, fetches them, and applies the verified rows to a store. Together
//! they make the operator's database disposable: lose it, or leave the operator
//! entirely, and the memories are still recoverable from the chain by whoever
//! holds the key.
//!
//! ## Two phases, on purpose
//!
//! [`fetch_restorable`] does all the network work and touches no store.
//! [`apply_restore`] does all the store work and performs no `await`. The split
//! is not stylistic: `rusqlite::Connection` is `!Send`, and the project rule is
//! that the store mutex is never held across an `.await`. Keeping the phases in
//! separate functions makes that impossible to get wrong here, and it makes
//! `apply_restore` testable without any network.
//!
//! ## Enumeration
//!
//! Two independent sources, unioned by `arweave_tx`, because neither is
//! complete on its own:
//!
//! - **Solana memo history.** Authoritative for historical items.
//!   `core/src/arweave/recovery.rs` records a live check from 2026-07-09: 16
//!   memos and 0 gateway-GraphQL hits, because gateways never indexed the old
//!   Irys-bundled items.
//! - **Gateway GraphQL.** Catches items whose memo write failed after the
//!   upload, and everything tagged going forward.
//!
//! ## Trust
//!
//! Every artifact is COSE-verified before any field is read (inside
//! [`crate::rebuild::rebuild_row_self_describing`]), so a hostile gateway can
//! withhold or corrupt items but can never inject one. A corrupt item is
//! reported and skipped rather than aborting the restore, so one bad blob
//! cannot deny recovery of everything else.

use crate::arweave::graphql::GraphQlClient;
use crate::arweave::ArweaveClient;
use crate::rebuild::{rebuild_row_self_describing, RebuiltRow};
use crate::solana::SolanaClient;

/// One anchored artifact, fetched and verified, ready to be written to a store.
#[derive(Debug, Clone)]
pub struct RestorableItem {
    pub arweave_tx: String,
    /// The Solana memo transaction, when enumeration found one. GraphQL-only
    /// items may have none, which does not make the memory less valid — the
    /// COSE signature is what proves authorship.
    pub solana_tx: Option<String>,
    pub row: RebuiltRow,
}

/// What a restore did. Reported rather than logged, so a CLI can print it and a
/// test can assert on it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RestoreReport {
    /// Distinct `arweave_tx` ids found by enumeration.
    pub enumerated: usize,
    /// Rows written to the store.
    pub restored: usize,
    /// Verified rows belonging to a different identity, left alone.
    pub skipped_other_owner: usize,
    /// `(arweave_tx, reason)` for each item that could not be fetched, verified
    /// or written. Never fatal.
    pub failed: Vec<(String, String)>,
}

/// Collect every `arweave_tx` this wallet ever anchored, from both sources.
///
/// `solana_wallet` is the fee-payer that wrote the memos; `arweave_addresses`
/// are the gateway owner addresses to enumerate. A failure of either source is
/// tolerated: the other still returns items, and a restore from one source is
/// better than no restore. Both failing yields an empty list, not an error, so
/// the caller can distinguish "nothing anchored" from "could not reach the
/// network" by looking at `report.failed`.
pub async fn enumerate_anchored(
    gql: &GraphQlClient,
    solana: &SolanaClient,
    solana_wallet: &str,
    arweave_addresses: &[String],
) -> Vec<(String, Option<String>)> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(String, Option<String>)> = Vec::new();

    // Memo history first: it carries the Solana tx, which the gateway cannot
    // know, and it is the authoritative source for legacy items.
    if let Ok(anchors) = solana.list_memo_anchors(solana_wallet).await {
        for a in anchors {
            if seen.insert(a.arweave_tx.clone()) {
                out.push((a.arweave_tx, Some(a.solana_tx)));
            }
        }
    }
    if let Ok(items) = gql.list_anchored(arweave_addresses).await {
        for i in items {
            if seen.insert(i.arweave_tx.clone()) {
                out.push((i.arweave_tx, None));
            }
        }
    }
    out
}

/// Fetch and verify every enumerated item. Network only — no store access.
///
/// Items that fail to fetch or verify land in `failed` and are skipped.
pub async fn fetch_restorable(
    gateway: &ArweaveClient,
    enumerated: &[(String, Option<String>)],
) -> (Vec<RestorableItem>, Vec<(String, String)>) {
    let mut items = Vec::new();
    let mut failed = Vec::new();
    for (arweave_tx, solana_tx) in enumerated {
        let bytes = match gateway.read(arweave_tx).await {
            Ok(b) => b,
            Err(e) => {
                failed.push((arweave_tx.clone(), format!("fetch failed: {e}")));
                continue;
            }
        };
        match rebuild_row_self_describing(&bytes) {
            Ok(row) => items.push(RestorableItem {
                arweave_tx: arweave_tx.clone(),
                solana_tx: solana_tx.clone(),
                row,
            }),
            Err(e) => failed.push((arweave_tx.clone(), e)),
        }
    }
    (items, failed)
}

/// Write verified rows belonging to `owner_pubkey` into the store.
///
/// Synchronous by design: no `await` happens here, so a caller may hold the
/// store mutex for the whole call.
///
/// Only rows whose author matches `owner_pubkey` are written. A restore is
/// recovery of *your* memories, and the enumeration sources are wallet-scoped,
/// not identity-scoped: the operator's fee-payer wallet anchored items for many
/// identities, so a naive restore would import other people's rows. They are
/// counted in `skipped_other_owner` rather than silently dropped.
///
/// Idempotent. `save_attestation` is an `INSERT OR REPLACE` keyed by
/// `attestation_id`, and `attestation_id` comes from the signed payload, so
/// restoring twice converges instead of duplicating.
///
/// `visibility` comes from the signed artifact when present. A legacy artifact
/// has none, and then the row is stored `Private`. That direction is deliberate:
/// mislabelling a public memory as private hides something that was meant to be
/// shared, which the owner can correct, while the opposite would publish
/// something that was not meant to be. Never invert this default.
pub fn apply_restore(
    store: &dyn crate::storage::AttestationStore,
    owner_pubkey: &str,
    items: &[RestorableItem],
) -> RestoreReport {
    use crate::storage::{Visibility, WriteMode};

    let mut report = RestoreReport {
        enumerated: items.len(),
        ..Default::default()
    };

    for item in items {
        if item.row.owner_pubkey != owner_pubkey {
            report.skipped_other_owner += 1;
            continue;
        }

        let visibility = match item.row.visibility.as_deref() {
            Some(v) => match Visibility::from_str_strict(v) {
                Some(v) => v,
                None => {
                    report.failed.push((
                        item.arweave_tx.clone(),
                        format!("artifact declares an unknown visibility {v:?}"),
                    ));
                    continue;
                }
            },
            None => Visibility::Private,
        };

        // An empty `solana_tx` is the existing convention for "no memo id"
        // (see `RowFact` in core/src/storage/sqlite.rs). The COSE signature,
        // not the memo, is what proves authorship.
        let solana_tx = item.solana_tx.clone().unwrap_or_default();

        match store.save_attestation(
            &item.row.attestation_id,
            &item.row.content,
            &item.row.content_hash,
            &item.row.tags,
            &solana_tx,
            &item.arweave_tx,
            &item.row.signer_pubkey,
            &item.row.owner_pubkey,
            &item.row.created_at,
            WriteMode::Anchored,
            visibility,
            &item.row.embedding,
        ) {
            Ok(()) => report.restored += 1,
            Err(e) => report
                .failed
                .push((item.arweave_tx.clone(), format!("store write failed: {e}"))),
        }
    }

    report
}

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

/// Results retain verified discovery hints even when a later page fails.
#[derive(Debug, Clone)]
pub struct SourceScan<T> {
    pub items: Vec<T>,
    pub exhausted: bool,
    pub budget_exhausted: bool,
    pub error: Option<String>,
}
impl<T> Default for SourceScan<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            exhausted: false,
            budget_exhausted: false,
            error: None,
        }
    }
}
/// Independent source status. Exhaustion is not proof of global completeness.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SourceDiagnostic {
    pub source: &'static str,
    pub status: &'static str,
    pub count: usize,
    pub error: Option<String>,
}
impl<T> SourceScan<T> {
    fn diagnostic(&self, source: &'static str) -> SourceDiagnostic {
        SourceDiagnostic {
            source,
            status: if self.exhausted {
                "exhausted"
            } else if self.budget_exhausted {
                "budget_exhausted"
            } else if self.items.is_empty() {
                "failed"
            } else {
                "partial"
            },
            count: self.items.len(),
            error: self.error.clone(),
        }
    }
}
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct EnumerationReport {
    pub items: Vec<(String, Option<String>)>,
    pub sources: Vec<SourceDiagnostic>,
}
/// Preserve successful candidates and every source error independently.
pub async fn enumerate_anchored(
    gql: &GraphQlClient,
    solana: &SolanaClient,
    solana_wallet: &str,
    arweave_addresses: &[String],
) -> EnumerationReport {
    let mut report = EnumerationReport::default();
    let mut seen = std::collections::HashSet::new();
    let memos = solana.scan_memo_anchors(solana_wallet, 1000).await;
    report.sources.push(memos.diagnostic("solana"));
    for a in memos.items {
        if seen.insert(a.arweave_tx.clone()) {
            report.items.push((a.arweave_tx, Some(a.solana_tx)));
        }
    }
    let index = gql.scan_anchored(arweave_addresses, 10000).await;
    report.sources.push(index.diagnostic("graphql"));
    for i in index.items {
        if seen.insert(i.arweave_tx.clone()) {
            report.items.push((i.arweave_tx, None));
        }
    }
    report
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
        // Kind selection follows signature verification. This legacy index
        // command has no decryption key; sealed callers use fetch_recovered_memories.
        let verified = match crate::codec::sign::verify_artifact(&bytes, None) {
            Ok(v) if v.valid => v,
            _ => {
                failed.push((
                    arweave_tx.clone(),
                    "invalid_envelope: signature verification failed".into(),
                ));
                continue;
            }
        };
        let artifact = match crate::codec::canonical::from_canonical_cbor(&verified.payload) {
            Ok(v) => v,
            Err(e) => {
                failed.push((arweave_tx.clone(), format!("malformed_payload: {e}")));
                continue;
            }
        };
        match (
            artifact["type"].as_str(),
            artifact["schema_version"].as_u64(),
        ) {
            (Some("memory"), Some(1)) => {}
            (Some("sealed"), Some(1)) => {
                failed.push((
                    arweave_tx.clone(),
                    "key_required: use client-local sealed recovery".into(),
                ));
                continue;
            }
            _ => {
                failed.push((
                    arweave_tx.clone(),
                    "unsupported_kind: legacy index supports memory.v1".into(),
                ));
                continue;
            }
        }
        if artifact["producer"].as_str() != Some(format!("did:sol:{}", verified.signer).as_str()) {
            failed.push((
                arweave_tx.clone(),
                "author_binding: producer differs from signer".into(),
            ));
            continue;
        }
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

/// Fetch known locators with an independently pinned author per item. Network
/// or malformed candidates cannot suppress successfully recovered memories.
/// Complete plaintext stays in the caller's local process.
pub async fn fetch_recovered_memories(
    gateway: &ArweaveClient,
    candidates: &[(String, String)],
    x25519_secret: Option<&[u8; 32]>,
) -> (
    Vec<crate::rebuild::RecoveredMemory>,
    Vec<(String, crate::rebuild::RecoveryError)>,
) {
    let mut items = Vec::new();
    let mut failed = Vec::new();
    for (locator, author) in candidates {
        // The same bounded, fixed-origin ANS-104 read applies to memory bytes.
        let bytes = match gateway.read_a2a(locator).await {
            Ok(bytes) => bytes,
            Err(e) => {
                failed.push((
                    locator.clone(),
                    crate::rebuild::RecoveryError {
                        code: "fetch_failed",
                        message: e.to_string(),
                    },
                ));
                continue;
            }
        };
        match crate::rebuild::recover_memory(&bytes, author, x25519_secret) {
            Ok(item) => items.push(item),
            Err(e) => failed.push((locator.clone(), e)),
        }
    }
    (items, failed)
}

//! Chain-backed traction stats (recover-traction-from-chain).
//!
//! After the server migration lost the SQLite database, the on-chain record
//! is the only surviving history: every `anchored` write is an Arweave
//! data item signed by the server wallet and tagged `App-Name:
//! mnemonic-protocol`. This module periodically snapshots that record via
//! gateway GraphQL and merges it with whatever the (fresh) local DB holds,
//! so `/stats` and `/analytics/attestations` show true lifetime numbers:
//!
//! - **anchored** — union of chain items and DB `anchored` rows, deduped
//!   by `arweave_tx` (a new write appears in the DB before the gateway
//!   indexes it; the union keeps the count exact in both directions).
//! - **users** — distinct chain producers (normalized `did:sol:` → sub)
//!   ∪ distinct DB owners. Local-only users that existed solely in the lost
//!   DB are gone — the merge cannot invent them.
//! - **on-node** — anchored + DB local-only rows.
//!
//! Enabled by setting `CHAIN_STATS_WALLETS` (comma-separated base58 Solana
//! pubkeys of every wallet that ever paid for anchoring). Disabled = the
//! endpoints keep their DB-only behaviour.

use mnemonic_core::arweave::graphql::{solana_pubkey_to_arweave_address, GraphQlClient};
use mnemonic_core::arweave::recovery::{
    normalize_producer, snapshot_chain, RecoveredEmbedding, RecoveredItem,
};
use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::compress::{CompressedEmbedding, EmbeddingCompressor};
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::sqlite::{RowFact, TimelineBucket};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, OnceLock};

pub struct ChainStatsCache {
    gql: GraphQlClient,
    gateway: ArweaveClient,
    solana: SolanaClient,
    /// Base58 Solana pubkeys — enumeration source 1 (memo history).
    wallets: Vec<String>,
    /// The same wallets in Arweave owner form — enumeration source 2
    /// (gateway GraphQL, for indexed/tagged items).
    owner_addresses: Vec<String>,
    snapshot: tokio::sync::RwLock<Option<Arc<ChainLedger>>>,
}

impl ChainStatsCache {
    /// `None` when no wallets are configured (feature off). Wallets that
    /// fail base58 decoding are rejected loudly — a typo'd wallet silently
    /// shrinking the traction numbers is worse than a startup error.
    pub fn new(
        wallets: &[String],
        graphql_url: &str,
        gateway_url: &str,
        solana_rpc_url: &str,
    ) -> anyhow::Result<Option<Self>> {
        if wallets.is_empty() {
            return Ok(None);
        }
        let owner_addresses = wallets
            .iter()
            .map(|w| solana_pubkey_to_arweave_address(w))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(Self {
            gql: GraphQlClient::new(graphql_url),
            gateway: ArweaveClient::new(gateway_url),
            solana: SolanaClient::new(solana_rpc_url),
            wallets: wallets.to_vec(),
            owner_addresses,
            snapshot: tokio::sync::RwLock::new(None),
        }))
    }

    /// Re-enumerate the chain. Errors are returned (caller logs and keeps
    /// the previous snapshot — a gateway outage must not zero the page).
    ///
    /// The Solana memo history is the primary source: gateways do not list
    /// every historical item, so GraphQL alone can return zero for them
    /// (verified live 2026-07-09 — 16 memos, 0 GraphQL hits). GraphQL still runs to catch memo-less uploads and future
    /// tagged items.
    pub async fn refresh(&self) -> anyhow::Result<usize> {
        let mut anchors = Vec::new();
        for wallet in &self.wallets {
            anchors.extend(self.solana.list_memo_anchors(wallet).await?);
        }
        let snap =
            snapshot_chain(&self.gql, &self.gateway, &self.owner_addresses, &anchors).await?;
        let n = snap.items.len();
        *self.snapshot.write().await = Some(Arc::new(ChainLedger::new(snap.items)));
        Ok(n)
    }

    /// Latest snapshot, or `None` until the first successful refresh.
    pub async fn items(&self) -> Option<Vec<RecoveredItem>> {
        self.snapshot
            .read()
            .await
            .as_ref()
            .map(|ledger| ledger.items().to_vec())
    }

    /// Latest snapshot with its recall index, or `None` until the first
    /// successful refresh. Cheap (`Arc` clone); used by `?q=` recall.
    pub async fn ledger(&self) -> Option<Arc<ChainLedger>> {
        self.snapshot.read().await.clone()
    }

    /// Cache pre-loaded with `items`, for tests that exercise the chain
    /// merge and recall paths without a Solana / Arweave backend.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub fn from_items(items: Vec<RecoveredItem>) -> Self {
        Self {
            gql: GraphQlClient::new("http://localhost:0/graphql"),
            gateway: ArweaveClient::new("http://localhost:0"),
            solana: SolanaClient::new("http://localhost:0"),
            wallets: Vec::new(),
            owner_addresses: Vec::new(),
            snapshot: tokio::sync::RwLock::new(Some(Arc::new(ChainLedger::new(items)))),
        }
    }
}

// ── Chain recall (#201) ───────────────────────────────────────────────────────

/// Score a literal text match gets when a recovered item has no usable
/// embedding. Equal to a perfect cosine match, so a memory that literally
/// contains the query is never pushed out of the result page by weaker
/// semantic neighbours.
pub const TEXT_MATCH_SCORE: f32 = 1.0;

/// How a recall hit matched the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    /// Cosine similarity between the query and the memory embedding.
    Semantic,
    /// Case-insensitive substring match on content or tags (fallback for
    /// items whose embedding is missing or not decodable by this server).
    Text,
}

/// One chain-recovered memory that matched a recall query.
#[derive(Debug, Clone)]
pub struct ChainHit<'a> {
    pub item: &'a RecoveredItem,
    pub score: f32,
    pub kind: MatchKind,
}

/// One chain snapshot plus a disposable in-memory recall index over it.
///
/// The index holds unit-length f32 embeddings decoded from each item's
/// signed payload. It is built on the first query and dropped with the
/// snapshot on the next refresh, so it is rebuilt from the external
/// artifacts alone — SQLite is never read or written for chain recall.
pub struct ChainLedger {
    items: Vec<RecoveredItem>,
    recall_index: OnceLock<Vec<Option<Vec<f32>>>>,
}

impl ChainLedger {
    pub fn new(items: Vec<RecoveredItem>) -> Self {
        Self {
            items,
            recall_index: OnceLock::new(),
        }
    }

    pub fn items(&self) -> &[RecoveredItem] {
        &self.items
    }

    /// Search the snapshot for `query_text` / `query_emb`.
    ///
    /// Items with an embedding decodable by `compressor` (same dimension and
    /// bit width as this server's) are scored by cosine similarity, like the
    /// SQLite recall path, so every such item is a candidate. Items without
    /// one match only when the query text occurs in their content or tags,
    /// and score [`TEXT_MATCH_SCORE`]. Hits come back unsorted; the caller
    /// ranks them together with SQLite hits.
    pub fn recall(
        &self,
        query_text: &str,
        query_emb: &[f32],
        compressor: &EmbeddingCompressor,
    ) -> Vec<ChainHit<'_>> {
        let index = self
            .recall_index
            .get_or_init(|| build_recall_index(&self.items, compressor));
        let query = unit(query_emb);
        let needle = query_text.trim().to_lowercase();
        let mut hits = Vec::new();
        for (item, emb) in self.items.iter().zip(index) {
            if let (Some(q), Some(e)) = (query.as_deref(), emb.as_deref()) {
                if q.len() == e.len() {
                    let score: f32 = q.iter().zip(e).map(|(a, b)| a * b).sum();
                    if score.is_finite() {
                        hits.push(ChainHit {
                            item,
                            score,
                            kind: MatchKind::Semantic,
                        });
                        continue;
                    }
                }
            }
            if !needle.is_empty() && text_matches(item, &needle) {
                hits.push(ChainHit {
                    item,
                    score: TEXT_MATCH_SCORE,
                    kind: MatchKind::Text,
                });
            }
        }
        hits
    }
}

fn text_matches(item: &RecoveredItem, needle_lower: &str) -> bool {
    item.content
        .as_deref()
        .is_some_and(|c| c.to_lowercase().contains(needle_lower))
        || item
            .tags
            .iter()
            .any(|t| t.to_lowercase().contains(needle_lower))
}

/// Scale `v` to unit length. `None` for an empty, zero or non-finite vector.
fn unit(v: &[f32]) -> Option<Vec<f32>> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (norm.is_finite() && norm > 0.0).then(|| v.iter().map(|x| x / norm).collect())
}

fn build_recall_index(
    items: &[RecoveredItem],
    compressor: &EmbeddingCompressor,
) -> Vec<Option<Vec<f32>>> {
    // TurboQuant's dequantize asserts on shape mismatches, so a compressed
    // blob is decoded only when its dimension, bit width and packed lengths
    // equal what this compressor itself produces.
    let probe = compressor.compress(&vec![1.0; compressor.dim()]);
    items
        .iter()
        .map(|item| {
            let raw = match item.embedding.as_ref()? {
                RecoveredEmbedding::F32(v) => v.clone(),
                RecoveredEmbedding::Compressed(bytes) => {
                    let c = CompressedEmbedding::from_bytes(bytes)?;
                    let same_shape = c.dim == probe.dim
                        && c.bit_width == probe.bit_width
                        && c.mse_indices_packed.len() == probe.mse_indices_packed.len()
                        && c.qjl_signs_packed.len() == probe.qjl_signs_packed.len();
                    if !same_shape {
                        return None;
                    }
                    compressor.decompress(&c)
                }
            };
            unit(&raw)
        })
        .collect()
}

/// All-time merged traction numbers plus the daily timeline.
#[derive(Debug, Clone)]
pub struct MergedStats {
    pub unique_users: i64,
    pub saved_on_node: i64,
    pub saved_onchain: i64,
    /// Sparse daily buckets, ascending by date, spanning all time. The
    /// analytics handler filters by range and sums per-range totals.
    pub buckets: Vec<TimelineBucket>,
}

/// Pure merge of a chain snapshot with the local DB rows. See the module
/// docs for the union/dedup semantics. Day attribution for anchored items
/// prefers the DB's exact `created_at` day over the (later) block day.
pub fn merge_stats(chain: &[RecoveredItem], db: &[RowFact]) -> MergedStats {
    // arweave_tx → best-known day (None = pending block and not in DB).
    let mut anchored_days: HashMap<&str, Option<&str>> = chain
        .iter()
        .map(|c| (c.arweave_tx.as_str(), c.day.as_deref()))
        .collect();

    let mut users: HashSet<String> = chain
        .iter()
        .filter_map(|c| c.producer.as_deref().map(normalize_producer))
        .collect();

    let mut local_days: Vec<&str> = Vec::new();
    for row in db {
        if let Some(owner) = &row.owner_pubkey {
            users.insert(owner.clone());
        }
        let real_arweave = !row.arweave_tx.is_empty() && !row.arweave_tx.starts_with("local:");
        if row.write_mode == "anchored" && real_arweave {
            // DB day wins: exact write time vs. eventual block time.
            anchored_days.insert(row.arweave_tx.as_str(), Some(row.day.as_str()));
        } else {
            local_days.push(row.day.as_str());
        }
    }

    let saved_onchain = anchored_days.len() as i64;
    let saved_on_node = saved_onchain + local_days.len() as i64;

    let mut day_buckets: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for day in anchored_days.values().flatten() {
        day_buckets.entry(day.to_string()).or_default().1 += 1;
    }
    for day in local_days {
        day_buckets.entry(day.to_string()).or_default().0 += 1;
    }
    let buckets = day_buckets
        .into_iter()
        .map(|(date, (on_node, on_chain))| TimelineBucket {
            date,
            on_node,
            on_chain,
        })
        .collect();

    MergedStats {
        unique_users: users.len() as i64,
        saved_on_node,
        saved_onchain,
        buckets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_item(tx: &str, day: Option<&str>, producer: Option<&str>) -> RecoveredItem {
        RecoveredItem {
            arweave_tx: tx.to_string(),
            solana_tx: None,
            content_hash: None,
            content: None,
            tags: Vec::new(),
            day: day.map(str::to_string),
            producer: producer.map(str::to_string),
            embedding: None,
        }
    }

    fn db_row(arweave_tx: &str, owner: Option<&str>, day: &str, mode: &str) -> RowFact {
        RowFact {
            arweave_tx: arweave_tx.to_string(),
            owner_pubkey: owner.map(str::to_string),
            day: day.to_string(),
            write_mode: mode.to_string(),
        }
    }

    #[test]
    fn chain_only_recovers_all_anchored_history() {
        // The post-migration scenario: empty DB, three historic anchors.
        let chain = vec![
            chain_item("tx1", Some("2026-06-01"), Some("did:sol:alice")),
            chain_item("tx2", Some("2026-06-01"), Some("did:sol:bob")),
            chain_item("tx3", Some("2026-06-02"), Some("did:sol:alice")),
        ];
        let m = merge_stats(&chain, &[]);
        assert_eq!(m.saved_onchain, 3);
        assert_eq!(m.saved_on_node, 3);
        assert_eq!(m.unique_users, 2);
        assert_eq!(m.buckets.len(), 2);
        assert_eq!(m.buckets[0].date, "2026-06-01");
        assert_eq!(m.buckets[0].on_chain, 2);
        assert_eq!(m.buckets[0].on_node, 0);
    }

    #[test]
    fn union_dedupes_anchored_rows_present_in_both() {
        // tx1 is on chain AND already re-written into the new DB; tx-new is
        // a fresh anchor the gateway hasn't indexed yet. Neither double-counts.
        let chain = vec![chain_item("tx1", Some("2026-06-05"), Some("did:sol:alice"))];
        let db = vec![
            db_row("tx1", Some("alice"), "2026-06-01", "anchored"),
            db_row("tx-new", Some("carol"), "2026-07-01", "anchored"),
        ];
        let m = merge_stats(&chain, &db);
        assert_eq!(m.saved_onchain, 2);
        assert_eq!(m.unique_users, 2); // did:sol:alice normalizes onto alice
                                       // DB day (exact write time) wins over the later block day.
        assert!(m
            .buckets
            .iter()
            .any(|b| b.date == "2026-06-01" && b.on_chain == 1));
        assert!(!m.buckets.iter().any(|b| b.date == "2026-06-05"));
    }

    #[test]
    fn local_rows_count_on_node_and_their_owners_count_as_users() {
        let chain = vec![chain_item("tx1", Some("2026-06-01"), Some("did:sol:alice"))];
        let db = vec![
            db_row("local:abcd1234", Some("dave"), "2026-07-02", "local"),
            db_row("", None, "2026-07-02", "local"),
        ];
        let m = merge_stats(&chain, &db);
        assert_eq!(m.saved_onchain, 1);
        assert_eq!(m.saved_on_node, 3);
        assert_eq!(m.unique_users, 2); // alice + dave; NULL owner ignored
        let d = m.buckets.iter().find(|b| b.date == "2026-07-02").unwrap();
        assert_eq!(d.on_node, 2);
        assert_eq!(d.on_chain, 0);
    }

    #[test]
    fn pending_block_items_count_in_totals_but_not_buckets() {
        let chain = vec![
            chain_item("tx1", None, None), // pending + legacy unreadable payload
        ];
        let m = merge_stats(&chain, &[]);
        assert_eq!(m.saved_onchain, 1);
        assert_eq!(m.unique_users, 0);
        assert!(m.buckets.is_empty());
    }

    // ── Chain recall (#201) ──────────────────────────────────────────────

    fn recall_item(
        tx: &str,
        content: Option<&str>,
        tags: &[&str],
        embedding: Option<RecoveredEmbedding>,
    ) -> RecoveredItem {
        RecoveredItem {
            content: content.map(str::to_string),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            embedding,
            ..chain_item(tx, Some("2026-07-01"), None)
        }
    }

    fn compressed(compressor: &EmbeddingCompressor, v: &[f32]) -> Option<RecoveredEmbedding> {
        Some(RecoveredEmbedding::Compressed(
            compressor.compress(v).to_bytes(),
        ))
    }

    fn hit_for<'a>(hits: &'a [ChainHit<'a>], tx: &str) -> Option<&'a ChainHit<'a>> {
        hits.iter().find(|h| h.item.arweave_tx == tx)
    }

    #[test]
    fn recall_scores_decoded_embeddings_semantically() {
        let compressor = EmbeddingCompressor::new(8, 4, 42);
        let query = [0.1f32; 8];
        let orthogonal = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
        let ledger = ChainLedger::new(vec![
            recall_item("near", Some("x"), &[], compressed(&compressor, &query)),
            recall_item("far", Some("y"), &[], compressed(&compressor, &orthogonal)),
            recall_item(
                "exact",
                Some("z"),
                &[],
                Some(RecoveredEmbedding::F32(query.to_vec())),
            ),
        ]);
        let hits = ledger.recall("anything", &query, &compressor);
        assert_eq!(hits.len(), 3, "every decodable item is a candidate");
        assert!(hits.iter().all(|h| h.kind == MatchKind::Semantic));
        let near = hit_for(&hits, "near").unwrap().score;
        let far = hit_for(&hits, "far").unwrap().score;
        let exact = hit_for(&hits, "exact").unwrap().score;
        assert!(near > 0.8, "dequantized neighbour stays close: {near}");
        assert!(far < near, "far={far} near={near}");
        assert!((exact - 1.0).abs() < 1e-5, "f32 copy is exact: {exact}");
    }

    #[test]
    fn recall_falls_back_to_text_match_without_usable_embedding() {
        let compressor = EmbeddingCompressor::new(8, 4, 42);
        // A 16-dim blob from another deploy must not reach dequantize
        // (it would panic on the shape mismatch) — it falls back to text.
        let other = EmbeddingCompressor::new(16, 4, 42);
        let ledger = ChainLedger::new(vec![
            recall_item("content-hit", Some("A Memory from chain"), &[], None),
            recall_item("tag-hit", Some("unrelated"), &["memory-log"], None),
            recall_item("miss", Some("unrelated"), &["other"], None),
            recall_item("no-content", None, &[], None),
            recall_item(
                "wrong-shape",
                Some("memory in a foreign embedding space"),
                &[],
                compressed(&other, &[0.1; 16]),
            ),
            recall_item(
                "garbage",
                Some("no match here"),
                &[],
                Some(RecoveredEmbedding::Compressed(vec![9, 9, 9])),
            ),
        ]);
        let hits = ledger.recall("  MEMORY ", &[0.1; 8], &compressor);
        let mut txs: Vec<&str> = hits.iter().map(|h| h.item.arweave_tx.as_str()).collect();
        txs.sort_unstable();
        assert_eq!(txs, vec!["content-hit", "tag-hit", "wrong-shape"]);
        for h in &hits {
            assert_eq!(h.kind, MatchKind::Text);
            assert_eq!(h.score, TEXT_MATCH_SCORE);
        }
    }

    #[test]
    fn cache_from_items_exposes_ledger_and_items() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let cache = ChainStatsCache::from_items(vec![recall_item("tx1", Some("c"), &[], None)]);
        rt.block_on(async {
            assert_eq!(cache.items().await.map(|v| v.len()), Some(1));
            let ledger = cache.ledger().await.expect("ledger");
            assert_eq!(ledger.items()[0].arweave_tx, "tx1");
        });
    }
}

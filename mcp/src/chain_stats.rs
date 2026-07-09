//! Chain-backed traction stats (recover-traction-from-chain).
//!
//! After the server migration lost the SQLite database, the on-chain record
//! is the only surviving history: every `participate` write is an Arweave
//! data item signed by the server wallet and tagged `App-Name:
//! mnemonic-protocol`. This module periodically snapshots that record via
//! gateway GraphQL and merges it with whatever the (fresh) local DB holds,
//! so `/stats` and `/analytics/attestations` show true lifetime numbers:
//!
//! - **anchored** — union of chain items and DB `participate` rows, deduped
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
use mnemonic_core::arweave::recovery::{normalize_producer, snapshot_chain, RecoveredItem};
use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::storage::sqlite::{RowFact, TimelineBucket};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct ChainStatsCache {
    gql: GraphQlClient,
    gateway: ArweaveClient,
    owner_addresses: Vec<String>,
    snapshot: tokio::sync::RwLock<Option<Vec<RecoveredItem>>>,
}

impl ChainStatsCache {
    /// `None` when no wallets are configured (feature off). Wallets that
    /// fail base58 decoding are rejected loudly — a typo'd wallet silently
    /// shrinking the traction numbers is worse than a startup error.
    pub fn new(
        wallets: &[String],
        graphql_url: &str,
        gateway_url: &str,
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
            owner_addresses,
            snapshot: tokio::sync::RwLock::new(None),
        }))
    }

    /// Re-enumerate the chain. Errors are returned (caller logs and keeps
    /// the previous snapshot — a gateway outage must not zero the page).
    pub async fn refresh(&self) -> anyhow::Result<usize> {
        let snap = snapshot_chain(&self.gql, &self.gateway, &self.owner_addresses).await?;
        let n = snap.items.len();
        *self.snapshot.write().await = Some(snap.items);
        Ok(n)
    }

    /// Latest snapshot, or `None` until the first successful refresh.
    pub async fn items(&self) -> Option<Vec<RecoveredItem>> {
        self.snapshot.read().await.clone()
    }
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
        if row.write_mode == "participate" && real_arweave {
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
            day: day.map(str::to_string),
            producer: producer.map(str::to_string),
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
            db_row("tx1", Some("alice"), "2026-06-01", "participate"),
            db_row("tx-new", Some("carol"), "2026-07-01", "participate"),
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
}

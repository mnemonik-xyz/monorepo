//! Chain recovery — rebuild traction facts from the chain after DB loss.
//!
//! Two enumeration sources, unioned by `arweave_tx`:
//!
//! 1. **Solana memo history** — authoritative for the historical items:
//!    every `participate` write left an SPL Memo naming its Arweave tx,
//!    and `getSignaturesForAddress` enumerates them even though the
//!    gateways' GraphQL never indexed the old Irys-bundled items
//!    (verified live 2026-07-09: 16 memos, 0 GraphQL hits).
//! 2. **Gateway GraphQL** ([`super::graphql`]) — catches items whose memo
//!    is missing (e.g. a Solana write failed after the Arweave upload)
//!    and future tagged uploads.
//!
//! The producer identity of each memory lives in the COSE_Sign1 payload
//! (`producer = "did:sol:<sub>"`). New uploads carry a `Producer` tag so
//! the payload fetch is only needed for legacy items — a one-time backfill
//! cost per process.

use super::graphql::GraphQlClient;
use super::ArweaveClient;
use crate::solana::MemoAnchor;
use coset::CborSerializable;

/// One anchored memory with everything traction stats need.
#[derive(Debug, Clone)]
pub struct RecoveredItem {
    pub arweave_tx: String,
    /// UTC day `YYYY-MM-DD` from the block timestamp; `None` while pending.
    pub day: Option<String>,
    /// Producer DID (`did:sol:<pubkey-or-oauth-sub>`), from the `Producer`
    /// tag or decoded from the COSE payload for legacy items.
    pub producer: Option<String>,
}

/// Point-in-time view of every item this node's wallet(s) ever anchored.
#[derive(Debug, Clone, Default)]
pub struct ChainSnapshot {
    pub items: Vec<RecoveredItem>,
}

/// Union memo anchors and GraphQL items into the anchored ledger, then
/// backfill producers by fetching + decoding COSE payloads where no
/// `Producer` tag was indexed. A single unreadable payload downgrades that
/// item's producer to `None` (still counted in totals) rather than failing
/// the whole snapshot.
pub async fn snapshot_chain(
    gql: &GraphQlClient,
    gateway: &ArweaveClient,
    owner_addresses: &[String],
    memo_anchors: &[MemoAnchor],
) -> anyhow::Result<ChainSnapshot> {
    let anchored = gql.list_anchored(owner_addresses).await?;

    // (arweave_tx, block_time, producer) — memo anchors first (they carry
    // the memo-write time, closer to the user action than the item's own
    // block), GraphQL items add anything the memo history missed.
    let mut seen = std::collections::HashSet::new();
    let mut pending: Vec<(String, Option<i64>, Option<String>)> = Vec::new();
    for m in memo_anchors {
        if seen.insert(m.arweave_tx.clone()) {
            pending.push((m.arweave_tx.clone(), m.block_time, None));
        }
    }
    for a in anchored {
        if seen.insert(a.arweave_tx.clone()) {
            pending.push((a.arweave_tx, a.block_time, a.producer));
        }
    }

    let mut items = Vec::with_capacity(pending.len());
    for (arweave_tx, block_time, tagged_producer) in pending {
        let producer = match tagged_producer {
            Some(p) => Some(p),
            None => match gateway.read(&arweave_tx).await {
                Ok(bytes) => producer_from_cose(&bytes),
                Err(e) => {
                    tracing::warn!("chain recovery: payload fetch failed for {arweave_tx}: {e}");
                    None
                }
            },
        };
        items.push(RecoveredItem {
            arweave_tx,
            day: block_time.and_then(day_from_unix),
            producer,
        });
    }
    Ok(ChainSnapshot { items })
}

/// Extract the `producer` field from a COSE_Sign1 envelope over canonical
/// CBOR. Pure structural decode — no signature verification, since the
/// caller only needs the identity for a distinct-count (the envelope's
/// authenticity is anyone's to verify independently).
pub fn producer_from_cose(cose_bytes: &[u8]) -> Option<String> {
    let cose = coset::CoseSign1::from_slice(cose_bytes).ok()?;
    let payload = cose.payload.as_ref()?;
    let json = crate::codec::canonical::from_canonical_cbor(payload).ok()?;
    json.get("producer")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// `did:sol:<sub>` → `<sub>` — aligns chain producers with the raw
/// `owner_pubkey` column so distinct-user sets union correctly.
pub fn normalize_producer(producer: &str) -> String {
    producer
        .strip_prefix("did:sol:")
        .unwrap_or(producer)
        .to_string()
}

fn day_from_unix(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::schema::MEMORY_V1;
    use crate::codec::sign::sign_artifact;
    use httpmock::prelude::*;
    use solana_sdk::signature::Keypair;

    fn signed_memory_cose(producer: &str) -> Vec<u8> {
        let artifact = serde_json::json!({
            "artifact_id": "11111111-2222-3333-4444-555555555555",
            "type": "memory",
            "schema_version": 1,
            "content": "the sky over the port",
            "producer": producer,
            "created_at": "2026-07-01T00:00:00Z",
            "tags": ["test"],
            "metadata": {},
        });
        sign_artifact(&artifact, &MEMORY_V1, &Keypair::new())
            .expect("sign")
            .cose_bytes
    }

    #[test]
    fn producer_extracted_from_cose_payload() {
        let cose = signed_memory_cose("did:sol:user-123");
        assert_eq!(
            producer_from_cose(&cose).as_deref(),
            Some("did:sol:user-123")
        );
    }

    #[test]
    fn producer_from_garbage_is_none() {
        assert_eq!(producer_from_cose(b"not cose at all"), None);
    }

    #[test]
    fn normalize_strips_did_sol_prefix_only() {
        assert_eq!(normalize_producer("did:sol:abc"), "abc");
        assert_eq!(normalize_producer("raw-sub"), "raw-sub");
    }

    #[test]
    fn day_conversion_is_utc() {
        assert_eq!(day_from_unix(1_700_000_000).as_deref(), Some("2023-11-14"));
    }

    #[tokio::test]
    async fn snapshot_backfills_legacy_producers_from_payload() {
        let server = MockServer::start();
        let cose = signed_memory_cose("did:sol:legacy-user");

        // GraphQL: one tagged item (producer known) + one legacy item.
        server.mock(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200).json_body(serde_json::json!({
                "data": { "transactions": {
                    "pageInfo": { "hasNextPage": false },
                    "edges": [
                        { "cursor": "c1", "node": { "id": "tagged-tx",
                          "block": {"timestamp": 1_700_000_000},
                          "tags": [{"name": "Producer", "value": "did:sol:tagged-user"}] } },
                        { "cursor": "c2", "node": { "id": "legacy-tx",
                          "block": null, "tags": [] } },
                    ],
                }}
            }));
        });
        // Gateway payload fetch for the legacy item only.
        let payload_mock = server.mock(|when, then| {
            when.method(GET).path("/legacy-tx");
            then.status(200).body(cose.clone());
        });

        let gql = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
        let gateway = ArweaveClient::new(&server.base_url());
        let snap = snapshot_chain(&gql, &gateway, &[], &[]).await.unwrap();

        assert_eq!(snap.items.len(), 2);
        assert_eq!(
            snap.items[0].producer.as_deref(),
            Some("did:sol:tagged-user")
        );
        assert_eq!(snap.items[0].day.as_deref(), Some("2023-11-14"));
        assert_eq!(
            snap.items[1].producer.as_deref(),
            Some("did:sol:legacy-user")
        );
        assert_eq!(snap.items[1].day, None);
        payload_mock.assert();
    }

    #[tokio::test]
    async fn memo_anchors_union_with_graphql_and_backfill_producers() {
        // Production shape (verified live): the memo history knows every
        // anchor, GraphQL knows none of the old ones. One tx appears in
        // both sources and must not double-count; the memo-only item gets
        // its producer from the payload.
        let server = MockServer::start();
        let cose = signed_memory_cose("did:sol:memo-user");

        server.mock(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200).json_body(serde_json::json!({
                "data": { "transactions": {
                    "pageInfo": { "hasNextPage": false },
                    "edges": [
                        { "cursor": "c1", "node": { "id": "shared-tx",
                          "block": {"timestamp": 1_700_000_000},
                          "tags": [{"name": "Producer", "value": "did:sol:tagged-user"}] } },
                    ],
                }}
            }));
        });
        server.mock(|when, then| {
            when.method(GET).path("/memo-only-tx");
            then.status(200).body(cose.clone());
        });
        // The shared tx also needs a payload fetch: the memo anchor comes
        // first in the union and carries no producer tag.
        server.mock(|when, then| {
            when.method(GET).path("/shared-tx");
            then.status(200)
                .body(signed_memory_cose("did:sol:tagged-user"));
        });

        let anchors = vec![
            MemoAnchor {
                solana_tx: "sig1".into(),
                arweave_tx: "memo-only-tx".into(),
                content_hash: "h1".into(),
                block_time: Some(1_746_144_000), // 2025-05-02
            },
            MemoAnchor {
                solana_tx: "sig2".into(),
                arweave_tx: "shared-tx".into(),
                content_hash: "h2".into(),
                block_time: Some(1_700_000_000),
            },
        ];

        let gql = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
        let gateway = ArweaveClient::new(&server.base_url());
        let snap = snapshot_chain(&gql, &gateway, &[], &anchors).await.unwrap();

        assert_eq!(snap.items.len(), 2, "shared tx must not double-count");
        assert_eq!(snap.items[0].arweave_tx, "memo-only-tx");
        assert_eq!(snap.items[0].producer.as_deref(), Some("did:sol:memo-user"));
        assert_eq!(snap.items[0].day.as_deref(), Some("2025-05-02"));
        assert_eq!(snap.items[1].arweave_tx, "shared-tx");
    }
}

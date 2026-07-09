//! Chain recovery — rebuild traction facts from Arweave after DB loss.
//!
//! The gateway index ([`super::graphql`]) yields the item ids and block
//! times; the producer identity of each memory lives in the COSE_Sign1
//! payload (`producer = "did:sol:<sub>"`). New uploads carry a `Producer`
//! tag so the payload fetch is only needed for legacy items — a one-time
//! backfill cost per process.

use super::graphql::GraphQlClient;
use super::ArweaveClient;
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

/// Enumerate anchored items via GraphQL and backfill producers for legacy
/// items by fetching + decoding their COSE payloads. A single unreadable
/// payload downgrades that item's producer to `None` (still counted in
/// totals) rather than failing the whole snapshot.
pub async fn snapshot_chain(
    gql: &GraphQlClient,
    gateway: &ArweaveClient,
    owner_addresses: &[String],
) -> anyhow::Result<ChainSnapshot> {
    let anchored = gql.list_anchored(owner_addresses).await?;

    let mut items = Vec::with_capacity(anchored.len());
    for a in anchored {
        let producer = match a.producer {
            Some(p) => Some(p),
            None => match gateway.read(&a.arweave_tx).await {
                Ok(bytes) => producer_from_cose(&bytes),
                Err(e) => {
                    tracing::warn!(
                        "chain recovery: payload fetch failed for {}: {e}",
                        a.arweave_tx
                    );
                    None
                }
            },
        };
        items.push(RecoveredItem {
            arweave_tx: a.arweave_tx,
            day: a.block_time.and_then(day_from_unix),
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
        let snap = snapshot_chain(&gql, &gateway, &[]).await.unwrap();

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
}

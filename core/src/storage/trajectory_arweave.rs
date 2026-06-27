//! Canonical Arweave-bundle backend for verifiable trajectories.
//!
//! Per `work/verifiable-trajectories/decisions.md` the canonical store is the
//! permaweb and the MCP server is stateless. Steps/verdicts are written as
//! tagged ANS-104 data items (one bundled write per checkpoint in production via
//! Irys) and read back by **GraphQL tag query** — no server database.
//!
//! - **Writes** (`write_step` / `write_verdict`) are async, via [`ArweaveClient`]
//!   (Irys in prod, arlocal in dev), tagging each item with `Trajectory-Id` /
//!   `Seq` / `Content-Hash` / `Producer` (steps) or `Step-Hash` / `Status` /
//!   `Judge` (verdicts).
//! - **Reads** implement the sync [`TrajectoryStore`] trait using a *blocking*
//!   HTTP client so the same `verify_chain` / `verdict_coverage` run unchanged.
//!   `canonical_cbor` is recovered as the COSE_Sign1 payload; the rest comes
//!   from the item's tags. Call from an async context via
//!   `tokio::task::spawn_blocking`.

use std::collections::HashMap;

use anyhow::{anyhow, Context};
use coset::{CborSerializable, CoseSign1};
use solana_sdk::signature::Keypair;

use super::super::arweave::ArweaveClient;
use crate::trajectory::{StepRecord, TrajectoryStore, VerdictRecord, VerdictStatus};

const APP: (&str, &str) = ("App-Name", "mnemonic-protocol");

pub struct ArweaveStore {
    gateway_url: String,
    client: ArweaveClient,
}

impl ArweaveStore {
    /// `gateway_url` serves both `/graphql` (tag queries) and `/<tx_id>` (data).
    pub fn new(gateway_url: &str) -> Self {
        let g = gateway_url.trim_end_matches('/').to_string();
        Self {
            client: ArweaveClient::new(&g),
            gateway_url: g,
        }
    }

    #[cfg(test)]
    fn new_for_test(gateway_url: &str) -> Self {
        let g = gateway_url.trim_end_matches('/').to_string();
        Self {
            client: ArweaveClient::new_for_test(g.clone()),
            gateway_url: g,
        }
    }

    /// Blocking HTTP client for the sync read trait. Built per call so the
    /// struct never holds a runtime-backed client across an `.await` (reads run
    /// on `spawn_blocking` threads, where transient construction is safe).
    fn http() -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }

    /// Write one step as a tagged data item. Returns its Arweave tx id.
    pub async fn write_step(&self, r: &StepRecord, keypair: &Keypair) -> anyhow::Result<String> {
        let seq = r.seq.to_string();
        let prev = r.prev_hash.clone().unwrap_or_default();
        let tags = [
            ("Kind", "step"),
            ("Trajectory-Id", r.trajectory_id.as_str()),
            ("Seq", seq.as_str()),
            ("Prev-Hash", prev.as_str()),
            ("Content-Hash", r.content_hash.as_str()),
            ("Producer", r.producer.as_str()),
        ];
        self.client.write_item(&r.cose_bytes, keypair, &tags).await
    }

    /// Write one verdict as a tagged data item. Returns its Arweave tx id.
    pub async fn write_verdict(
        &self,
        v: &VerdictRecord,
        judge: &Keypair,
    ) -> anyhow::Result<String> {
        let tags = [
            ("Kind", "verdict"),
            ("Step-Hash", v.step_hash.as_str()),
            ("Status", v.status.as_str()),
            ("Judge", v.judge.as_str()),
            ("Content-Hash", v.content_hash.as_str()),
        ];
        self.client.write_item(&v.cose_bytes, judge, &tags).await
    }

    /// Run a GraphQL tag query, returning `(tx_id, tags)` per match.
    fn query(
        &self,
        filters: &[(&str, &str)],
    ) -> anyhow::Result<Vec<(String, HashMap<String, String>)>> {
        let tags_gql = filters
            .iter()
            .map(|(n, v)| format!("{{name:\"{n}\",values:[\"{v}\"]}}"))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "query{{transactions(tags:[{tags_gql}],first:1000){{edges{{node{{id tags{{name value}}}}}}}}}}"
        );
        let resp = Self::http()
            .post(format!("{}/graphql", self.gateway_url))
            .json(&serde_json::json!({ "query": query }))
            .send()
            .context("graphql query")?
            .error_for_status()
            .context("graphql status")?;
        let v: serde_json::Value = resp.json().context("graphql json")?;
        let edges = v["data"]["transactions"]["edges"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(edges.len());
        for e in edges {
            let id = e["node"]["id"].as_str().unwrap_or_default().to_string();
            let mut tags = HashMap::new();
            if let Some(ts) = e["node"]["tags"].as_array() {
                for t in ts {
                    if let (Some(n), Some(val)) = (t["name"].as_str(), t["value"].as_str()) {
                        tags.insert(n.to_string(), val.to_string());
                    }
                }
            }
            out.push((id, tags));
        }
        Ok(out)
    }

    fn fetch(&self, tx_id: &str) -> anyhow::Result<Vec<u8>> {
        let resp = Self::http()
            .get(format!("{}/{tx_id}", self.gateway_url))
            .send()
            .context("fetch tx")?
            .error_for_status()
            .context("fetch status")?;
        Ok(resp.bytes().context("fetch bytes")?.to_vec())
    }

    /// The COSE_Sign1 payload is exactly the canonical CBOR the verifier rehashes.
    fn cose_payload(cose: &[u8]) -> anyhow::Result<Vec<u8>> {
        CoseSign1::from_slice(cose)
            .map_err(|e| anyhow!("decode COSE: {e}"))?
            .payload
            .ok_or_else(|| anyhow!("COSE has no payload"))
    }
}

impl TrajectoryStore for ArweaveStore {
    fn steps_for_trajectory(&self, trajectory_id: &str) -> anyhow::Result<Vec<StepRecord>> {
        let rows = self.query(&[APP, ("Kind", "step"), ("Trajectory-Id", trajectory_id)])?;
        let mut steps = Vec::with_capacity(rows.len());
        for (id, tags) in rows {
            let cose = self.fetch(&id)?;
            let canonical_cbor = Self::cose_payload(&cose)?;
            steps.push(StepRecord {
                trajectory_id: trajectory_id.to_string(),
                seq: tags.get("Seq").and_then(|s| s.parse().ok()).unwrap_or(0),
                content_hash: tags.get("Content-Hash").cloned().unwrap_or_default(),
                prev_hash: tags.get("Prev-Hash").cloned().filter(|s| !s.is_empty()),
                producer: tags.get("Producer").cloned().unwrap_or_default(),
                cose_bytes: cose,
                canonical_cbor,
            });
        }
        steps.sort_by_key(|s| s.seq);
        Ok(steps)
    }

    fn verdicts_for_step(&self, step_hash: &str) -> anyhow::Result<Vec<VerdictRecord>> {
        let rows = self.query(&[APP, ("Kind", "verdict"), ("Step-Hash", step_hash)])?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, tags) in rows {
            let cose = self.fetch(&id)?;
            out.push(VerdictRecord {
                step_hash: step_hash.to_string(),
                status: tags
                    .get("Status")
                    .and_then(|s| VerdictStatus::from_str(s))
                    .unwrap_or(VerdictStatus::Concern),
                judge: tags.get("Judge").cloned().unwrap_or_default(),
                content_hash: tags.get("Content-Hash").cloned().unwrap_or_default(),
                cose_bytes: cose,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::{build_step, verify_chain, StepInput};
    use httpmock::prelude::*;
    use solana_sdk::signature::Signer;

    fn step(seq: u64, prev: Option<&str>, kp: &Keypair) -> StepRecord {
        let s = build_step(
            &StepInput {
                trajectory_id: "t",
                seq,
                content: "c",
                prev_hash: prev,
                created_at: "2026-06-27T00:00:00Z",
            },
            kp,
        )
        .unwrap();
        StepRecord {
            trajectory_id: "t".into(),
            seq,
            content_hash: s.content_hash,
            prev_hash: prev.map(str::to_string),
            producer: kp.pubkey().to_string(),
            cose_bytes: s.cose_bytes,
            canonical_cbor: s.canonical_cbor,
        }
    }

    fn edge(id: &str, s: &StepRecord) -> serde_json::Value {
        serde_json::json!({"node": {"id": id, "tags": [
            {"name":"Kind","value":"step"},
            {"name":"Trajectory-Id","value":"t"},
            {"name":"Seq","value": s.seq.to_string()},
            {"name":"Prev-Hash","value": s.prev_hash.clone().unwrap_or_default()},
            {"name":"Content-Hash","value": s.content_hash},
            {"name":"Producer","value": s.producer},
        ]}})
    }

    #[test]
    fn reads_chain_from_graphql_and_verifies() {
        let kp = Keypair::new();
        let s0 = step(0, None, &kp);
        let s1 = step(1, Some(&s0.content_hash), &kp);

        let server = MockServer::start();
        // GraphQL returns the two steps (out of order — store must sort by seq).
        let gql = serde_json::json!({"data":{"transactions":{"edges":[
            edge("tx1", &s1), edge("tx0", &s0)
        ]}}});
        server.mock(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200).json_body(gql);
        });
        // Data fetches return each step's COSE bytes.
        server.mock(|when, then| {
            when.method(GET).path("/tx0");
            then.status(200).body(s0.cose_bytes.clone());
        });
        server.mock(|when, then| {
            when.method(GET).path("/tx1");
            then.status(200).body(s1.cose_bytes.clone());
        });

        let store = ArweaveStore::new(&server.base_url());
        let steps = store.steps_for_trajectory("t").unwrap();
        assert_eq!(steps.iter().map(|s| s.seq).collect::<Vec<_>>(), vec![0, 1]);
        // The verifier runs unchanged against the permaweb-backed store.
        assert!(verify_chain(&store, "t").unwrap().chain_valid);
    }

    #[tokio::test]
    async fn write_step_posts_data_item() {
        let kp = Keypair::new();
        let s0 = step(0, None, &kp);
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST).path("/upload");
            then.status(200).body(r#"{"id":"bundle-tx-1"}"#);
        });
        let store = ArweaveStore::new_for_test(&server.base_url());
        let id = store.write_step(&s0, &kp).await.unwrap();
        assert_eq!(id, "bundle-tx-1");
        m.assert();
    }
}

//! Arweave/Irys gateway GraphQL client — enumerates anchored mnemonic-protocol
//! data items so traction stats survive a total node-database loss.
//!
//! Every `anchored` write uploads a COSE_Sign1 envelope as an ANS-104
//! item signed by the server's Solana keypair and tagged
//! `App-Name: mnemonic-protocol` (see `ArweaveClient::write_irys` /
//! `write_item`). The Irys GraphQL endpoint indexes those items (Arweave
//! gateways index the containing bundle, not the items themselves).
//!
//! ## Gateway schemas differ
//!
//! Irys rejects two fields the Arweave schema accepts:
//! - `sort` argument on `transactions` → `Unknown argument`
//! - `block { timestamp }` field on `Transaction` → `Cannot query field`
//!
//! [`GatewayFlavour`] selects the query shape and controls how timestamps
//! are parsed. Derive it from the URL via [`flavour_from_url`], or override
//! with [`GraphQlClient::new_with_flavour`].

use anyhow::Context;
use base64::Engine;
use sha2::{Digest, Sha256};

/// The `App-Name` tag value stamped on every upload.
pub const APP_NAME: &str = "mnemonic-protocol";

/// Gateway page size (arweave.net caps `first` at 100).
const PAGE_SIZE: usize = 100;

/// Hard cap on pages per enumeration — backstop against a gateway that
/// keeps returning `hasNextPage: true` (1M items is far beyond current scale).
const MAX_PAGES: usize = 10_000;

/// Which gateway schema to use when building queries and parsing responses.
///
/// Irys rejects `sort` and `block { ... }` — both valid in the Arweave schema
/// but hard errors on Irys. Irys also returns timestamps as milliseconds on the
/// node directly, whereas Arweave nests them under `block.timestamp` in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayFlavour {
    /// Standard Arweave gateway (arweave.net, goldsky, …): supports
    /// `sort: HEIGHT_ASC`, returns timestamps under `block.timestamp` in
    /// seconds. Results arrive oldest-first from the gateway.
    Arweave,
    /// Irys gateway (uploader.irys.xyz, …): no `sort` argument, timestamp is a
    /// top-level field in **milliseconds**. Results arrive newest-first;
    /// ordering is applied client-side after all pages are fetched.
    Irys,
}

/// Derive the gateway flavour from a URL. Any URL whose host contains
/// `irys.xyz` is treated as Irys; everything else is Arweave.
pub fn flavour_from_url(url: &str) -> GatewayFlavour {
    if url.contains("irys.xyz") {
        GatewayFlavour::Irys
    } else {
        GatewayFlavour::Arweave
    }
}

/// One anchored data item as reported by the gateway index.
#[derive(Debug, Clone)]
pub struct AnchoredItem {
    /// Arweave/ANS-104 data-item id (the `arweave_tx` persisted in SQLite).
    pub arweave_tx: String,
    /// Unix seconds of the containing block; `None` while still pending.
    pub block_time: Option<i64>,
    /// `Producer` tag value when present. Legacy uploads (before the tag was
    /// introduced) carry the producer DID only inside the COSE payload.
    pub producer: Option<String>,
}

/// Derive the **Arweave-gateway** owner address for an ANS-104 item signed by
/// an Ed25519 (Solana) key: `base64url_nopad(sha256(pubkey_bytes))` — the
/// arbundles `ownerToAddress` rule.
///
/// **This address is only correct for `GatewayFlavour::Arweave` queries.**
/// Irys indexes items by the raw base58 Solana pubkey, not by this SHA-256
/// derived form. When querying Irys, pass the base58 pubkey directly as the
/// `owners` filter value (see [`GatewayFlavour`] and [`GraphQlClient::list_anchored`]).
pub fn solana_pubkey_to_arweave_address(pubkey_base58: &str) -> anyhow::Result<String> {
    let bytes = bs58::decode(pubkey_base58.trim())
        .into_vec()
        .context("invalid base58 Solana pubkey")?;
    anyhow::ensure!(
        bytes.len() == 32,
        "expected 32-byte Ed25519 pubkey, got {}",
        bytes.len()
    );
    let digest = Sha256::digest(&bytes);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

pub struct GraphQlClient {
    url: String,
    flavour: GatewayFlavour,
    client: reqwest::Client,
}

impl GraphQlClient {
    /// Create a client and auto-detect the gateway flavour from the URL.
    /// URLs containing `irys.xyz` use the Irys schema; everything else uses
    /// the Arweave schema. Use [`new_with_flavour`] for an explicit override.
    pub fn new(url: &str) -> Self {
        Self::new_with_flavour(url, flavour_from_url(url))
    }

    pub fn new_with_flavour(url: &str, flavour: GatewayFlavour) -> Self {
        Self {
            url: url.to_string(),
            flavour,
            client: super::http_client(),
        }
    }

    pub fn flavour(&self) -> GatewayFlavour {
        self.flavour
    }

    /// Enumerate all anchored items, oldest-first.
    ///
    /// `owner_addresses` format depends on the gateway flavour:
    /// - `GatewayFlavour::Irys` — pass the **raw base58 Solana pubkey**. Irys
    ///   indexes items by the raw pubkey, not by the SHA-256-derived Arweave
    ///   address. Verified 2026-09-28 against `uploader.irys.xyz`.
    /// - `GatewayFlavour::Arweave` — pass the Arweave-form address produced by
    ///   [`solana_pubkey_to_arweave_address`] (SHA-256 / base64url-nopad).
    /// - Empty slice — no owner filter (tag-only query).
    pub async fn list_anchored(
        &self,
        owner_addresses: &[String],
    ) -> anyhow::Result<Vec<AnchoredItem>> {
        let report = self.scan_anchored(owner_addresses, MAX_PAGES).await;
        anyhow::ensure!(
            report.exhausted,
            "{}",
            report
                .error
                .unwrap_or_else(|| "discovery budget exhausted".into())
        );
        Ok(report.items)
    }

    /// Preserve fetched pages on outage, cursor loops or budget exhaustion.
    pub async fn scan_anchored(
        &self,
        owner_addresses: &[String],
        max_pages: usize,
    ) -> crate::restore::SourceScan<AnchoredItem> {
        let mut report = crate::restore::SourceScan::default();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        if !(1..=MAX_PAGES).contains(&max_pages) {
            report.error = Some("invalid discovery page budget".into());
            return report;
        }
        for _ in 0..max_pages {
            let page = match self.fetch_page(owner_addresses, cursor.as_deref()).await {
                Ok(page) => page,
                Err(e) => {
                    report.error = Some(e.to_string());
                    break;
                }
            };
            report.items.extend(page.items);
            match page.next_cursor {
                Some(c) => {
                    if !seen.insert(c.clone()) {
                        report.error = Some("discovery cursor loop".into());
                        break;
                    }
                    cursor = Some(c);
                }
                None => {
                    report.exhausted = true;
                    break;
                }
            }
        }
        report.budget_exhausted = !report.exhausted && report.error.is_none();
        if self.flavour == GatewayFlavour::Irys {
            report
                .items
                .sort_by_key(|i| i.block_time.unwrap_or(i64::MAX));
        }
        report
    }

    async fn fetch_page(
        &self,
        owner_addresses: &[String],
        after: Option<&str>,
    ) -> anyhow::Result<PageResult> {
        // Owners are inlined as a GraphQL list literal (serde_json string
        // array is valid GraphQL syntax); omitting the argument entirely is
        // the only reliable "no filter" form across gateway implementations.
        let owners_clause = if owner_addresses.is_empty() {
            String::new()
        } else {
            format!("owners: {},", serde_json::to_string(owner_addresses)?)
        };

        let query = match self.flavour {
            GatewayFlavour::Arweave => format!(
                r#"query($after: String) {{
  transactions(
    {owners_clause}
    tags: [{{ name: "App-Name", values: ["{APP_NAME}"] }}],
    first: {PAGE_SIZE},
    after: $after,
    sort: HEIGHT_ASC
  ) {{
    pageInfo {{ hasNextPage }}
    edges {{ cursor node {{ id block {{ timestamp }} tags {{ name value }} }} }}
  }}
}}"#
            ),
            GatewayFlavour::Irys => format!(
                r#"query($after: String) {{
  transactions(
    {owners_clause}
    tags: [{{ name: "App-Name", values: ["{APP_NAME}"] }}],
    first: {PAGE_SIZE},
    after: $after
  ) {{
    pageInfo {{ hasNextPage }}
    edges {{ cursor node {{ id timestamp tags {{ name value }} }} }}
  }}
}}"#
            ),
        };

        let body = serde_json::json!({
            "query": query,
            "variables": { "after": after },
        });

        let resp = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .context("arweave graphql request")?;
        anyhow::ensure!(
            resp.status().is_success(),
            "arweave graphql returned {}",
            resp.status()
        );
        let json: serde_json::Value = resp.json().await.context("arweave graphql body")?;
        if let Some(errors) = json.get("errors") {
            if errors.as_array().is_some_and(|a| !a.is_empty()) {
                anyhow::bail!("arweave graphql errors: {errors}");
            }
        }
        parse_page(&json, self.flavour)
    }
}

struct PageResult {
    items: Vec<AnchoredItem>,
    /// Cursor of the last edge when the gateway reports another page.
    next_cursor: Option<String>,
}

fn parse_page(json: &serde_json::Value, flavour: GatewayFlavour) -> anyhow::Result<PageResult> {
    let tx = &json["data"]["transactions"];
    let edges = tx["edges"]
        .as_array()
        .context("graphql response missing data.transactions.edges")?;

    let mut items = Vec::with_capacity(edges.len());
    let mut last_cursor = None;
    for edge in edges {
        let node = &edge["node"];
        let id = node["id"]
            .as_str()
            .context("graphql edge node missing id")?
            .to_string();
        let block_time = match flavour {
            // Arweave: seconds nested under block.timestamp.
            GatewayFlavour::Arweave => node["block"]["timestamp"].as_i64(),
            // Irys: milliseconds as a top-level field; convert to seconds.
            GatewayFlavour::Irys => node["timestamp"].as_i64().map(|ms| ms / 1000),
        };
        let producer = node["tags"].as_array().and_then(|tags| {
            tags.iter()
                .find(|t| t["name"].as_str() == Some("Producer"))
                .and_then(|t| t["value"].as_str())
                .map(str::to_string)
        });
        items.push(AnchoredItem {
            arweave_tx: id,
            block_time,
            producer,
        });
        last_cursor = edge["cursor"].as_str().map(str::to_string);
    }

    let has_next = tx["pageInfo"]["hasNextPage"]
        .as_bool()
        .context("graphql missing pageInfo.hasNextPage")?;
    // A gateway claiming hasNextPage with an empty/cursorless page would
    // loop forever — treat it as the final page instead.
    let next_cursor = if has_next { last_cursor } else { None };
    Ok(PageResult { items, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn arweave_edge(id: &str, ts: Option<i64>, producer: Option<&str>) -> serde_json::Value {
        let mut tags = vec![serde_json::json!({"name": "App-Name", "value": APP_NAME})];
        if let Some(p) = producer {
            tags.push(serde_json::json!({"name": "Producer", "value": p}));
        }
        serde_json::json!({
            "cursor": format!("cur-{id}"),
            "node": {
                "id": id,
                "block": ts.map(|t| serde_json::json!({"timestamp": t})),
                "tags": tags,
            }
        })
    }

    fn irys_edge(id: &str, ts_ms: Option<i64>, producer: Option<&str>) -> serde_json::Value {
        let mut tags = vec![serde_json::json!({"name": "App-Name", "value": APP_NAME})];
        if let Some(p) = producer {
            tags.push(serde_json::json!({"name": "Producer", "value": p}));
        }
        let mut node = serde_json::json!({
            "id": id,
            "tags": tags,
        });
        if let Some(ms) = ts_ms {
            node["timestamp"] = serde_json::json!(ms);
        }
        serde_json::json!({
            "cursor": format!("cur-{id}"),
            "node": node,
        })
    }

    fn page(edges: Vec<serde_json::Value>, has_next: bool) -> serde_json::Value {
        serde_json::json!({
            "data": { "transactions": {
                "pageInfo": { "hasNextPage": has_next },
                "edges": edges,
            }}
        })
    }

    #[test]
    fn address_derivation_is_stable_base64url() {
        // Base58 of 32 0x01 bytes; address must be 43-char base64url (no pad).
        let pubkey = bs58::encode([1u8; 32]).into_string();
        let addr = solana_pubkey_to_arweave_address(&pubkey).unwrap();
        assert_eq!(addr.len(), 43);
        assert!(!addr.contains('='));
        // Deterministic: same input, same address.
        assert_eq!(addr, solana_pubkey_to_arweave_address(&pubkey).unwrap());
    }

    #[test]
    fn address_derivation_rejects_garbage() {
        assert!(solana_pubkey_to_arweave_address("not-base58-0OIl").is_err());
        assert!(solana_pubkey_to_arweave_address(&bs58::encode([1u8; 16]).into_string()).is_err());
    }

    #[test]
    fn flavour_detection_from_url() {
        assert_eq!(
            flavour_from_url("https://uploader.irys.xyz/graphql"),
            GatewayFlavour::Irys
        );
        assert_eq!(
            flavour_from_url("https://devnet.irys.xyz/graphql"),
            GatewayFlavour::Irys
        );
        assert_eq!(
            flavour_from_url("https://arweave.net/graphql"),
            GatewayFlavour::Arweave
        );
        assert_eq!(
            flavour_from_url("https://arweave-search.goldsky.com/graphql"),
            GatewayFlavour::Arweave
        );
        assert_eq!(
            flavour_from_url("http://localhost:1984/graphql"),
            GatewayFlavour::Arweave
        );
    }

    /// Arweave query must contain `sort` and `block`; these would be rejected
    /// by Irys. Both schemas must be checked in the same test suite so that a
    /// change to one cannot silently break the other.
    #[tokio::test]
    async fn arweave_query_contains_sort_and_block() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/graphql")
                .body_includes("sort: HEIGHT_ASC")
                .body_includes("block {");
            then.status(200).json_body(page(vec![], false));
        });
        let client = GraphQlClient::new_with_flavour(
            &format!("{}/graphql", server.base_url()),
            GatewayFlavour::Arweave,
        );
        client.list_anchored(&[]).await.unwrap();
        mock.assert();
    }

    /// Irys query must contain neither `sort` nor `block` — both cause hard
    /// errors on the Irys GraphQL endpoint.
    #[tokio::test]
    async fn irys_query_omits_sort_and_block() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/graphql")
                .body_excludes("sort:")
                .body_excludes("block {");
            then.status(200).json_body(page(vec![], false));
        });
        let client = GraphQlClient::new_with_flavour(
            &format!("{}/graphql", server.base_url()),
            GatewayFlavour::Irys,
        );
        client.list_anchored(&[]).await.unwrap();
        mock.assert();
    }

    /// Irys timestamps are milliseconds and must be divided by 1000.
    /// Arweave timestamps are already seconds and must pass through unchanged.
    #[test]
    fn timestamp_units_irys_converts_ms_to_s() {
        let ms: i64 = 1_700_000_000_000;
        let p = page(vec![irys_edge("tx1", Some(ms), None)], false);
        let result = parse_page(&p, GatewayFlavour::Irys).unwrap();
        assert_eq!(result.items[0].block_time, Some(1_700_000_000));
    }

    #[test]
    fn timestamp_units_arweave_passes_through() {
        let s: i64 = 1_700_000_000;
        let p = page(vec![arweave_edge("tx1", Some(s), None)], false);
        let result = parse_page(&p, GatewayFlavour::Arweave).unwrap();
        assert_eq!(result.items[0].block_time, Some(1_700_000_000));
    }

    /// Irys returns newest-first; list_anchored must sort to oldest-first.
    #[tokio::test]
    async fn irys_results_are_sorted_oldest_first() {
        let server = MockServer::start();
        // Gateway returns newest-first (descending timestamps in ms).
        server.mock(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200).json_body(page(
                vec![
                    irys_edge("tx-new", Some(1_700_000_100_000), None),
                    irys_edge("tx-old", Some(1_700_000_000_000), None),
                    irys_edge("tx-pending", None, None),
                ],
                false,
            ));
        });
        let client = GraphQlClient::new_with_flavour(
            &format!("{}/graphql", server.base_url()),
            GatewayFlavour::Irys,
        );
        let items = client.list_anchored(&[]).await.unwrap();
        assert_eq!(items[0].arweave_tx, "tx-old");
        assert_eq!(items[1].arweave_tx, "tx-new");
        assert_eq!(items[2].arweave_tx, "tx-pending"); // None → i64::MAX → last
    }

    #[tokio::test]
    async fn paginates_until_last_page() {
        let server = MockServer::start();
        // First page: hasNextPage=true → client sends after=cur-tx2 next.
        server.mock(|when, then| {
            when.method(POST)
                .path("/graphql")
                .body_includes("\"after\":null");
            then.status(200).json_body(page(
                vec![
                    arweave_edge("tx1", Some(1_700_000_000), None),
                    arweave_edge("tx2", Some(1_700_000_100), Some("did:sol:alice")),
                ],
                true,
            ));
        });
        server.mock(|when, then| {
            when.method(POST).path("/graphql").body_includes("cur-tx2");
            then.status(200)
                .json_body(page(vec![arweave_edge("tx3", None, None)], false));
        });

        let client = GraphQlClient::new_with_flavour(
            &format!("{}/graphql", server.base_url()),
            GatewayFlavour::Arweave,
        );
        let items = client.list_anchored(&[]).await.unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].arweave_tx, "tx1");
        assert_eq!(items[1].producer.as_deref(), Some("did:sol:alice"));
        assert_eq!(items[2].block_time, None);
    }

    #[tokio::test]
    async fn owner_filter_is_inlined_in_query() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            // The query string is JSON-encoded inside the POST body, so the
            // inlined GraphQL list literal arrives with escaped quotes.
            when.method(POST)
                .path("/graphql")
                .body_includes("owners: [\\\"addr-A\\\"]");
            then.status(200).json_body(page(vec![], false));
        });
        let client = GraphQlClient::new_with_flavour(
            &format!("{}/graphql", server.base_url()),
            GatewayFlavour::Arweave,
        );
        let items = client.list_anchored(&["addr-A".to_string()]).await.unwrap();
        assert!(items.is_empty());
        mock.assert();
    }

    #[tokio::test]
    async fn graphql_errors_surface_as_err() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200)
                .json_body(serde_json::json!({"errors": [{"message": "boom"}]}));
        });
        let client = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
        assert!(client.list_anchored(&[]).await.is_err());
    }
}

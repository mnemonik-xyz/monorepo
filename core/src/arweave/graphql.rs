//! Arweave gateway GraphQL client — enumerates anchored mnemonic-protocol
//! data items so traction stats survive a total node-database loss.
//!
//! Every `participate` write uploads a COSE_Sign1 envelope as an ANS-104
//! item signed by the server's Solana keypair and tagged
//! `App-Name: mnemonic-protocol` (see `ArweaveClient::write_irys` /
//! `write_item`). Gateways (arweave.net, goldsky) index those items, so a
//! paginated GraphQL query filtered by owner + tag is a complete, permanent
//! ledger of everything this node ever anchored.

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

/// Derive the Arweave owner address of an ANS-104 item signed by an Ed25519
/// (Solana) key: `base64url_nopad(sha256(pubkey_bytes))` — the arbundles
/// `ownerToAddress` rule. This lets operators configure the familiar base58
/// Solana wallet address while we query the gateway by its Arweave form.
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
    client: reqwest::Client,
}

impl GraphQlClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            client: super::http_client(),
        }
    }

    /// Enumerate all anchored items, oldest-first. `owner_addresses` are
    /// Arweave-form addresses (see [`solana_pubkey_to_arweave_address`]);
    /// empty = no owner filter (tag-only — fine for a private App-Name).
    pub async fn list_anchored(
        &self,
        owner_addresses: &[String],
    ) -> anyhow::Result<Vec<AnchoredItem>> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let page = self.fetch_page(owner_addresses, cursor.as_deref()).await?;
            let PageResult {
                items: page_items,
                next_cursor,
            } = page;
            items.extend(page_items);
            match next_cursor {
                Some(c) => cursor = Some(c),
                None => return Ok(items),
            }
        }
        anyhow::bail!("gateway pagination exceeded {MAX_PAGES} pages — aborting")
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
        let query = format!(
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
        );
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
        parse_page(&json)
    }
}

struct PageResult {
    items: Vec<AnchoredItem>,
    /// Cursor of the last edge when the gateway reports another page.
    next_cursor: Option<String>,
}

fn parse_page(json: &serde_json::Value) -> anyhow::Result<PageResult> {
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
        let block_time = node["block"]["timestamp"].as_i64();
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

    let has_next = tx["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
    // A gateway claiming hasNextPage with an empty/cursorless page would
    // loop forever — treat it as the final page instead.
    let next_cursor = if has_next { last_cursor } else { None };
    Ok(PageResult { items, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn edge(id: &str, ts: Option<i64>, producer: Option<&str>) -> serde_json::Value {
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
                    edge("tx1", Some(1_700_000_000), None),
                    edge("tx2", Some(1_700_000_100), Some("did:sol:alice")),
                ],
                true,
            ));
        });
        server.mock(|when, then| {
            when.method(POST).path("/graphql").body_includes("cur-tx2");
            then.status(200)
                .json_body(page(vec![edge("tx3", None, None)], false));
        });

        let client = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
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
        let client = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
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

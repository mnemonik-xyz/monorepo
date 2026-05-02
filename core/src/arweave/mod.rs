//! Arweave client -- arlocal (local) + Irys (production) via HTTP.
//!
//! Production uploads go to Irys (uploader.irys.xyz) as signed ANS-104 bundle
//! items using the server's Solana Ed25519 keypair.  Local uploads (arlocal)
//! use unsigned stub transactions -- no signing needed for dev/test.

use anyhow::Context;
use base64::Engine;
use sha2::{Digest, Sha256, Sha384};
use solana_sdk::signature::{Keypair, Signer};

pub struct ArweaveClient {
    base_url: String,
    upload_url: String,
    bypass_local_routing: bool,
    client: reqwest::Client,
}

impl ArweaveClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            // Irys ANS-104 upload endpoint. Per the bundler API,
            // path is `/tx/<token>` where <token> identifies the
            // signing currency. Solana-signed data items go to
            // `/tx/solana`. The legacy `/upload` path returns 404
            // — likely renamed when Bundlr migrated to Irys L1.
            upload_url: "https://uploader.irys.xyz/tx/solana".to_string(),
            bypass_local_routing: false,
            client: reqwest::Client::new(),
        }
    }

    #[cfg(test)]
    pub fn new_for_test(base_url: String) -> Self {
        let upload_url = format!("{}/upload", base_url);
        Self {
            base_url,
            upload_url,
            bypass_local_routing: true,
            client: reqwest::Client::new(),
        }
    }

    /// Write string payload to Arweave.
    pub async fn write(&self, payload: &str, keypair: &Keypair) -> anyhow::Result<String> {
        if !self.bypass_local_routing && self.is_local() {
            self.write_arlocal(payload.as_bytes()).await
        } else {
            self.write_irys(keypair, payload.as_bytes()).await
        }
    }

    /// Write raw bytes to Arweave (arlocal in dev, Irys in prod).
    /// Used for COSE_Sign1 encoded artifacts.
    pub async fn write_bytes(&self, data: &[u8], keypair: &Keypair) -> anyhow::Result<String> {
        if !self.bypass_local_routing && self.is_local() {
            self.write_arlocal(data).await
        } else {
            self.write_irys(keypair, data).await
        }
    }

    pub async fn read(&self, tx_id: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/{tx_id}", self.base_url);
        let resp = self.client.get(&url).send().await.context("arweave read")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("arweave tx not found: {tx_id}");
        }
        resp.error_for_status_ref().context("arweave read status")?;
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn mine(&self) -> anyhow::Result<()> {
        if self.is_local() {
            self.client
                .get(format!("{}/mine", self.base_url))
                .send()
                .await?;
        }
        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/info", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn is_local(&self) -> bool {
        self.base_url.contains("localhost") || self.base_url.contains("127.0.0.1")
    }

    // -- Local (arlocal) --

    async fn write_arlocal(&self, data: &[u8]) -> anyhow::Result<String> {
        let b64url = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let mut sig_bytes = vec![0u8; 512];
        prng_fill(&mut sig_bytes);
        let id_hash = Sha256::digest(&sig_bytes);
        let id = b64url.encode(id_hash);

        let mut owner = vec![0u8; 256];
        prng_fill(&mut owner);

        let data_root = Sha256::digest(data);

        let tx = serde_json::json!({
            "format": 2,
            "id": id,
            "last_tx": "",
            "owner": b64url.encode(&owner),
            "tags": [
                {"name": b64url.encode(b"Content-Type"), "value": b64url.encode(b"application/json")},
                {"name": b64url.encode(b"App-Name"),     "value": b64url.encode(b"mnemonic-protocol")},
            ],
            "target": "",
            "quantity": "0",
            "data_size": data.len().to_string(),
            "data": b64url.encode(data),
            "data_root": b64url.encode(data_root),
            "reward": "0",
            "signature": b64url.encode(&sig_bytes),
        });

        let resp = self
            .client
            .post(format!("{}/tx", self.base_url))
            .json(&tx)
            .send()
            .await
            .context("arweave POST")?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("arweave write failed: {body}");
        }
        Ok(id)
    }

    // -- Production (Irys) --

    /// Upload a signed ANS-104 data item to Irys.
    async fn write_irys(&self, keypair: &Keypair, data: &[u8]) -> anyhow::Result<String> {
        let tags = [
            ("Content-Type", "application/json"),
            ("App-Name", "mnemonic-protocol"),
        ];
        let item = build_data_item(keypair, data, &tags);

        let resp = self
            .client
            .post(&self.upload_url)
            .header("Content-Type", "application/octet-stream")
            .header("x-token", "solana")
            .body(item)
            .send()
            .await
            .context("irys upload")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("irys upload failed: {status} -- {body}");
        }
        let result: serde_json::Value = resp.json().await?;
        result["id"]
            .as_str()
            .map(|s| s.to_string())
            .context("no id in irys response")
    }
}

// -- ANS-104 data item construction --

fn sha384(data: &[u8]) -> [u8; 48] {
    Sha384::digest(data).into()
}

fn deep_hash_blob(data: &[u8]) -> [u8; 48] {
    let tag = format!("blob{}", data.len());
    let mut h = Sha384::new();
    h.update(tag.as_bytes());
    h.update(data);
    h.finalize().into()
}

fn deep_hash_list(items: &[&[u8]]) -> [u8; 48] {
    let tag = format!("list{}", items.len());
    let mut accum = sha384(tag.as_bytes());
    for &item in items {
        let item_hash = deep_hash_blob(item);
        let mut combined = [0u8; 96];
        combined[..48].copy_from_slice(&accum);
        combined[48..].copy_from_slice(&item_hash);
        accum = sha384(&combined);
    }
    accum
}

fn zigzag_varint(n: usize) -> Vec<u8> {
    let mut val = (n as u64) << 1;
    let mut out = Vec::new();
    loop {
        let b = (val & 0x7f) as u8;
        val >>= 7;
        if val == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
    out
}

fn avro_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut v = zigzag_varint(bytes.len());
    v.extend_from_slice(bytes);
    v
}

fn avro_encode_tags(tags: &[(&str, &str)]) -> Vec<u8> {
    if tags.is_empty() {
        return vec![0x00];
    }
    let mut v = zigzag_varint(tags.len());
    for (name, value) in tags {
        v.extend_from_slice(&avro_string(name));
        v.extend_from_slice(&avro_string(value));
    }
    v.push(0x00);
    v
}

fn build_data_item(keypair: &Keypair, data: &[u8], tags: &[(&str, &str)]) -> Vec<u8> {
    let sig_type: u16 = 3; // SOLANA
    let pubkey = keypair.pubkey().to_bytes();
    let avro_tags = avro_encode_tags(tags);

    let msg = deep_hash_list(&[b"dataitem", b"1", b"3", &pubkey, b"", b"", &avro_tags, data]);

    let sig = keypair.sign_message(&msg);

    let num_tags = tags.len() as u64;
    let tags_bytes_len = avro_tags.len() as u64;

    let mut item = Vec::with_capacity(2 + 64 + 32 + 2 + 16 + avro_tags.len() + data.len());
    item.extend_from_slice(&sig_type.to_le_bytes());
    item.extend_from_slice(sig.as_ref());
    item.extend_from_slice(&pubkey);
    item.push(0);
    item.push(0);
    item.extend_from_slice(&num_tags.to_le_bytes());
    item.extend_from_slice(&tags_bytes_len.to_le_bytes());
    item.extend_from_slice(&avro_tags);
    item.extend_from_slice(data);
    item
}

fn prng_fill(buf: &mut [u8]) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut state = seed;
    for byte in buf.iter_mut() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *byte = (state >> 33) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use solana_sdk::signature::Keypair;

    #[tokio::test]
    async fn test_write_success() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/upload");
            then.status(200).body(r#"{"id":"test-tx-id-123"}"#);
        });

        let client = ArweaveClient::new_for_test(server.base_url());
        let keypair = Keypair::new();
        let result = client.write("test payload", &keypair).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-tx-id-123");
        mock.assert();
    }

    #[tokio::test]
    async fn test_read_success() {
        let server = MockServer::start();
        let tx_id = "some-tx-id";
        server.mock(|when, then| {
            when.method(GET).path(format!("/{}", tx_id));
            then.status(200).body(b"raw bytes content");
        });

        let client = ArweaveClient::new(&server.base_url());
        let result = client.read(tx_id).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"raw bytes content");
    }

    #[tokio::test]
    async fn test_write_bytes_success() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/upload");
            then.status(200).body(r#"{"id":"bytes-tx-456"}"#);
        });

        let client = ArweaveClient::new_for_test(server.base_url());
        let keypair = Keypair::new();
        let result = client.write_bytes(b"binary data", &keypair).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "bytes-tx-456");
        mock.assert();
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/info");
            then.status(200);
        });

        let client = ArweaveClient::new(&server.base_url());
        assert!(client.health_check().await);
    }

    #[tokio::test]
    async fn test_network_timeout() {
        // Use a port that nothing listens on
        let client = ArweaveClient::new("http://127.0.0.1:1");
        assert!(!client.health_check().await);

        let keypair = Keypair::new();
        let result = client.write_bytes(b"data", &keypair).await;
        // arlocal path will be taken (is_local=true, bypass=false) and also fail
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_malformed_json_response() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/upload");
            then.status(200).body("not valid json at all");
        });

        let client = ArweaveClient::new_for_test(server.base_url());
        let keypair = Keypair::new();
        let result = client.write("test", &keypair).await;
        assert!(result.is_err());
    }
}

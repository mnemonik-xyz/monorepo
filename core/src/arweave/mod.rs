//! Arweave client -- arlocal (local) + Irys (production) via HTTP.
//!
//! Production uploads go to Irys (uploader.irys.xyz) as signed ANS-104 bundle
//! items using the server's Solana Ed25519 keypair.  Local uploads (arlocal)
//! use unsigned stub transactions -- no signing needed for dev/test.

pub mod arlocal;
pub mod graphql;
pub mod recovery;

use anyhow::Context;

/// Shared reqwest client with an explicit User-Agent. Irys gateways sit
/// behind a WAF that 403s certain default agents (verified live:
/// `Python-urllib` blocked, named agents pass) — an identifiable UA keeps
/// payload fetches and uploads out of that filter.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("mnemonic-core/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

use sha2::{Digest, Sha384};
use solana_sdk::signature::{Keypair, Signer};

/// Irys network used for ANS-104 uploads.
///
/// The upload endpoint is deliberately selected from this enum rather than
/// accepted as an arbitrary environment URL. In particular, a test-only MCP
/// must not be able to turn a Devnet deployment into a mainnet upload through
/// a copied or mistyped endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrysNetwork {
    Mainnet,
    Devnet,
}

impl IrysNetwork {
    fn upload_url(self) -> &'static str {
        match self {
            Self::Mainnet => "https://uploader.irys.xyz/tx/solana",
            Self::Devnet => "https://devnet.irys.xyz/tx/solana",
        }
    }
}

pub struct ArweaveClient {
    base_url: String,
    upload_url: String,
    network: IrysNetwork,
    bypass_local_routing: bool,
    client: reqwest::Client,
}

impl ArweaveClient {
    /// Construct an Irys-backed client for an explicit network.
    ///
    /// `base_url` remains the read gateway. Uploads always use the fixed,
    /// network-specific ANS-104 endpoint selected by `network`.
    pub fn new_with_network(base_url: &str, network: IrysNetwork) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            upload_url: network.upload_url().to_string(),
            network,
            bypass_local_routing: false,
            client: http_client(),
        }
    }

    pub fn new(base_url: &str) -> Self {
        Self::new_with_network(base_url, IrysNetwork::Mainnet)
    }

    /// Configured Irys read gateway, without a trailing slash.
    pub fn gateway_url(&self) -> &str {
        &self.base_url
    }

    /// Irys network selected for uploads and data links.
    pub fn network(&self) -> IrysNetwork {
        self.network
    }

    #[cfg(test)]
    pub fn new_for_test(base_url: String) -> Self {
        let upload_url = format!("{}/upload", base_url);
        Self {
            base_url,
            upload_url,
            network: IrysNetwork::Mainnet,
            bypass_local_routing: true,
            client: reqwest::Client::new(),
        }
    }

    /// Write string payload to Arweave.
    pub async fn write(&self, payload: &str, keypair: &Keypair) -> anyhow::Result<String> {
        if !self.bypass_local_routing && self.is_local() {
            self.write_arlocal(payload.as_bytes(), keypair).await
        } else {
            self.write_irys(keypair, payload.as_bytes()).await
        }
    }

    /// Write raw bytes to Arweave (arlocal in dev, Irys in prod).
    /// Used for COSE_Sign1 encoded artifacts.
    pub async fn write_bytes(&self, data: &[u8], keypair: &Keypair) -> anyhow::Result<String> {
        if !self.bypass_local_routing && self.is_local() {
            self.write_arlocal(data, keypair).await
        } else {
            self.write_irys(keypair, data).await
        }
    }

    /// Write a tagged ANS-104 data item (Irys) / arlocal stub. The base
    /// `App-Name` / `Content-Type` tags are always present; `extra_tags` (e.g.
    /// `trajectory_id`, `seq`, `content_hash`) are appended so the item is
    /// retrievable via a GraphQL tag query. Returns the tx id.
    pub async fn write_item(
        &self,
        data: &[u8],
        keypair: &Keypair,
        extra_tags: &[(&str, &str)],
    ) -> anyhow::Result<String> {
        let mut tags: Vec<(&str, &str)> = vec![
            ("Content-Type", "application/octet-stream"),
            ("App-Name", "mnemonic-protocol"),
        ];
        tags.extend_from_slice(extra_tags);

        if !self.bypass_local_routing && self.is_local() {
            // arlocal dev path: post a signed ANS-104 data item the same way
            // Irys accepts it. arlocal's /tx/solana endpoint validates the
            // signature and data root, so the previous unsigned JSON stub no
            // longer works with modern arlocal versions.
            return self.write_arlocal(data, keypair).await;
        }
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

    async fn write_arlocal(&self, data: &[u8], _keypair: &Keypair) -> anyhow::Result<String> {
        let (tx, id, address) = arlocal::build_signed_transaction(data)?;

        // Fund the derived Arweave wallet on arlocal. The mint endpoint is
        // idempotent; calling it before every upload keeps the local client
        // self-sufficient.
        let _ = self
            .client
            .get(format!("{}/mint/{address}/1000000000000", self.base_url))
            .send()
            .await
            .context("arlocal mint")?;

        let resp = self
            .client
            .post(format!("{}/tx", self.base_url))
            .json(&tx)
            .send()
            .await
            .context("arlocal POST")?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("arlocal write failed: {body}");
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
    // Per arweave-js deepHash spec:
    //   sha384( sha384("blob" + len_str) || sha384(data) )
    // Hashing tag and data SEPARATELY then combining is required —
    // hashing the concatenation in one pass produces a different digest
    // and Irys rejects with "Invalid signature".
    let tag = format!("blob{}", data.len());
    let tag_hash = sha384(tag.as_bytes());
    let data_hash = sha384(data);
    let mut combined = [0u8; 96];
    combined[..48].copy_from_slice(&tag_hash);
    combined[48..].copy_from_slice(&data_hash);
    sha384(&combined)
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
    // Irys's SolanaSigner extends Curve25519, which sets signatureType=2
    // (ED25519). The SOLANA=4 enum exists in @irys/bundles/constants.ts
    // but is NEVER used by the Solana signer at runtime — sig_type=4 is
    // routed to a different verifier and rejected as "Invalid signature".
    // Solana keys ARE Ed25519 keys; sig_type=2 is the right wire value.
    // Ref: @irys/bundles/src/signing/keys/curve25519.ts
    let sig_type: u16 = 2;
    let pubkey = keypair.pubkey().to_bytes();
    let avro_tags = avro_encode_tags(tags);

    let msg = deep_hash_list(&[b"dataitem", b"1", b"2", &pubkey, b"", b"", &avro_tags, data]);

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

    #[test]
    fn devnet_uses_the_devnet_upload_endpoint() {
        let client =
            ArweaveClient::new_with_network("https://devnet.irys.xyz", IrysNetwork::Devnet);
        assert_eq!(client.base_url, "https://devnet.irys.xyz");
        assert_eq!(client.upload_url, "https://devnet.irys.xyz/tx/solana");
    }

    /// Sign a data item, then re-derive the deep hash from the buffer's
    /// own owner+tags+data fields and verify the signature against it.
    /// If this fails, our build_data_item is internally broken (signing
    /// over different bytes than what Irys would re-derive from the
    /// buffer it receives). If it passes but Irys still says "Invalid
    /// signature", the divergence is in the deep_hash formula vs. the
    /// arweave-js spec.
    #[test]
    fn signature_self_verifies() {
        use solana_sdk::signature::Signature;
        let kp = Keypair::new();
        let data = b"hello world";
        let tags = [
            ("Content-Type", "application/json"),
            ("App-Name", "mnemonic-protocol"),
        ];
        let item = build_data_item(&kp, data, &tags);

        // Re-derive owner + tags from the buffer (mirroring how Irys
        // parses rawOwner / rawTags from the item it receives).
        let sig_bytes = &item[2..66];
        let owner = &item[66..98];
        assert_eq!(item[98], 0, "target flag must be 0");
        assert_eq!(item[99], 0, "anchor flag must be 0");
        let num_tags = u64::from_le_bytes(item[100..108].try_into().unwrap());
        let tags_bytes_size = u64::from_le_bytes(item[108..116].try_into().unwrap());
        assert_eq!(num_tags, tags.len() as u64);
        let tags_end = 116 + tags_bytes_size as usize;
        let raw_tags = &item[116..tags_end];
        let raw_data = &item[tags_end..];
        assert_eq!(raw_data, data);

        // Compute the deep hash that Irys would compute for this buffer.
        // sig_type=2 (ED25519) → ASCII "2" — matches Curve25519 signer.
        let msg = deep_hash_list(&[b"dataitem", b"1", b"2", owner, b"", b"", raw_tags, raw_data]);

        let sig = Signature::try_from(sig_bytes).expect("parse sig");
        assert!(
            sig.verify(owner, &msg),
            "data item signature failed self-verification — build_data_item is signing over different bytes than what re-derivation produces"
        );
    }

    /// Diagnostic: dump a real data item to /tmp/item.bin and print hex
    /// for each field. Then we verify it externally with pynacl using
    /// Irys's exact verification path. Run with --nocapture.
    #[test]
    fn dump_data_item_for_external_verification() {
        let kp = Keypair::new();
        let data = b"hello world";
        let tags = [
            ("Content-Type", "application/json"),
            ("App-Name", "mnemonic-protocol"),
        ];
        let item = build_data_item(&kp, data, &tags);
        let pubkey = kp.pubkey().to_bytes();

        std::fs::write("/tmp/item.bin", &item).unwrap();
        std::fs::write("/tmp/item_pubkey.bin", pubkey).unwrap();
        std::fs::write("/tmp/item_data.bin", data).unwrap();

        println!("ITEM_LEN={}", item.len());
        println!("PUBKEY_HEX={}", hex::encode(pubkey));
        println!("SIG_HEX={}", hex::encode(&item[2..66]));
        println!("OWNER_IN_BUFFER={}", hex::encode(&item[66..98]));
        println!("TARGET_FLAG={}", item[98]);
        println!("ANCHOR_FLAG={}", item[99]);
        println!("NUM_TAGS_LE_BYTES={}", hex::encode(&item[100..108]));
        println!("NUM_TAGS_BYTES_LE={}", hex::encode(&item[108..116]));
        let tags_size = u64::from_le_bytes(item[108..116].try_into().unwrap()) as usize;
        println!("TAGS_HEX={}", hex::encode(&item[116..116 + tags_size]));
        println!("DATA_HEX={}", hex::encode(&item[116 + tags_size..]));
    }

    /// Cross-language oracle: a pure-python mirror of arweave-js deepHash
    /// computed for fixed inputs (pubkey=32 zeros, data="hello world",
    /// tags=[("Content-Type","application/json"),("App-Name","mnemonic-protocol")],
    /// sigType=4). Our Rust deep_hash MUST produce the same digest, otherwise
    /// our signature is over the wrong message and Irys will reject as
    /// "Invalid signature".
    #[test]
    fn deep_hash_matches_python_reference() {
        let pubkey = [0u8; 32];
        let data = b"hello world";
        let tags = [
            ("Content-Type", "application/json"),
            ("App-Name", "mnemonic-protocol"),
        ];
        let avro_tags = avro_encode_tags(&tags);

        // Sanity: avro_tags must match the Python reference byte-for-byte.
        let expected_avro = hex::decode(
            "0418436f6e74656e742d54797065206170706c69636174696f6e2f6a736f6e104170702d4e616d65226d6e656d6f6e69632d70726f746f636f6c00",
        ).unwrap();
        assert_eq!(
            avro_tags, expected_avro,
            "avro_encode_tags diverges from spec"
        );

        let dh = deep_hash_list(&[b"dataitem", b"1", b"2", &pubkey, b"", b"", &avro_tags, data]);
        // Reference value from /tmp/deephash_ref.py with sigType=2.
        let expected =
            hex::decode("ec8618225e5424fef34953635059619fdb0ac65ef2d091133bd3ea86d48f5b1b84b2d620a6da895b21e517166511697b")
                .unwrap();
        assert_eq!(
            &dh[..],
            &expected[..],
            "deep_hash diverges from arweave-js reference — Irys will reject"
        );
    }

    /// Spec-grounded reference test for the blob branch of deep_hash:
    ///   sha384( sha384("blob" + len_str) || sha384(data) )
    /// Computed with stdlib sha384 to catch any future regression in
    /// deep_hash_blob.
    #[test]
    fn deep_hash_blob_matches_spec() {
        let data = b"hello";
        let tag_hash: [u8; 48] = Sha384::digest(b"blob5").into();
        let data_hash: [u8; 48] = Sha384::digest(data).into();
        let mut buf = [0u8; 96];
        buf[..48].copy_from_slice(&tag_hash);
        buf[48..].copy_from_slice(&data_hash);
        let expected: [u8; 48] = Sha384::digest(buf).into();
        assert_eq!(deep_hash_blob(data), expected);
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

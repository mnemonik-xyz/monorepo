//! Arweave client — arlocal (local) + Irys (production) via HTTP.
//!
//! Production uploads go to Irys (uploader.irys.xyz) as signed ANS-104 bundle
//! items using the server's Solana Ed25519 keypair.  Local uploads (arlocal)
//! use unsigned stub transactions — no signing needed for dev/test.

use anyhow::Context;
use base64::Engine;
use sha2::{Digest, Sha256, Sha384};
use solana_sdk::signature::{Keypair, Signer};

pub struct ArweaveClient {
    base_url: String,
    client: reqwest::Client,
}

impl ArweaveClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Write string payload to Arweave.
    pub async fn write(&self, payload: &str, keypair: &Keypair) -> anyhow::Result<String> {
        self.write_bytes(payload.as_bytes(), keypair).await
    }

    /// Write raw bytes to Arweave (arlocal in dev, Irys in prod).
    /// Used for COSE_Sign1 encoded artifacts.
    pub async fn write_bytes(&self, data: &[u8], keypair: &Keypair) -> anyhow::Result<String> {
        if self.is_local() {
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
                .get(&format!("{}/mine", self.base_url))
                .send()
                .await?;
        }
        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        self.client
            .get(&format!("{}/info", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn is_local(&self) -> bool {
        self.base_url.contains("localhost") || self.base_url.contains("127.0.0.1")
    }

    // ── Local (arlocal) ───────────────────────────────────────────────────────

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
            .post(&format!("{}/tx", self.base_url))
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

    // ── Production (Irys) ────────────────────────────────────────────────────

    /// Upload a signed ANS-104 data item to Irys.
    ///
    /// The data item is signed with the server's Solana keypair using the
    /// standard Irys/Bundlr signing scheme (deep-hash + Ed25519).
    async fn write_irys(&self, keypair: &Keypair, data: &[u8]) -> anyhow::Result<String> {
        let tags = [
            ("Content-Type", "application/json"),
            ("App-Name", "mnemonic-protocol"),
        ];
        let item = build_data_item(keypair, data, &tags);

        let resp = self
            .client
            .post("https://uploader.irys.xyz/upload")
            .header("Content-Type", "application/octet-stream")
            .header("x-token", "solana")
            .body(item)
            .send()
            .await
            .context("irys upload")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("irys upload failed: {status} — {body}");
        }
        let result: serde_json::Value = resp.json().await?;
        result["id"]
            .as_str()
            .map(|s| s.to_string())
            .context("no id in irys response")
    }
}

// ── ANS-104 data item construction ───────────────────────────────────────────
//
// Implements the Irys/Bundlr bundle item format (arbundles ANS-104 spec).
// Signer type 3 = SOLANA (Ed25519, 64-byte sig, 32-byte pubkey).
// https://github.com/ArweaveTeam/arweave-standards/blob/master/ans/ANS-104.md

/// SHA-384 helper (Irys deep hash uses SHA-384 throughout).
fn sha384(data: &[u8]) -> [u8; 48] {
    Sha384::digest(data).into()
}

/// Deep hash of a single blob:  SHA-384("blob" + str(len) + data)
fn deep_hash_blob(data: &[u8]) -> [u8; 48] {
    let tag = format!("blob{}", data.len());
    let mut h = Sha384::new();
    h.update(tag.as_bytes());
    h.update(data);
    h.finalize().into()
}

/// Deep hash of a list of blobs:
///   accum = SHA-384("list" + str(count))
///   for each item: accum = SHA-384(accum || deep_hash_blob(item))
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

/// Avro zigzag-varint encoding (positive integers only).
fn zigzag_varint(n: usize) -> Vec<u8> {
    let mut val = (n as u64) << 1; // zigzag(n) = 2n for n >= 0
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

/// Avro binary-encoded string: zigzag(len) + utf8 bytes.
fn avro_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut v = zigzag_varint(bytes.len());
    v.extend_from_slice(bytes);
    v
}

/// Avro binary array of Tag records {name: string, value: string}.
fn avro_encode_tags(tags: &[(&str, &str)]) -> Vec<u8> {
    if tags.is_empty() {
        return vec![0x00]; // Avro: empty array = single zero byte
    }
    let mut v = zigzag_varint(tags.len()); // block count
    for (name, value) in tags {
        v.extend_from_slice(&avro_string(name));
        v.extend_from_slice(&avro_string(value));
    }
    v.push(0x00); // Avro end-of-array sentinel
    v
}

/// Build a signed ANS-104 data item for Irys (Solana signer type 3).
fn build_data_item(keypair: &Keypair, data: &[u8], tags: &[(&str, &str)]) -> Vec<u8> {
    let sig_type: u16 = 3; // SOLANA
    let pubkey = keypair.pubkey().to_bytes(); // 32 bytes
    let avro_tags = avro_encode_tags(tags);

    // Signing message: deep hash of all data item fields
    let msg = deep_hash_list(&[
        b"dataitem",   // fixed literal
        b"1",          // ANS-104 version
        b"3",          // signature type as ASCII string
        &pubkey,       // 32-byte owner public key
        b"",           // target: absent (empty slice)
        b"",           // anchor: absent
        &avro_tags,    // Avro-encoded tags
        data,          // payload
    ]);

    let sig = keypair.sign_message(&msg); // Ed25519 over the deep-hash

    let num_tags = tags.len() as u64;
    let tags_bytes_len = avro_tags.len() as u64;

    let mut item = Vec::with_capacity(2 + 64 + 32 + 2 + 16 + avro_tags.len() + data.len());
    item.extend_from_slice(&sig_type.to_le_bytes());        // [0..2]    sig type
    item.extend_from_slice(sig.as_ref());                   // [2..66]   signature (64 B)
    item.extend_from_slice(&pubkey);                        // [66..98]  pubkey (32 B)
    item.push(0);                                           // [98]      target: absent
    item.push(0);                                           // [99]      anchor: absent
    item.extend_from_slice(&num_tags.to_le_bytes());        // [100..108] tag count
    item.extend_from_slice(&tags_bytes_len.to_le_bytes());  // [108..116] tag bytes len
    item.extend_from_slice(&avro_tags);                     // tags payload
    item.extend_from_slice(data);                           // data payload
    item
}

// ── arlocal PRNG (non-crypto, stub fields only) ───────────────────────────────

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

//! Helpers for posting real, signed Arweave format-2 transactions to arlocal.
//!
//! Modern arlocal validates the transaction signature, id, and data_root, so
//! the previous unsigned JSON stub no longer works. This module constructs a
//! minimal format-2 transaction, computes the data_root, deep-hash, and an
//! RSA-PSS-SHA256 signature, and returns the JSON body expected by arlocal's
//! `POST /tx` endpoint.
//!
//! Only small payloads (one chunk) are supported, which covers the CBOR
//! artifacts the MCP uploads during local e2e runs.

use anyhow::Context;
use base64::Engine;
use rsa::{
    pss::SigningKey,
    signature::{RandomizedSigner, SignatureEncoding},
    traits::PublicKeyParts,
    RsaPrivateKey,
};
use sha2::{Digest, Sha256, Sha384};
use std::sync::OnceLock;

const NOTE_SIZE: usize = 32;
const HASH_SIZE: usize = 32;

fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256(data: &[u8]) -> [u8; HASH_SIZE] {
    Sha256::digest(data).into()
}

fn sha384(data: &[u8]) -> [u8; 48] {
    Sha384::digest(data).into()
}

fn int_to_buffer(note: usize) -> [u8; NOTE_SIZE] {
    let mut buffer = [0u8; NOTE_SIZE];
    let mut n = note;
    for i in (0..NOTE_SIZE).rev() {
        let byte = (n % 256) as u8;
        buffer[i] = byte;
        n = (n - byte as usize) / 256;
    }
    buffer
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(a.len() + b.len());
    v.extend_from_slice(a);
    v.extend_from_slice(b);
    v
}

/// Compute the data_root for small data that fits in a single chunk.
fn compute_data_root(data: &[u8]) -> [u8; HASH_SIZE] {
    // chunkData for small data produces a single chunk where
    // dataHash = sha256(data). The leaf id is:
    //   hash( hash(dataHash) || hash(intToBuffer(maxByteRange)) )
    // where hash = sha256. Hence data_root = sha256( sha256(sha256(data)) || sha256(intToBuffer(len)) ).
    let data_hash_hash = sha256(&sha256(data));
    let range_buf_hash = sha256(&int_to_buffer(data.len()));
    sha256(&concat(&data_hash_hash, &range_buf_hash))
}

fn deep_hash_blob(data: &[u8]) -> [u8; 48] {
    let tag = format!("blob{}", data.len());
    let tag_hash = sha384(tag.as_bytes());
    let data_hash = sha384(data);
    sha384(&concat(&tag_hash, &data_hash))
}

fn list_tag(n: usize) -> [u8; 48] {
    sha384(format!("list{}", n).as_bytes())
}

fn combine_hashes(a: [u8; 48], b: [u8; 48]) -> [u8; 48] {
    sha384(&concat(&a, &b))
}

/// Deep-hash a list whose items are already deep hashes.
fn deep_hash_list_of_hashes(items: &[[u8; 48]]) -> [u8; 48] {
    let mut acc = list_tag(items.len());
    for item in items {
        acc = combine_hashes(acc, *item);
    }
    acc
}

/// Deep-hash a list of raw byte buffers.
fn deep_hash_list_of_blobs(items: &[&[u8]]) -> [u8; 48] {
    let hashes: Vec<[u8; 48]> = items.iter().map(|b| deep_hash_blob(b)).collect();
    deep_hash_list_of_hashes(&hashes)
}

/// Deep-hash the format-2 transaction fields the same way arweave-js does.
///
/// The argument list mirrors arweave-js's field order deliberately — grouping
/// them into a struct would obscure the correspondence this function exists to
/// preserve.
#[allow(clippy::too_many_arguments)]
fn transaction_deep_hash(
    owner: &[u8],
    target: &[u8],
    quantity: &str,
    reward: &str,
    last_tx: &[u8],
    tags: &[(String, String)],
    data_size: &str,
    data_root: &[u8],
) -> [u8; 48] {
    // Each tag is itself a list [name, value].
    let tag_hashes: Vec<[u8; 48]> = tags
        .iter()
        .map(|(name, value)| deep_hash_list_of_blobs(&[name.as_bytes(), value.as_bytes()]))
        .collect();
    let tags_hash = deep_hash_list_of_hashes(&tag_hashes);

    let item_hashes = [
        deep_hash_blob(b"2"),
        deep_hash_blob(owner),
        deep_hash_blob(target),
        deep_hash_blob(quantity.as_bytes()),
        deep_hash_blob(reward.as_bytes()),
        deep_hash_blob(last_tx),
        tags_hash,
        deep_hash_blob(data_size.as_bytes()),
        deep_hash_blob(data_root),
    ];

    deep_hash_list_of_hashes(&item_hashes)
}

fn cached_rsa_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut rng = rand::thread_rng();
        RsaPrivateKey::new(&mut rng, 4096).expect("failed to generate arlocal RSA key")
    })
}

fn arweave_address(owner_n: &[u8]) -> String {
    b64url_encode(&sha256(owner_n))
}

/// Build a signed arlocal format-2 transaction JSON body for the given data.
pub fn build_signed_transaction(
    data: &[u8],
) -> anyhow::Result<(serde_json::Value, String, String)> {
    let key = cached_rsa_key();
    let n = key.to_public_key().n().to_bytes_be();
    let owner_b64 = b64url_encode(&n);
    let address = arweave_address(&n);

    let data_root = compute_data_root(data);
    let data_root_b64 = b64url_encode(&data_root);
    let data_b64 = b64url_encode(data);

    let tags = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("App-Name".to_string(), "mnemonic-protocol".to_string()),
    ];

    let tags_json: Vec<serde_json::Value> = tags
        .iter()
        .map(|(name, value)| {
            serde_json::json!({
                "name": b64url_encode(name.as_bytes()),
                "value": b64url_encode(value.as_bytes()),
            })
        })
        .collect();

    let to_sign = transaction_deep_hash(
        &n,
        b"",
        "0",
        "0",
        b"",
        &tags,
        &data.len().to_string(),
        &data_root,
    );

    let signing_key = SigningKey::<Sha256>::new(key.clone());
    let mut rng = rand::thread_rng();
    let signature = signing_key
        .try_sign_with_rng(&mut rng, &to_sign)
        .context("RSA-PSS sign failed")?
        .to_bytes();
    let sig_b64 = b64url_encode(&signature);
    let id = b64url_encode(&sha256(&signature));

    let tx = serde_json::json!({
        "format": 2,
        "id": id,
        "last_tx": "",
        "owner": owner_b64,
        "tags": tags_json,
        "target": "",
        "quantity": "0",
        "data_size": data.len().to_string(),
        "data": data_b64,
        "data_root": data_root_b64,
        "reward": "0",
        "signature": sig_b64,
    });

    Ok((tx, id, address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_root_is_32_bytes() {
        let root = compute_data_root(b"hello");
        assert_eq!(root.len(), 32);
    }

    #[test]
    fn signed_transaction_fields_present() {
        let (tx, id, address) = build_signed_transaction(b"hello world").unwrap();
        assert_eq!(tx["id"].as_str().unwrap(), id);
        assert_eq!(tx["format"], 2);
        assert!(tx["owner"].as_str().unwrap().len() > 0);
        assert!(tx["signature"].as_str().unwrap().len() > 0);
        assert!(tx["data_root"].as_str().unwrap().len() > 0);
        assert!(address.len() > 0);
    }
}

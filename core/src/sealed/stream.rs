//! Complete sealed stream verification. Prefix, index and final flag bind nonces.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamChunk {
    pub index: u32,
    pub last: bool,
    pub prev_hash: String,
    pub ciphertext: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedStream {
    pub stream_id: String,
    pub nonce_prefix: String,
    pub header_hash: String,
    pub chunks: Vec<StreamChunk>,
    pub head: String,
}

fn nonce(prefix: &[u8; 19], index: u32, last: bool) -> [u8; 24] {
    let mut nonce = [0; 24];
    nonce[..19].copy_from_slice(prefix);
    nonce[19..23].copy_from_slice(&index.to_be_bytes());
    nonce[23] = u8::from(last);
    nonce
}

fn aad(stream_id: &str, header_hash: &str, index: u32, last: bool, prev: &str) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "mnemonic.sealed.stream.v1",
        header_hash,
        stream_id,
        index,
        last,
        prev,
    ))?)
}

fn chunk_hash(chunk: &StreamChunk) -> Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(&(
        chunk.index,
        chunk.last,
        &chunk.prev_hash,
        &chunk.ciphertext,
    ))?)
    .to_hex()
    .to_string())
}

/// Seal once with a fresh per-stream prefix. This function never reuses a prefix.
pub fn seal_stream(
    chunks: &[Vec<u8>],
    k: &[u8; 32],
    stream_id: &str,
    header_hash: &str,
    rng: &mut impl rand_core::RngCore,
) -> Result<SealedStream> {
    ensure!(
        !stream_id.is_empty() && !chunks.is_empty() && chunks.len() <= 65536,
        "invalid stream size/id"
    );
    let mut prefix = [0; 19];
    rng.fill_bytes(&mut prefix);
    let mut prev = "00".repeat(32);
    let mut sealed = Vec::new();
    for (idx, plaintext) in chunks.iter().enumerate() {
        let index = u32::try_from(idx)?;
        let last = idx + 1 == chunks.len();
        let ct = super::encrypt_content(
            k,
            &nonce(&prefix, index, last),
            &aad(stream_id, header_hash, index, last, &prev)?,
            plaintext,
        )?;
        let mut c = StreamChunk {
            index,
            last,
            prev_hash: prev,
            ciphertext: STANDARD.encode(ct),
            hash: String::new(),
        };
        c.hash = chunk_hash(&c)?;
        prev = c.hash.clone();
        sealed.push(c);
    }
    Ok(SealedStream {
        stream_id: stream_id.into(),
        nonce_prefix: STANDARD.encode(prefix),
        header_hash: header_hash.into(),
        chunks: sealed,
        head: prev,
    })
}

/// Public hash-chain verification is independent of decryption and K.
pub fn verify_stream_chain(stream: &SealedStream, expected_header_hash: &str) -> Result<()> {
    ensure!(
        !stream.stream_id.is_empty() && !stream.chunks.is_empty() && stream.chunks.len() <= 65536,
        "invalid stream"
    );
    ensure!(
        stream.header_hash == expected_header_hash,
        "stream/header mismatch"
    );
    ensure!(
        STANDARD.decode(&stream.nonce_prefix)?.len() == 19,
        "invalid nonce prefix"
    );
    let mut prev = "00".repeat(32);
    for (idx, chunk) in stream.chunks.iter().enumerate() {
        ensure!(
            chunk.index == u32::try_from(idx)? && chunk.prev_hash == prev,
            "broken sealed chain"
        );
        ensure!(
            chunk.last == (idx + 1 == stream.chunks.len()),
            "missing or premature final chunk"
        );
        ensure!(chunk.hash == chunk_hash(chunk)?, "chunk hash mismatch");
        prev = chunk.hash.clone();
    }
    ensure!(prev == stream.head, "sealed stream head mismatch");
    Ok(())
}

/// Verify the complete chain before releasing any plaintext.
pub fn open_stream(
    stream: &SealedStream,
    k: &[u8; 32],
    expected_header_hash: &str,
) -> Result<Vec<Vec<u8>>> {
    verify_stream_chain(stream, expected_header_hash)?;
    let prefix: [u8; 19] = STANDARD
        .decode(&stream.nonce_prefix)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("nonce prefix"))?;
    let mut out = zeroize::Zeroizing::new(Vec::new());
    for chunk in &stream.chunks {
        let ct = STANDARD
            .decode(&chunk.ciphertext)
            .context("ciphertext base64")?;
        out.push(super::decrypt_content(
            k,
            &nonce(&prefix, chunk.index, chunk.last),
            &aad(
                &stream.stream_id,
                &stream.header_hash,
                chunk.index,
                chunk.last,
                &chunk.prev_hash,
            )?,
            &ct,
        )?);
    }
    Ok(std::mem::take(&mut *out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_chain_detects_order_drop_truncation_splice_and_metadata_tamper() {
        let k = [7; 32];
        let s = seal_stream(
            &[b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            &k,
            "stream",
            "header",
            &mut rand_core::OsRng,
        )
        .unwrap();
        assert_eq!(
            open_stream(&s, &k, "header").unwrap(),
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]
        );
        let mut bad = s.clone();
        bad.chunks.swap(0, 1);
        assert!(open_stream(&bad, &k, "header").is_err());
        let mut bad = s.clone();
        bad.chunks.remove(1);
        assert!(open_stream(&bad, &k, "header").is_err());
        let mut bad = s.clone();
        bad.chunks.pop();
        assert!(open_stream(&bad, &k, "header").is_err());
        // A forged shorter manifest can repair hashes but cannot forge final AEAD.
        let mut bad = s.clone();
        bad.chunks.pop();
        let last = bad.chunks.last_mut().unwrap();
        last.last = true;
        last.hash = chunk_hash(last).unwrap();
        bad.head = last.hash.clone();
        assert!(verify_stream_chain(&bad, "header").is_ok());
        assert!(open_stream(&bad, &k, "header").is_err());
        let mut bad = s.clone();
        bad.stream_id = "other".into();
        assert!(open_stream(&bad, &k, "header").is_err());
        let mut bad = s.clone();
        bad.chunks[0].last = true;
        assert!(open_stream(&bad, &k, "header").is_err());
        // Recompute all public hashes after changing ciphertext: AEAD still rejects.
        let mut bad = s.clone();
        bad.chunks[0].ciphertext = STANDARD.encode([1; 17]);
        let mut prev = "00".repeat(32);
        for chunk in &mut bad.chunks {
            chunk.prev_hash = prev;
            chunk.hash = chunk_hash(chunk).unwrap();
            prev = chunk.hash.clone();
        }
        bad.head = prev;
        assert!(verify_stream_chain(&bad, "header").is_ok());
        assert!(open_stream(&bad, &k, "header").is_err());
        let mut bad = s.clone();
        bad.nonce_prefix = STANDARD.encode([8; 19]);
        assert!(open_stream(&bad, &k, "header").is_err());
        assert!(open_stream(&s, &[8; 32], "header").is_err());
        assert!(open_stream(&s, &k, "other-header").is_err());
        let other = seal_stream(
            &[b"x".to_vec()],
            &k,
            "stream",
            "header",
            &mut rand_core::OsRng,
        )
        .unwrap();
        let mut bad = s.clone();
        bad.chunks[0] = other.chunks[0].clone();
        assert!(open_stream(&bad, &k, "header").is_err());
    }
}

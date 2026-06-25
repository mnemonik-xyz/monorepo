//! Client-side trustless RAG primitives (Wave 3 of project-knowledge-context).
//!
//! Compiled for **all** targets (unlike [`crate::embed`], which is native-only)
//! so the webapp's WASM bundle can run retrieval entirely in-browser: fetch a
//! chunk's COSE_Sign1 bytes from Arweave, verify the signature, decode the
//! canonical-CBOR payload, decompress the TurboQuant embedding, and rank it
//! against a query embedding with cosine similarity — no server trust required.
//!
//! These are thin compositions over existing primitives in `crate::codec` and
//! `crate::compress`; no new crypto or business logic lives here.

use base64::Engine;

use crate::codec::canonical::from_canonical_cbor;
use crate::codec::sign::verify_artifact;
use crate::compress::{CompressedEmbedding, EmbeddingCompressor};

/// Deterministic compressor seed shared across the whole codebase (server,
/// extension, webapp). MUST match `core/src/wasm/mod.rs::COMPRESS_SEED` and
/// `mcp/src/main.rs`. Drift makes server-stored embeddings undecodable here.
pub const COMPRESS_SEED: u64 = 42;

/// Cosine similarity between two equal-length f32 vectors. Returns 0.0 if either
/// vector has zero magnitude or the lengths differ (callers treat that as "no
/// match" rather than a hard error, so a single malformed chunk can't abort a
/// whole retrieval pass).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// A knowledge chunk recovered + verified from its on-chain (Arweave) bytes.
#[derive(Debug, Clone)]
pub struct ExtractedMemory {
    /// Whether the COSE_Sign1 signature + content integrity verified.
    pub valid: bool,
    /// Base58 pubkey that signed the attestation (the `kid`).
    pub signer: String,
    /// The chunk's plaintext content (markdown).
    pub content: String,
    /// The decompressed (approximate) f32 embedding for cosine ranking.
    pub embedding: Vec<f32>,
}

/// Verify a chunk's COSE_Sign1 bytes and extract its content + embedding.
///
/// This is the trustless retrieval core: the caller hands raw bytes fetched
/// from an Arweave gateway and gets back a *verified* chunk. The embedding's
/// dimension and bit-width are read from the compressed bytes themselves, so no
/// out-of-band metadata is required to decompress.
pub fn verify_and_extract_memory(cose_bytes: &[u8]) -> Result<ExtractedMemory, String> {
    let vr = verify_artifact(cose_bytes, None)?;
    let json = from_canonical_cbor(&vr.payload)?;

    let content = json
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let emb_b64 = json
        .get("metadata")
        .and_then(|m| m.get("embedding_compressed"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing metadata.embedding_compressed".to_string())?;

    let emb_bytes = base64::engine::general_purpose::STANDARD
        .decode(emb_b64)
        .map_err(|e| format!("embedding_compressed not valid base64: {e}"))?;

    let compressed = CompressedEmbedding::from_bytes(&emb_bytes)
        .ok_or_else(|| "malformed compressed embedding".to_string())?;
    let compressor = EmbeddingCompressor::new(compressed.dim, compressed.bit_width, COMPRESS_SEED);
    let embedding = compressor.decompress(&compressed);

    Ok(ExtractedMemory {
        valid: vr.valid,
        signer: vr.signer,
        content,
        embedding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_is_one_and_orthogonal_is_zero() {
        let v = vec![0.1, 0.2, 0.3, 0.4];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-5);
        // Length mismatch / zero vector → 0.0, never a panic.
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn extract_roundtrips_a_signed_chunk() {
        use crate::codec::canonical::to_canonical_cbor;
        use crate::codec::schema::MEMORY_V1;
        use crate::codec::sign::sign_cose;
        use solana_sdk::signature::Keypair;

        // Build a compressed embedding exactly as the server would.
        let embedding: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();
        let compressor = EmbeddingCompressor::new(embedding.len(), 4, COMPRESS_SEED);
        let compressed_bytes = compressor.compress(&embedding).to_bytes();
        let emb_b64 = base64::engine::general_purpose::STANDARD.encode(&compressed_bytes);

        let artifact = serde_json::json!({
            "artifact_id": "art:test",
            "type": "memory",
            "schema_version": 1,
            "content": "# WHITEPAPER.md\n\nThe Mnemonic Protocol is verifiable memory.",
            "producer": "did:sol:test",
            "created_at": "2026-06-25T00:00:00+00:00",
            "metadata": { "embedding_compressed": emb_b64 },
        });
        let cbor = to_canonical_cbor(&artifact, &MEMORY_V1).expect("cbor");
        let kp = Keypair::new();
        let cose = sign_cose(&cbor, &kp).expect("cose");

        let extracted = verify_and_extract_memory(&cose).expect("extract");
        assert!(extracted.valid, "signature must verify");
        assert!(extracted.content.contains("Mnemonic Protocol"));
        assert_eq!(extracted.embedding.len(), 16, "embedding dim recovered");
        // The recovered (lossy) embedding should still rank itself top vs noise.
        let noise = vec![0.0f32; 16];
        assert!(
            cosine_similarity(&extracted.embedding, &embedding)
                > cosine_similarity(&extracted.embedding, &noise)
        );
    }
}

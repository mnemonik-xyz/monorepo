//! Rebuild the recall index from stored signed artifacts (Wave 5, design §16).
//!
//! This is the routine that makes "SQLite is a *rebuildable cache*, not a
//! source of truth" actually true rather than aspirational. Given the signed
//! COSE artifact bytes (the same bytes uploaded to Arweave), it reconstructs
//! the index row — content, owner, tags, content_hash, and the embedding — so
//! the operator's (or anyone's) vector store can be regenerated from the
//! durable artifacts alone. Combined with the per-owner Merkle commitment
//! ([`crate::merkle`]), a rebuilt index is *checkable*: recompute the root from
//! the rebuilt set and compare to the anchored one.
//!
//! Reconstruction trusts only cryptographically valid artifacts: every input
//! is COSE-verified before any field is extracted, so a tampered or unsigned
//! blob can never inject a row.
//!
//! ## Precision tiers (§16)
//! Every artifact carries the **TurboQuant-compressed** embedding
//! (`metadata.embedding_compressed`); dequantizing it yields an *approximate*
//! f32 vector — fine for coarse recall, lossy for fine-grained search. The
//! opt-in **f32 tier** additionally stores the full vector in
//! `metadata.embedding_f32` (base64 little-endian f32), so `rebuild_row`
//! recovers it **exactly** ([`Precision::F32`]) and falls back to the
//! compressed copy ([`Precision::Compressed`]) when it's absent. f32-on-public-
//! Arweave is a *public-memory* feature only (embeddings can leak content via
//! inversion); the producer gates the toggle on visibility.
//!
//! Native-only for now (depends on the compressor); a wasm client rebuild is a
//! follow-up gated on verifying the wasm build.

use crate::codec::canonical::from_canonical_cbor;
use crate::codec::sign::verify_artifact;
use crate::compress::{CompressedEmbedding, EmbeddingCompressor};

/// Which embedding representation a row was rebuilt from. The opt-in f32 tier
/// (§16) stores the full vector in the signed artifact, so rebuild recovers it
/// **exactly**; otherwise only the TurboQuant-compressed copy is available and
/// the rebuilt vector is the dequantized approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// Recovered losslessly from `metadata.embedding_f32`.
    F32,
    /// Dequantized from `metadata.embedding_compressed` (lossy).
    Compressed,
}

/// Serialize an f32 embedding to little-endian bytes (4 bytes/dim) for the
/// `metadata.embedding_f32` artifact field. Producers base64 this; the matching
/// reader is [`f32_embedding_from_bytes`].
pub fn f32_embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Parse little-endian f32 bytes back to an embedding. Returns `None` if the
/// byte length isn't a multiple of 4.
pub fn f32_embedding_from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// A recall-index row reconstructed from a signed artifact. Carries exactly the
/// fields recoverable from the signed payload; storage-layer provenance the
/// payload doesn't sign (the Arweave/Solana tx ids) is supplied separately by
/// the caller that fetched the bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct RebuiltRow {
    pub attestation_id: String,
    pub content: String,
    pub content_hash: String,
    pub tags: Vec<String>,
    /// Author identity (`producer` minus the `did:sol:` prefix). In the
    /// self-sovereign model this equals `signer_pubkey`.
    pub owner_pubkey: String,
    /// COSE_Sign1 signer recovered from the envelope `kid`.
    pub signer_pubkey: String,
    pub created_at: String,
    /// The recovered embedding. Exact when `precision == F32`, else the
    /// dequantized approximation (see the precision caveat above).
    pub embedding: Vec<f32>,
    /// Which representation `embedding` was recovered from.
    pub precision: Precision,
}

/// Reconstruct a [`RebuiltRow`] from one signed artifact's COSE bytes.
///
/// `compressor` MUST be configured identically to the one that produced the
/// artifact (same `dim` / `bit_width` / seed) — pass the operator's compressor.
/// Returns `Err` if the artifact fails COSE verification or is missing a
/// required field.
pub fn rebuild_row(
    cose_bytes: &[u8],
    compressor: &EmbeddingCompressor,
) -> Result<RebuiltRow, String> {
    // 1. Verify the signature + content integrity BEFORE trusting any field.
    let v = verify_artifact(cose_bytes, None)?;
    if !(v.valid && v.cose_signature && v.content_integrity && v.algorithm_valid) {
        return Err("artifact failed COSE verification; refusing to rebuild from it".into());
    }

    // 2. Decode the canonical-CBOR payload back to the artifact JSON.
    let artifact = from_canonical_cbor(&v.payload)?;
    let get_str = |k: &str| artifact.get(k).and_then(|x| x.as_str()).map(str::to_string);

    let content = get_str("content").ok_or("artifact missing `content`")?;
    let attestation_id = get_str("artifact_id").ok_or("artifact missing `artifact_id`")?;
    let created_at = get_str("created_at").unwrap_or_default();
    let producer = get_str("producer").ok_or("artifact missing `producer`")?;
    let owner_pubkey = producer
        .strip_prefix("did:sol:")
        .unwrap_or(&producer)
        .to_string();
    let tags = artifact
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    // 3. Recover the embedding. Prefer the opt-in full-precision f32 copy
    //    (§16 precision tier) — lossless — and fall back to dequantizing the
    //    always-present compressed copy.
    let metadata = artifact.get("metadata");
    let f32_b64 = metadata
        .and_then(|m| m.get("embedding_f32"))
        .and_then(|x| x.as_str());

    let (embedding, precision) = if let Some(b64) = f32_b64 {
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .map_err(|e| format!("embedding_f32 is not valid base64: {e}"))?;
        let emb = f32_embedding_from_bytes(&raw)
            .ok_or("embedding_f32 byte length is not a multiple of 4")?;
        (emb, Precision::F32)
    } else {
        let b64 = metadata
            .and_then(|m| m.get("embedding_compressed"))
            .and_then(|x| x.as_str())
            .ok_or("artifact has neither embedding_f32 nor embedding_compressed")?;
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .map_err(|e| format!("embedding_compressed is not valid base64: {e}"))?;
        let compressed = CompressedEmbedding::from_bytes(&raw)
            .ok_or("could not parse compressed embedding bytes")?;
        (compressor.decompress(&compressed), Precision::Compressed)
    };

    Ok(RebuiltRow {
        attestation_id,
        content,
        content_hash: v.content_hash,
        tags,
        owner_pubkey,
        signer_pubkey: v.signer,
        created_at,
        embedding,
        precision,
    })
}

/// Reconstruct many rows, returning the successfully-rebuilt ones plus the
/// `(index, error)` of any that failed verification/decoding. A bad artifact in
/// the batch never aborts the rebuild — it is reported and skipped, so a single
/// corrupt blob can't deny reconstruction of the rest.
pub fn rebuild_rows(
    artifacts: &[Vec<u8>],
    compressor: &EmbeddingCompressor,
) -> (Vec<RebuiltRow>, Vec<(usize, String)>) {
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for (i, bytes) in artifacts.iter().enumerate() {
        match rebuild_row(bytes, compressor) {
            Ok(row) => ok.push(row),
            Err(e) => errs.push((i, e)),
        }
    }
    (ok, errs)
}

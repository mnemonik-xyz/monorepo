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
//! ## Precision caveat (the motivation for the f32 tier, §16)
//! The signed artifact carries only the **TurboQuant-compressed** embedding
//! (`metadata.embedding_compressed`). A rebuilt embedding is therefore the
//! *dequantized approximation* of the original f32 vector — fine for coarse
//! semantic recall, lossy for fine-grained search. The opt-in "f32 in the
//! signed artifact" precision tier is what closes this gap; it has a concrete
//! acceptance test only because this rebuild path exists to consume it.
//!
//! Native-only for now (depends on the compressor); a wasm client rebuild is a
//! follow-up gated on verifying the wasm build.

use crate::codec::canonical::from_canonical_cbor;
use crate::codec::sign::verify_artifact;
use crate::compress::{CompressedEmbedding, EmbeddingCompressor};

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
    /// Dequantized (approximate) embedding — see the precision caveat above.
    pub embedding: Vec<f32>,
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

    // 3. Recover the embedding from metadata.embedding_compressed.
    let b64 = artifact
        .get("metadata")
        .and_then(|m| m.get("embedding_compressed"))
        .and_then(|x| x.as_str())
        .ok_or("artifact missing metadata.embedding_compressed")?;
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        .map_err(|e| format!("embedding_compressed is not valid base64: {e}"))?;
    let compressed = CompressedEmbedding::from_bytes(&raw)
        .ok_or("could not parse compressed embedding bytes")?;
    let embedding = compressor.decompress(&compressed);

    Ok(RebuiltRow {
        attestation_id,
        content,
        content_hash: v.content_hash,
        tags,
        owner_pubkey,
        signer_pubkey: v.signer,
        created_at,
        embedding,
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

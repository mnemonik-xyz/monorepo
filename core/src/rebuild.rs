//! Rebuild the recall index from stored signed artifacts (Wave 5, design §16).
//!
//! This is the routine that makes "SQLite is a *rebuildable cache*, not a
//! source of truth" actually true rather than aspirational. Given the signed
//! COSE artifact bytes (the same bytes uploaded to Arweave), it reconstructs
//! the index row — content, owner, tags, content_hash, and the embedding — so
//! the operator's (or anyone's) vector store can be regenerated from the
//! durable artifacts alone. SQL Merkle inclusion proofs do not establish that
//! the recovered set is complete. Completeness requires independently trusted
//! expected heads and verified ancestry; unrecorded newer heads remain unknown.
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
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
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
    /// The signed `visibility` label, when the artifact carries one. Anchored
    /// artifacts record it (work/arweave-as-source-of-truth Wave 1) precisely so
    /// a restore does not have to guess. `None` for a legacy artifact written
    /// before the field existed; a caller must then default to the private,
    /// non-disclosing side.
    pub visibility: Option<String>,
    /// The signed `anchor` label naming the durable backend, when present. Lets
    /// a restore recover the write mode from the artifact instead of a column.
    pub anchor: Option<String>,
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
        visibility: get_str("visibility"),
        anchor: get_str("anchor"),
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

/// The TurboQuant seed every producer used before `metadata.turbo_seed` was
/// recorded in the artifact (work/arweave-as-source-of-truth Wave 1). It is a
/// protocol constant, not a tuning knob: change it and every pre-existing
/// compressed embedding becomes undequantizable.
pub const LEGACY_TURBO_SEED: u64 = 42;

/// Decrypt a sealed COSE_Sign1 artifact and return the inner memory JSON bytes.
///
/// This is the sealed-memory counterpart of [`rebuild_row`]: given the COSE
/// bytes of a `sealed.v1` artifact (as stored on Arweave) and the recipient's
/// X25519 secret key, it:
///
/// 1. Parses the COSE_Sign1 envelope to extract the canonical CBOR payload.
/// 2. Calls [`crate::sealed::open_memory`] to unwrap K and decrypt the content.
/// 3. Returns the raw inner memory JSON bytes.
///
/// The function does **not** verify the COSE signature — callers that need
/// provenance guarantees should call [`verify_artifact`] first.  The content key
/// commitment check inside `open_memory` still ensures the ciphertext was not
/// tampered with.
#[cfg(not(target_arch = "wasm32"))]
pub fn rebuild_sealed_row(cose_bytes: &[u8], x25519_secret: &[u8; 32]) -> Result<Vec<u8>, String> {
    use coset::CborSerializable;

    let cose_sign1 =
        coset::CoseSign1::from_slice(cose_bytes).map_err(|e| format!("invalid COSE_Sign1: {e}"))?;
    let payload = cose_sign1
        .payload
        .as_ref()
        .ok_or_else(|| "COSE_Sign1 has no payload".to_string())?;

    crate::sealed::open_memory(payload, x25519_secret)
        .map_err(|e| format!("sealed open failed: {e}"))
}

/// Rebuild a row from the artifact alone, deriving the compressor from what the
/// artifact declares about itself.
///
/// [`rebuild_row`] makes the caller supply a compressor configured exactly like
/// the producer's, which is a hidden out-of-band dependency: `embed_dim` and
/// `turbo_bits` were always in `metadata`, but the seed was not — it was a
/// constant inside the producing code, so a third party could not reproduce the
/// dequantization from the data alone. Anchored artifacts now record
/// `metadata.turbo_seed`, and this function reads all three.
///
/// A legacy artifact that predates `turbo_seed` falls back to
/// [`LEGACY_TURBO_SEED`], which is what every producer used at the time, so old
/// items still rebuild.
///
/// `embed_dim` is only a cross-check: the compressed bytes carry their own
/// dimension, and that is what the compressor is built from, because a mismatch
/// between the two would otherwise silently produce a wrong vector.
///
/// This is the entry point a client should use — including the WebAssembly
/// build, where there is no operator compressor to borrow.
pub fn rebuild_row_self_describing(cose_bytes: &[u8]) -> Result<RebuiltRow, String> {
    // Verify before reading any field, exactly as `rebuild_row` does. The work
    // is repeated inside `rebuild_row` below; that is deliberate, so there is
    // one verification path rather than an unchecked shortcut here.
    let v = verify_artifact(cose_bytes, None)?;
    if !(v.valid && v.cose_signature && v.content_integrity && v.algorithm_valid) {
        return Err("artifact failed COSE verification; refusing to rebuild from it".into());
    }
    let artifact = from_canonical_cbor(&v.payload)?;
    let metadata = artifact.get("metadata");

    let seed = metadata
        .and_then(|m| m.get("turbo_seed"))
        .and_then(|x| x.as_u64())
        .unwrap_or(LEGACY_TURBO_SEED);

    // The compressed bytes are authoritative for dim and bit width; they are
    // what the vector must be decoded against.
    let b64 = metadata
        .and_then(|m| m.get("embedding_compressed"))
        .and_then(|x| x.as_str())
        .ok_or("artifact has no metadata.embedding_compressed")?;
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        .map_err(|e| format!("embedding_compressed is not valid base64: {e}"))?;
    let compressed = CompressedEmbedding::from_bytes(&raw)
        .ok_or("could not parse compressed embedding bytes")?;

    if let Some(declared) = metadata
        .and_then(|m| m.get("embed_dim"))
        .and_then(|x| x.as_u64())
    {
        if declared as usize != compressed.dim {
            return Err(format!(
                "metadata.embed_dim ({declared}) disagrees with the compressed bytes ({})",
                compressed.dim
            ));
        }
    }

    let compressor = EmbeddingCompressor::new(compressed.dim, compressed.bit_width, seed);
    rebuild_row(cose_bytes, &compressor)
}

/// Structured recovery failures; unsupported formats never masquerade as empty success.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryError {
    pub code: &'static str,
    pub message: String,
}
impl RecoveryError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Complete client-local recovery, retaining the exact original signed envelope.
/// Plaintext is never an operator receipt and must only enter the owner's local store.
#[derive(Debug, Clone)]
pub struct RecoveredMemory {
    pub envelope: Vec<u8>,
    pub envelope_digest: String,
    pub author: String,
    pub artifact_id: String,
    pub sealed: bool,
    pub plaintext: Vec<u8>,
    pub memory: serde_json::Value,
}

/// Verify identity before kind dispatch or decryption. The expected author must
/// come from caller trust, never from discovery tags alone. Grants and A2A use
/// their dedicated recovery paths and are explicitly unsupported here.
pub fn recover_memory(
    bytes: &[u8],
    expected_author: &str,
    x25519_secret: Option<&[u8; 32]>,
) -> Result<RecoveredMemory, RecoveryError> {
    if bytes.len() > 1048576 {
        return Err(RecoveryError::new("too_large", "artifact exceeds 1 MiB"));
    }
    let verified =
        verify_artifact(bytes, None).map_err(|e| RecoveryError::new("invalid_envelope", e))?;
    if !verified.valid || verified.signer != expected_author {
        return Err(RecoveryError::new(
            "untrusted_author",
            "signature or pinned author mismatch",
        ));
    }
    let outer = from_canonical_cbor(&verified.payload)
        .map_err(|e| RecoveryError::new("malformed_payload", e))?;
    let expected_producer = format!("did:sol:{expected_author}");
    if outer["producer"].as_str() != Some(expected_producer.as_str()) {
        return Err(RecoveryError::new(
            "author_binding",
            "producer differs from signer",
        ));
    }
    let sealed = match (outer["type"].as_str(), outer["schema_version"].as_u64()) {
        (Some("memory"), Some(1)) => false,
        (Some("sealed"), Some(1)) => true,
        _ => {
            return Err(RecoveryError::new(
                "unsupported_kind",
                "supported kinds: memory.v1, sealed.v1",
            ))
        }
    };
    let plaintext = if sealed {
        let key = x25519_secret.ok_or_else(|| {
            RecoveryError::new(
                "key_required",
                "sealed recovery requires a backed-up decryption key",
            )
        })?;
        crate::sealed::open_memory(&verified.payload, key).map_err(|_| {
            RecoveryError::new(
                "open_failed",
                "complete plaintext could not be authenticated",
            )
        })?
    } else {
        verified.payload
    };
    // The existing SDK seals JSON; native producers also seal canonical CBOR.
    // Neither format changes the signed outer identity or exact plaintext bytes.
    let memory = match from_canonical_cbor(&plaintext) {
        Ok(memory) => memory,
        Err(_) if sealed => serde_json::from_slice(&plaintext)
            .map_err(|e| RecoveryError::new("malformed_memory", e.to_string()))?,
        Err(e) => return Err(RecoveryError::new("malformed_memory", e)),
    };
    if memory["type"] != "memory"
        || memory.get("schema_version").is_some_and(|v| v != 1)
        || (!sealed && memory["schema_version"] != 1)
    {
        return Err(RecoveryError::new(
            "unsupported_inner_kind",
            "inner payload must be memory.v1 or legacy sealed memory JSON",
        ));
    }
    let artifact_id = outer["artifact_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RecoveryError::new("malformed_memory", "missing signed artifact ID"))?
        .to_string();
    if memory
        .get("producer")
        .is_some_and(|v| v.as_str() != Some(expected_producer.as_str()))
        || memory
            .get("artifact_id")
            .is_some_and(|v| v.as_str() != Some(artifact_id.as_str()))
        || !memory["content"].is_string()
    {
        return Err(RecoveryError::new(
            "memory_binding",
            "inner identity, author or content mismatch",
        ));
    }
    Ok(RecoveredMemory {
        envelope: bytes.to_vec(),
        envelope_digest: crate::codec::hash::hash_bytes(bytes),
        author: expected_author.to_owned(),
        artifact_id,
        sealed,
        plaintext,
        memory,
    })
}

/// Graph evidence relative only to caller-pinned heads. This never proves that
/// an index disclosed every artifact, fork or newer head.
#[derive(Debug, Default)]
pub struct MemoryRecoveryReport {
    pub memories: Vec<RecoveredMemory>,
    pub rejected: Vec<(usize, RecoveryError)>,
    pub missing_parents: Vec<String>,
    pub forks: Vec<String>,
    pub invalid_heads: Vec<String>,
    pub invalid_artifacts: Vec<String>,
    pub complete_to_heads: bool,
}

/// Independent checkpoint binding. MEMORY_V1 IDs alone are not content addressed.
#[derive(Debug, Clone)]
pub struct MemoryRecoveryHead {
    pub artifact_id: String,
    pub envelope_digest: String,
    pub author: String,
}

/// Verify every candidate before interpreting signed parent references. Duplicate
/// IDs with different envelopes are ambiguous and cannot satisfy pinned heads.
pub fn recover_memory_set(
    candidates: &[(&[u8], &str)],
    x25519_secret: Option<&[u8; 32]>,
    heads: &[MemoryRecoveryHead],
    trusted_ancestry: &[MemoryRecoveryHead],
) -> MemoryRecoveryReport {
    use std::collections::{BTreeSet, HashMap, HashSet};
    let mut report = MemoryRecoveryReport::default();
    let mut rows = HashMap::<String, RecoveredMemory>::new();
    let mut ambiguous = HashSet::new();
    for (index, (bytes, author)) in candidates.iter().enumerate() {
        match recover_memory(bytes, author, x25519_secret) {
            Err(e) => report.rejected.push((index, e)),
            Ok(row) => {
                if rows
                    .get(&row.artifact_id)
                    .is_some_and(|old| old.envelope != row.envelope)
                {
                    ambiguous.insert(row.artifact_id.clone());
                    report.rejected.push((
                        index,
                        RecoveryError::new(
                            "ambiguous_id",
                            "conflicting signed artifacts share an ID",
                        ),
                    ));
                } else {
                    rows.insert(row.artifact_id.clone(), row);
                }
            }
        }
    }
    // MEMORY_V1 links names, not content hashes. Every ancestry node must
    // therefore be pinned independently, not only the requested head.
    for (id, row) in &rows {
        if !heads.iter().chain(trusted_ancestry).any(|pin| {
            pin.artifact_id == *id
                && pin.author == row.author
                && pin.envelope_digest == row.envelope_digest
        }) {
            ambiguous.insert(id.clone());
        }
    }
    let mut edges = HashMap::<String, Vec<String>>::new();
    let mut children = HashMap::<String, HashSet<String>>::new();
    for (id, row) in &rows {
        let parent_value = row.memory.get("parents");
        let parents = match parent_value {
            None => Vec::new(),
            Some(serde_json::Value::Array(values))
                if values.len() <= crate::codec::schema::MAX_PARENTS =>
            {
                let parsed: Option<Vec<String>> = values
                    .iter()
                    .map(|p| {
                        p["artifact_id"]
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .map(str::to_owned)
                    })
                    .collect();
                match parsed {
                    Some(p) => p,
                    None => {
                        ambiguous.insert(id.clone());
                        Vec::new()
                    }
                }
            }
            _ => {
                ambiguous.insert(id.clone());
                Vec::new()
            }
        };
        for parent in &parents {
            children
                .entry(parent.clone())
                .or_default()
                .insert(id.clone());
        }
        edges.insert(id.clone(), parents);
    }
    let mut missing = BTreeSet::new();
    for parents in edges.values() {
        for parent in parents {
            if !edges.contains_key(parent) {
                missing.insert(parent.clone());
            }
        }
    }
    // Resolve one ancestry level per pass; bounded work, no recursive stack or
    // exponential revisits when many descendants share the same parents.
    let mut good = HashSet::new();
    for _ in 0..crate::codec::schema::MAX_DEPTH {
        let next: Vec<String> = edges
            .iter()
            .filter(|(id, parents)| {
                !ambiguous.contains(*id)
                    && !good.contains(*id)
                    && parents.iter().all(|p| good.contains(p))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if next.is_empty() {
            break;
        }
        good.extend(next);
    }
    for head in heads {
        let binding_matches = rows.get(&head.artifact_id).is_some_and(|row| {
            row.envelope_digest == head.envelope_digest && row.author == head.author
        });
        if !good.contains(&head.artifact_id) || !binding_matches {
            report.invalid_heads.push(head.artifact_id.clone());
        }
    }
    report.invalid_artifacts = rows
        .keys()
        .filter(|id| !good.contains(*id))
        .cloned()
        .collect();
    report.invalid_artifacts.sort();
    report.complete_to_heads = !heads.is_empty() && report.invalid_heads.is_empty();
    report.missing_parents = missing.into_iter().collect();
    report.forks = children
        .into_iter()
        .filter_map(|(parent, children)| (children.len() > 1).then_some(parent))
        .collect();
    report.forks.sort();
    report.memories = rows
        .into_iter()
        .filter_map(|(id, row)| good.contains(&id).then_some(row))
        .collect();
    report
        .memories
        .sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));
    report
}

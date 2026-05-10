//! Emit deterministic golden fixtures for `packages/extension`'s WASM
//! parity tests (T05). Output: a directory containing:
//!
//! - `manifest.json` — array of `{seed, plaintext, signer_pubkey_b58,
//!   signer_secret_hex, embedding_f32, dim, bit_width, content_hash_hex,
//!   canonical_cbor_b64, cose_sign1_b64, compressed_emb_b64}` for each
//!   seed.
//! - `cose_sign1_seed_<N>.bin` — raw COSE_Sign1 bytes per seed.
//! - `compressed_emb_seed_<N>.bin` — TurboQuant 4-bit compressed bytes.
//! - `canonical_cbor_seed_<N>.bin` — canonical CBOR bytes.
//!
//! Why a Cargo example and not a `#[test]`:
//!   - The existing `core/tests/golden_fixtures.rs::emit_fixtures`
//!     prints to stdout. Fine for the SDK lockstep gate (a single JSON
//!     dump). The extension wants binary `.bin` files committed at a
//!     known path so vitest can read them directly without base64 round-
//!     trip — a `#[test]` writing to a sibling-package path is awkward
//!     CI-wise. An example with an explicit output-dir argument is the
//!     idiomatic Rust way to do "build artifact for downstream package".
//!
//! Determinism rules:
//!   - Hardcoded 32-byte Ed25519 seed → deterministic Solana keypair →
//!     same COSE signature on every run (RFC 8032).
//!   - Embeddings are deterministic functions of the seed index.
//!   - Compressor seed = 42 (matches `mcp/src/main.rs:309` and every
//!     test compressor in the codebase).
//!   - Re-running the example twice produces byte-identical output.
//!
//! Invocation:
//!
//! ```sh
//! cargo run --features golden-fixtures --example emit_golden -- \
//!   packages/extension/tests/fixtures/golden/
//! ```
//!
//! Tech-spec reference: `work/chrome-extension/tasks/05.md`.

#![cfg(all(feature = "golden-fixtures", not(target_arch = "wasm32")))]

use base64::Engine;
use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::codec::hash::hash_bytes;
use mnemonic_core::codec::schema::MEMORY_V1;
use mnemonic_core::codec::sign::sign_cose;
use mnemonic_core::compress::EmbeddingCompressor;
use serde::Serialize;
use serde_json::Value as JsonValue;
use solana_sdk::signature::{Keypair, Signer};
use std::fs;
use std::path::{Path, PathBuf};

/// Hardcoded 32-byte Ed25519 seed. Test-only; do NOT use to sign real
/// attestations. Hex value:
/// `00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`.
const FIXED_SEED_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// Number of fixtures to emit. Five matches the task spec.
const NUM_SEEDS: usize = 5;

/// Embedding dimension. 384 matches the default `Xenova/all-MiniLM-L6-v2`
/// (Decision 3) so the fixtures exercise the production-shape vector.
const DIM: usize = 384;

/// TurboQuant bit-width. 4 matches `mcp/src/config.rs::turbo_bits` default.
const BIT_WIDTH: usize = 4;

/// Compressor seed — matches every other instantiation in the codebase
/// (see `grep -rn "EmbeddingCompressor::new"`).
const COMPRESS_SEED: u64 = 42;

#[derive(Serialize)]
struct ManifestEntry {
    seed: usize,
    plaintext: String,
    signer_pubkey_b58: String,
    signer_secret_hex: String,
    embedding_f32: Vec<f32>,
    dim: usize,
    bit_width: u8,
    content_hash_hex: String,
    canonical_cbor_b64: String,
    cose_sign1_b64: String,
    compressed_emb_b64: String,
}

fn fixture_keypair() -> Keypair {
    let seed_vec = hex::decode(FIXED_SEED_HEX).expect("seed hex parses");
    assert_eq!(seed_vec.len(), 32);
    let mut seed_arr = [0u8; 32];
    seed_arr.copy_from_slice(&seed_vec);
    Keypair::new_from_array(seed_arr)
}

/// Deterministic 384-dim embedding for seed `n`. Pure function of `n`
/// using a tiny LCG so re-runs produce identical floats.
fn embedding_for_seed(n: usize) -> Vec<f32> {
    let mut state: u64 = (n as u64).wrapping_mul(0x9E3779B97F4A7C15);
    (0..DIM)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // Map to roughly [-1, 1] with reasonable distribution.
            ((state >> 32) as f32 / (u32::MAX as f32 / 2.0)) - 1.0
        })
        .collect()
}

/// Build the artifact JSON for seed `n`. Layout matches `MEMORY_V1` —
/// fields in the schema's `cbor_field_order` will be picked up; others
/// are dropped.
fn build_artifact(n: usize, kp: &Keypair, compressed_b64: &str) -> JsonValue {
    serde_json::json!({
        "artifact_id": format!("art:t05-seed-{n}"),
        "type": "memory",
        "schema_version": 1,
        "content": format!("T05 golden fixture seed {n}"),
        "producer": format!("did:sol:{}", kp.pubkey()),
        "created_at": format!("2026-05-10T00:00:{:02}Z", n),
        "metadata": {
            "embedding_compressed": compressed_b64,
            "embed_dim": DIM,
            "turbo_bits": BIT_WIDTH,
        }
    })
}

fn emit_one(n: usize, kp: &Keypair, out_dir: &Path) -> ManifestEntry {
    let embedding = embedding_for_seed(n);
    let compressor = EmbeddingCompressor::new(DIM, BIT_WIDTH, COMPRESS_SEED);
    let compressed = compressor.compress(&embedding).to_bytes();
    let compressed_b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);

    let artifact = build_artifact(n, kp, &compressed_b64);
    let canonical = to_canonical_cbor(&artifact, &MEMORY_V1).expect("canonical CBOR succeeds");
    let content_hash_hex = hash_bytes(&canonical);
    let cose = sign_cose(&canonical, kp).expect("sign_cose succeeds");

    fs::write(
        out_dir.join(format!("canonical_cbor_seed_{n}.bin")),
        &canonical,
    )
    .expect("write canonical_cbor");
    fs::write(out_dir.join(format!("cose_sign1_seed_{n}.bin")), &cose).expect("write cose_sign1");
    fs::write(
        out_dir.join(format!("compressed_emb_seed_{n}.bin")),
        &compressed,
    )
    .expect("write compressed_emb");

    ManifestEntry {
        seed: n,
        plaintext: format!("T05 golden fixture seed {n}"),
        signer_pubkey_b58: kp.pubkey().to_string(),
        signer_secret_hex: hex::encode(kp.to_bytes()),
        embedding_f32: embedding,
        dim: DIM,
        bit_width: BIT_WIDTH as u8,
        content_hash_hex,
        canonical_cbor_b64: base64::engine::general_purpose::STANDARD.encode(&canonical),
        cose_sign1_b64: base64::engine::general_purpose::STANDARD.encode(&cose),
        compressed_emb_b64: compressed_b64,
    }
}

fn main() {
    let out_dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "packages/extension/tests/fixtures/golden/".to_string())
        .into();
    fs::create_dir_all(&out_dir).expect("create out dir");

    let kp = fixture_keypair();
    let entries: Vec<ManifestEntry> = (0..NUM_SEEDS).map(|n| emit_one(n, &kp, &out_dir)).collect();

    let manifest = serde_json::to_string_pretty(&entries).expect("serialize manifest");
    fs::write(out_dir.join("manifest.json"), &manifest).expect("write manifest");

    eprintln!(
        "✓ wrote {} fixtures + manifest.json to {}",
        entries.len(),
        out_dir.display()
    );
}

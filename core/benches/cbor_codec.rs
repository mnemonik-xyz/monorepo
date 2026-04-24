//! Performance benchmark: CBOR canonicalization + blake3 hash + COSE signing.
//!
//! All canonicalization/signing primitives come from `mnemonic_core::codec`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::codec::hash::hash_bytes;
use mnemonic_core::codec::schema::MEMORY_V1;
use mnemonic_core::codec::sign::sign_artifact;
use solana_sdk::signature::Keypair;

fn sample_artifact(content_size: usize) -> serde_json::Value {
    let content: String = "x".repeat(content_size);
    serde_json::json!({
        "artifact_id": "art:bench",
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": "did:sol:7xKXtg2CabcdefghijklmnopqrstuvwxyzABCDEFGH",
        "created_at": "2026-04-14T12:00:00Z",
        "tags": ["bench", "perf"],
        "metadata": {"source": "benchmark"},
    })
}

fn bench_cbor_canonicalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("cbor_canonicalize");

    for size in [100, 500, 2000, 10000] {
        let artifact = sample_artifact(size);
        group.bench_function(format!("{size}B_content"), |b| {
            b.iter(|| to_canonical_cbor(black_box(&artifact), &MEMORY_V1).unwrap())
        });
    }
    group.finish();
}

fn bench_blake3_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3_hash");

    for size in [100, 500, 2000, 10000] {
        let artifact = sample_artifact(size);
        let cbor = to_canonical_cbor(&artifact, &MEMORY_V1).unwrap();
        group.bench_function(format!("{size}B_content"), |b| {
            b.iter(|| hash_bytes(black_box(&cbor)))
        });
    }
    group.finish();
}

fn bench_cose_sign(c: &mut Criterion) {
    let kp = Keypair::new();
    let mut group = c.benchmark_group("cose_sign");

    for size in [100, 500, 2000] {
        let artifact = sample_artifact(size);
        group.bench_function(format!("{size}B_content"), |b| {
            b.iter(|| sign_artifact(black_box(&artifact), &MEMORY_V1, &kp).unwrap())
        });
    }
    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let kp = Keypair::new();
    let mut group = c.benchmark_group("full_pipeline");

    for size in [100, 500, 2000] {
        let artifact = sample_artifact(size);
        group.bench_function(format!("{size}B_content"), |b| {
            b.iter(|| {
                // sign_artifact internally computes canonical CBOR + blake3 hash + COSE signature,
                // which matches the prior full_pipeline shape exactly.
                sign_artifact(black_box(&artifact), &MEMORY_V1, &kp).unwrap()
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_cbor_canonicalization,
    bench_blake3_hash,
    bench_cose_sign,
    bench_full_pipeline,
);
criterion_main!(benches);

//! Benchmark: EmbeddingCompressor decompression throughput.

// Native-only — criterion is a non-wasm dev-dep. The whole bench body is gated
// behind `not(target_arch = "wasm32")`. On wasm32 we provide a no-op `main` so
// `cargo clippy --all-targets --target wasm32-unknown-unknown` does not error
// with E0601; the bench is never actually run on wasm32.
#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
#[cfg(not(target_arch = "wasm32"))]
use mnemonic_core::compress::EmbeddingCompressor;

#[cfg(not(target_arch = "wasm32"))]
fn sample_vector(dim: usize) -> Vec<f32> {
    (0..dim).map(|i| ((i as f32) * 0.01).sin()).collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn bench_decompress(c: &mut Criterion) {
    let mut group = c.benchmark_group("embedding_decompress");
    let cases: &[(usize, usize)] = &[(128, 4), (384, 4), (768, 4), (1536, 4)];

    for &(dim, bits) in cases {
        let compressor = EmbeddingCompressor::new(dim, bits, 42);
        let input = sample_vector(dim);
        let compressed = compressor.compress(&input);

        group.throughput(Throughput::Elements(dim as u64));
        group.bench_with_input(
            BenchmarkId::new(format!("{bits}bit"), dim),
            &compressed,
            |b, compressed| {
                b.iter(|| {
                    black_box(compressor.decompress(black_box(compressed)));
                });
            },
        );
    }

    group.finish();
}

#[cfg(not(target_arch = "wasm32"))]
criterion_group!(benches, bench_decompress);
#[cfg(not(target_arch = "wasm32"))]
criterion_main!(benches);

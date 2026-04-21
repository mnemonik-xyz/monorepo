use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

#[path = "../src/compress.rs"]
mod compress;

use compress::EmbeddingCompressor;

fn sample_vector(dim: usize) -> Vec<f32> {
    (0..dim).map(|i| ((i as f32) * 0.01).sin()).collect()
}

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

criterion_group!(benches, bench_decompress);
criterion_main!(benches);

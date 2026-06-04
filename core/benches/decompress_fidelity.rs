//! Fidelity benchmark: how much accuracy does TurboQuant cost at each bit width?
//!
//! Unlike `decompress.rs` (which measures throughput), this bench measures
//! reconstruction *quality* across (dim, bit_width) combinations and prints
//! a table to stdout. Three metrics per cell:
//!
//!   - mean MSE between original and decompressed vector
//!   - mean cosine similarity (and worst-case min over the corpus)
//!   - Top-K recall: of the K nearest neighbors found over the f32 corpus,
//!     how many survive when we re-rank against the decompressed corpus?
//!
//! Top-K recall is the metric that actually matters for the protocol: it
//! tells us whether the compressed form is viable as a shadow index for
//! candidate generation (whitepaper §13.2). MSE and cosine give us the
//! per-vector distortion that drives that.
//!
//! Run with:  cargo bench -p mnemonic-core --bench decompress_fidelity
//!
//! Note: corpus is deterministic synthetic vectors (L2-normalized LCG-uniform).
//! Real text embeddings have anisotropic structure that may behave better or
//! worse than these baselines; treat numbers as an upper-bound on the
//! distortion you'd see with structured corpora.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use mnemonic_core::compress::EmbeddingCompressor;
    use mnemonic_core::embed::cosine_similarity;
    use std::collections::HashSet;

    fn corpus(dim: usize, n: usize, seed: u64) -> Vec<Vec<f32>> {
        // Tiny LCG (Numerical Recipes constants) so the bench has no rand dep
        // and runs are byte-identical across machines.
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut v = Vec::with_capacity(dim);
            for _ in 0..dim {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let bits = (state >> 32) as u32;
                v.push((bits as f32 / u32::MAX as f32) * 2.0 - 1.0);
            }
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in v.iter_mut() {
                    *x /= norm;
                }
            }
            out.push(v);
        }
        out
    }

    fn top_k(scores: &[f32], k: usize) -> Vec<usize> {
        let mut idxs: Vec<usize> = (0..scores.len()).collect();
        idxs.sort_by(|&a, &b| {
            scores[b]
                .partial_cmp(&scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idxs.truncate(k);
        idxs
    }

    let dims = [128usize, 384, 768, 1536];
    let bit_widths = [2usize, 3, 4];
    let corpus_size = 256;
    let n_queries = 32;
    let k = 10;

    println!();
    println!("=== TurboQuant Decompression Fidelity ===");
    println!(
        "Corpus: {corpus_size} synthetic L2-normalized vectors per cell, \
         {n_queries} held-out queries, Top-K = {k}"
    );
    println!();
    println!(
        "{:>6} {:>5} {:>11} {:>10} {:>9} {:>11} {:>10}",
        "dim", "bits", "mean MSE", "mean cos", "min cos", "Top-K rec", "ratio"
    );
    println!("{}", "-".repeat(70));

    for &dim in &dims {
        let docs = corpus(dim, corpus_size, 0xC0FF_EE00);
        let queries = corpus(dim, n_queries, 0xDEAD_BEEF);

        // f32 baseline ranking — what we have to preserve.
        let baseline_topk: Vec<Vec<usize>> = queries
            .iter()
            .map(|q| {
                let scores: Vec<f32> = docs.iter().map(|d| cosine_similarity(q, d)).collect();
                top_k(&scores, k)
            })
            .collect();

        for &bits in &bit_widths {
            let compressor = EmbeddingCompressor::new(dim, bits, 42);
            let ratio = compressor.compression_ratio();

            let mut mse_sum = 0.0f64;
            let mut cos_sum = 0.0f64;
            let mut min_cos = f32::INFINITY;
            let mut decompressed: Vec<Vec<f32>> = Vec::with_capacity(corpus_size);
            for d in &docs {
                let c = compressor.compress(d);
                let r = compressor.decompress(&c);
                let mse: f32 = d
                    .iter()
                    .zip(r.iter())
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    / d.len() as f32;
                let cos = cosine_similarity(d, &r);
                mse_sum += mse as f64;
                cos_sum += cos as f64;
                if cos < min_cos {
                    min_cos = cos;
                }
                decompressed.push(r);
            }
            let mean_mse = mse_sum / corpus_size as f64;
            let mean_cos = cos_sum / corpus_size as f64;

            let mut overlap = 0usize;
            for (qi, q) in queries.iter().enumerate() {
                let scores: Vec<f32> = decompressed
                    .iter()
                    .map(|d| cosine_similarity(q, d))
                    .collect();
                let approx = top_k(&scores, k);
                let truth: HashSet<usize> = baseline_topk[qi].iter().copied().collect();
                overlap += approx.iter().filter(|i| truth.contains(i)).count();
            }
            let recall = overlap as f64 / (n_queries * k) as f64;

            println!(
                "{:>6} {:>5} {:>11.6} {:>10.4} {:>9.4} {:>10.2}% {:>9.2}x",
                dim,
                bits,
                mean_mse,
                mean_cos,
                min_cos,
                recall * 100.0,
                ratio
            );
        }
    }
    println!();
    println!("Interpretation:");
    println!("  - mean cos > 0.95 and Top-K recall > 95% means the compressed form is");
    println!("    safe as a shadow-index / portability format at that bit width.");
    println!("  - synthetic uniform vectors are a hard case (no clustering); real");
    println!("    text embeddings typically retain more structure post-quantization.");
    println!();
}

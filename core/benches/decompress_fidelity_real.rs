//! Fidelity bench using REAL text embeddings (fastembed / all-MiniLM-L6-v2, 384-dim).
//!
//! Same metrics as decompress_fidelity.rs (MSE, cosine, Top-K recall) but the
//! corpus comes from real sentences embedded by a real model -- the actual
//! distribution we care about. Synthetic uniform vectors are an adversarial
//! worst case; this gives the realistic upper bound.
//!
//! Requires the `local-embed` feature, which pulls in fastembed and downloads
//! ~22MB of ONNX weights to ~/.cache/fastembed on first invocation.
//!
//! Run with:
//!   cargo bench -p mnemonic-core --bench decompress_fidelity_real \
//!     --features local-embed

#[cfg(any(target_arch = "wasm32", not(feature = "local-embed")))]
fn main() {
    eprintln!(
        "decompress_fidelity_real requires --features local-embed; \
         run: cargo bench -p mnemonic-core --bench decompress_fidelity_real --features local-embed"
    );
}

#[cfg(all(not(target_arch = "wasm32"), feature = "local-embed"))]
fn main() {
    use mnemonic_core::compress::EmbeddingCompressor;
    use mnemonic_core::embed::{cosine_similarity, Embedder, FastEmbedder};
    use std::collections::HashSet;

    fn load_lines(raw: &str) -> Vec<String> {
        raw.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect()
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

    let corpus_text = load_lines(include_str!("fixtures/fidelity_corpus.txt"));
    let query_text = load_lines(include_str!("fixtures/fidelity_queries.txt"));
    let k = 10;

    println!();
    println!("=== TurboQuant Fidelity on Real Embeddings ===");
    println!("Loading fastembed (all-MiniLM-L6-v2, 384-dim) -- first run downloads ~22MB.");
    let embedder = match FastEmbedder::try_new() {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("fastembed init failed: {msg}");
            eprintln!("(no network access? model cache missing? rerun once with internet)");
            std::process::exit(2);
        }
    };
    let dim = embedder.dim();
    println!(
        "Corpus: {} sentences, {} queries, dim = {}, Top-K = {}.",
        corpus_text.len(),
        query_text.len(),
        dim,
        k
    );
    println!();

    let docs: Vec<Vec<f32>> = corpus_text.iter().map(|s| embedder.embed(s)).collect();
    let queries: Vec<Vec<f32>> = query_text.iter().map(|s| embedder.embed(s)).collect();

    let baseline_topk: Vec<Vec<usize>> = queries
        .iter()
        .map(|q| {
            let scores: Vec<f32> = docs.iter().map(|d| cosine_similarity(q, d)).collect();
            top_k(&scores, k)
        })
        .collect();

    println!(
        "{:>5} {:>11} {:>10} {:>9} {:>11} {:>10}",
        "bits", "mean MSE", "mean cos", "min cos", "Top-K rec", "ratio"
    );
    println!("{}", "-".repeat(63));

    for &bits in &[2usize, 3, 4] {
        let compressor = EmbeddingCompressor::new(dim, bits, 42);
        let ratio = compressor.compression_ratio();

        let mut mse_sum = 0.0f64;
        let mut cos_sum = 0.0f64;
        let mut min_cos = f32::INFINITY;
        let mut decompressed: Vec<Vec<f32>> = Vec::with_capacity(docs.len());
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
        let mean_mse = mse_sum / docs.len() as f64;
        let mean_cos = cos_sum / docs.len() as f64;

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
        let recall = overlap as f64 / (queries.len() * k) as f64;

        println!(
            "{:>5} {:>11.6} {:>10.4} {:>9.4} {:>10.2}% {:>9.2}x",
            bits,
            mean_mse,
            mean_cos,
            min_cos,
            recall * 100.0,
            ratio
        );
    }
    println!();
    println!("Compare against decompress_fidelity (synthetic uniform vectors) to see");
    println!("how much of the recall loss was an artifact of the i.i.d. baseline.");
    println!();
}

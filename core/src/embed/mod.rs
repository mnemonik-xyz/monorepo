//! Embedding service -- pluggable: fastembed (local ONNX) or OpenAI API.

/// Embedding provider trait.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Vec<f32>;
    fn dim(&self) -> usize;
    fn provider_name(&self) -> &str;
    /// Canonical model identifier stored in on-chain anchor.
    /// Third parties use this to re-embed and verify.
    fn model_id(&self) -> &str;
    /// True if the model weights are publicly available (open source).
    /// Only open-weight models produce externally verifiable attestations.
    fn is_open_weights(&self) -> bool {
        false
    }
}

// -- FastEmbed (local ONNX) --

/// Local ONNX embedder via fastembed (all-MiniLM-L6-v2, 384-dim).
/// Downloads model on first run (~22MB), cached at ~/.cache/fastembed.
/// Enable with: cargo build --features local-embed
#[cfg(feature = "local-embed")]
pub struct FastEmbedder {
    model: fastembed::TextEmbedding,
    dim: usize,
}

#[cfg(feature = "local-embed")]
impl FastEmbedder {
    pub fn try_new() -> Result<Self, String> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let model = TextEmbedding::try_new(InitOptions {
            model_name: EmbeddingModel::AllMiniLML6V2,
            ..Default::default()
        })
        .map_err(|e| format!("fastembed init failed: {e}"))?;

        // Probe dimension
        let sample = model
            .embed(vec!["test"], None)
            .map_err(|e| format!("fastembed probe failed: {e}"))?;
        let dim = sample.first().map(|v| v.len()).unwrap_or(384);

        Ok(Self { model, dim })
    }
}

#[cfg(feature = "local-embed")]
impl Embedder for FastEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        match self.model.embed(vec![text.to_string()], None) {
            Ok(embeddings) => embeddings
                .into_iter()
                .next()
                .unwrap_or_else(|| vec![0.0; self.dim]),
            Err(e) => {
                tracing::error!("fastembed inference failed: {e}");
                vec![0.0; self.dim]
            }
        }
    }

    fn dim(&self) -> usize {
        self.dim
    }
    fn provider_name(&self) -> &str {
        "fastembed"
    }
    fn model_id(&self) -> &str {
        "all-MiniLM-L6-v2"
    }
    fn is_open_weights(&self) -> bool {
        true
    } // Apache 2.0
}

// -- OpenAI embedder --

/// OpenAI embeddings API embedder.
pub struct OpenAIEmbedder {
    api_key: String,
    model: String,
    dim: usize,
}

impl OpenAIEmbedder {
    pub fn new(api_key: &str, model: &str) -> Self {
        let dim = if model.contains("small") { 1536 } else { 3072 };
        Self {
            api_key: api_key.to_string(),
            model: model.to_string(),
            dim,
        }
    }
}

impl Embedder for OpenAIEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let client = reqwest::blocking::Client::new();
        let body = serde_json::json!({ "input": text, "model": self.model });
        let resp = client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send();

        match resp {
            Ok(r) => {
                if let Ok(json) = r.json::<serde_json::Value>() {
                    if let Some(embedding) = json["data"][0]["embedding"].as_array() {
                        return embedding
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();
                    }
                }
                tracing::error!("OpenAI embedding failed, returning zeros");
                vec![0.0; self.dim]
            }
            Err(e) => {
                tracing::error!("OpenAI request failed: {e}");
                vec![0.0; self.dim]
            }
        }
    }

    fn dim(&self) -> usize {
        self.dim
    }
    fn provider_name(&self) -> &str {
        "openai"
    }
    fn model_id(&self) -> &str {
        &self.model
    }
}

// -- MockEmbedder (test only) --

/// Deterministic mock embedder for testing. Returns normalized sequential float vectors.
#[cfg(test)]
pub struct MockEmbedder {
    dim: usize,
}

#[cfg(test)]
impl MockEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

#[cfg(test)]
impl Embedder for MockEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        if self.dim == 0 {
            return vec![];
        }
        let mut vec: Vec<f32> = (0..self.dim).map(|i| i as f32).collect();
        l2_normalize(&mut vec);
        vec
    }

    fn dim(&self) -> usize {
        self.dim
    }
    fn provider_name(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock-deterministic"
    }
}

// -- Builder --

/// Build embedder from config.
///
/// Returns Ok(embedder) or Err with a message if no real embedder is available.
///
/// Priority: fastembed (open, verifiable) > openai (proprietary but semantic)
pub fn build_embedder(
    provider: &str,
    api_key: &str,
    model: &str,
) -> Result<Box<dyn Embedder>, String> {
    match provider {
        "fastembed" => {
            #[cfg(feature = "local-embed")]
            {
                match FastEmbedder::try_new() {
                    Ok(e) => {
                        tracing::info!(
                            "Embedder: fastembed ({}, {}-dim, open weights, verifiable)",
                            e.model_id(),
                            e.dim()
                        );
                        return Ok(Box::new(e));
                    }
                    Err(msg) => {
                        tracing::warn!("fastembed unavailable: {msg}");
                    }
                }
            }
            #[cfg(not(feature = "local-embed"))]
            {
                tracing::warn!(
                    "fastembed not compiled in. Build with: cargo build --features local-embed"
                );
            }
            // Fallback to OpenAI if key available
            if !api_key.is_empty() {
                tracing::info!(
                    "Falling back to OpenAI embeddings ({model}) -- NOT externally verifiable"
                );
                Ok(Box::new(OpenAIEmbedder::new(api_key, model)))
            } else {
                Err(
                    "No embedding provider available. Configure one of:\n  \
                     1. Build with: cargo build --features local-embed  (recommended, open weights)\n  \
                     2. Set OPENAI_API_KEY in .env  (proprietary, not externally verifiable)\n\n\
                     The hash embedder has been removed -- recall requires real semantic embeddings.".to_string()
                )
            }
        }
        "openai" if !api_key.is_empty() => {
            tracing::info!(
                "Embedder: OpenAI {model} -- NOT externally verifiable (proprietary model)"
            );
            Ok(Box::new(OpenAIEmbedder::new(api_key, model)))
        }
        "openai" => Err("EMBED_PROVIDER=openai but OPENAI_API_KEY not set.".to_string()),
        other => Err(format!(
            "Unknown EMBED_PROVIDER={other}. Valid: fastembed, openai"
        )),
    }
}

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

#[allow(dead_code)]
pub(crate) fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embedder_correct_dimension() {
        let e = MockEmbedder::new(128);
        assert_eq!(e.embed("hello").len(), 128);
    }

    #[test]
    fn mock_embedder_unit_norm() {
        let e = MockEmbedder::new(128);
        let v = e.embed("hello");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mock_embedder_deterministic() {
        let e = MockEmbedder::new(128);
        assert_eq!(e.embed("hello"), e.embed("hello"));
    }

    #[test]
    fn mock_embedder_different_inputs_same_vector() {
        let e = MockEmbedder::new(128);
        assert_eq!(e.embed("hello"), e.embed("world"));
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let e = MockEmbedder::new(64);
        let v = e.embed("test");
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-5);
    }

    #[test]
    fn build_embedder_no_provider_returns_err() {
        let result = build_embedder("none", "", "");
        assert!(result.is_err());
    }

    #[test]
    fn empty_string_input() {
        let e = MockEmbedder::new(64);
        let v = e.embed("");
        assert_eq!(v.len(), 64);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn zero_dimension_configuration() {
        let e = MockEmbedder::new(0);
        let v = e.embed("test");
        assert!(v.is_empty());
    }
}

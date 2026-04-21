//! TurboQuant compression layer — compresses f32 embeddings for efficient
//! storage on Arweave and compact representation in SQLite.
//!
//! Pipeline: f32 vec → TurboQuant compress → serialized bytes
//!           serialized bytes → TurboQuant decompress → f32 vec (approximate)

use ndarray::Array1;
use turboquant_plus_rs::{pack_bits, pack_indices, TurboQuant};

/// Compressed embedding — serializable to bytes for storage.
#[derive(Debug, Clone)]
pub struct CompressedEmbedding {
    pub dim: usize,
    pub bit_width: usize,
    pub mse_indices_packed: Vec<u8>,
    pub qjl_signs_packed: Vec<u8>,
    pub vector_norm: f32,
    pub residual_norm: f32,
}

impl CompressedEmbedding {
    /// Serialize to bytes for SQLite/Arweave storage.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // Header: dim (4), bit_width (1), vector_norm (4), residual_norm (4)
        buf.extend_from_slice(&(self.dim as u32).to_le_bytes());
        buf.push(self.bit_width as u8);
        buf.extend_from_slice(&self.vector_norm.to_le_bytes());
        buf.extend_from_slice(&self.residual_norm.to_le_bytes());
        // Indices length + data
        buf.extend_from_slice(&(self.mse_indices_packed.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.mse_indices_packed);
        // Signs length + data
        buf.extend_from_slice(&(self.qjl_signs_packed.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.qjl_signs_packed);
        buf
    }

    /// Deserialize from bytes.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        Self::from_bytes_packed_v2(data).or_else(|| Self::from_bytes_legacy_v1(data))
    }

    fn from_bytes_packed_v2(data: &[u8]) -> Option<Self> {
        if data.len() < 21 {
            return None;
        }
        let mut off = 0;

        let dim = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let bit_width = *data.get(off)? as usize;
        off += 1;
        let vector_norm = f32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?);
        off += 4;
        let residual_norm = f32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?);
        off += 4;

        let idx_len = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let mse_indices_packed = data.get(off..off + idx_len)?.to_vec();
        off += idx_len;

        let signs_len = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let qjl_signs_packed = data.get(off..off + signs_len)?.to_vec();

        Some(Self {
            dim,
            bit_width,
            mse_indices_packed,
            qjl_signs_packed,
            vector_norm,
            residual_norm,
        })
    }

    fn from_bytes_legacy_v1(data: &[u8]) -> Option<Self> {
        if data.len() < 25 {
            return None;
        }
        let mut off = 0;

        let dim = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let bit_width = *data.get(off)? as usize;
        off += 1;
        let vector_norm = f64::from_le_bytes(data.get(off..off + 8)?.try_into().ok()?) as f32;
        off += 8;
        let residual_norm = f64::from_le_bytes(data.get(off..off + 8)?.try_into().ok()?) as f32;
        off += 8;

        let idx_len = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let mse_indices = data.get(off..off + idx_len)?;
        off += idx_len;

        let signs_len = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let qjl_signs = data.get(off..off + signs_len)?;

        // Legacy format stored unpacked indices and {-1,+1} signs.
        if idx_len != dim || signs_len != dim || bit_width < 2 {
            return None;
        }
        let mse_indices_unpacked = Array1::from_iter(mse_indices.iter().map(|&v| v as usize));
        let qjl_signs_unpacked = Array1::from_iter(qjl_signs.iter().map(|&b| b as i8));
        let mse_indices_packed = pack_indices(&mse_indices_unpacked, bit_width - 1);
        let qjl_signs_packed = pack_bits(&qjl_signs_unpacked);

        Some(Self {
            dim,
            bit_width,
            mse_indices_packed,
            qjl_signs_packed,
            vector_norm,
            residual_norm,
        })
    }
}

/// Compressor wrapping TurboQuant.
pub struct EmbeddingCompressor {
    tq: TurboQuant,
    dim: usize,
    bit_width: usize,
}

impl EmbeddingCompressor {
    pub fn new(dim: usize, bit_width: usize, seed: u64) -> Self {
        let tq = TurboQuant::new(dim, bit_width, seed, true);
        Self { tq, dim, bit_width }
    }

    /// Compress an f32 embedding vector.
    pub fn compress(&self, embedding: &[f32]) -> CompressedEmbedding {
        let x = Array1::from_iter(embedding.iter().map(|&v| v as f64));
        let compressed = self.tq.quantize(&x);

        // Extract single-vector data from packed batch format.
        let mse_indices_packed = compressed.mse_indices_packed[0].clone();
        let qjl_signs_packed = compressed.qjl_signs_packed[0].clone();
        let vector_norm = compressed.vector_norms[0];
        let residual_norm = compressed.residual_norms[0];

        CompressedEmbedding {
            dim: self.dim,
            bit_width: self.bit_width,
            mse_indices_packed,
            qjl_signs_packed,
            vector_norm,
            residual_norm,
        }
    }

    /// Decompress back to approximate f32 vector.
    pub fn decompress(&self, compressed: &CompressedEmbedding) -> Vec<f32> {
        let cv = turboquant_plus_rs::CompressedVectors {
            mse_indices_packed: vec![compressed.mse_indices_packed.clone()],
            vector_norms: vec![compressed.vector_norm],
            qjl_signs_packed: vec![compressed.qjl_signs_packed.clone()],
            residual_norms: vec![compressed.residual_norm],
            bit_width: compressed.bit_width,
            d: compressed.dim,
        };

        let reconstructed = self.tq.dequantize(&cv);
        reconstructed.row(0).iter().map(|&v| v as f32).collect()
    }

    /// Compression ratio vs f32.
    pub fn compression_ratio(&self) -> f64 {
        self.tq.compression_ratio(32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vector(dim: usize) -> Vec<f32> {
        (0..dim).map(|i| ((i as f32) * 0.01).sin()).collect()
    }

    #[test]
    fn test_compress_decompress_roundtrip() {
        let c = EmbeddingCompressor::new(128, 4, 42);
        let original = sample_vector(128);
        let compressed = c.compress(&original);
        let restored = c.decompress(&compressed);

        assert_eq!(restored.len(), 128);
        // Check approximate reconstruction (TurboQuant is lossy)
        let mse: f32 = original.iter().zip(restored.iter())
            .map(|(a, b)| (a - b).powi(2)).sum::<f32>() / original.len() as f32;
        assert!(mse < 0.1, "MSE too high: {mse}");
    }

    #[test]
    fn test_serialize_deserialize() {
        let c = EmbeddingCompressor::new(128, 4, 42);
        let original = sample_vector(128);
        let compressed = c.compress(&original);

        let bytes = compressed.to_bytes();
        let restored = CompressedEmbedding::from_bytes(&bytes).unwrap();

        assert_eq!(restored.dim, 128);
        assert_eq!(restored.bit_width, 4);
        assert_eq!(
            restored.mse_indices_packed.len(),
            compressed.mse_indices_packed.len()
        );
        assert_eq!(
            restored.qjl_signs_packed.len(),
            compressed.qjl_signs_packed.len()
        );
        assert!((restored.vector_norm - compressed.vector_norm).abs() < 1e-6);
    }

    #[test]
    fn test_compression_ratio() {
        let c = EmbeddingCompressor::new(384, 4, 42);
        let ratio = c.compression_ratio();
        assert!(ratio > 3.0, "Expected >3x compression, got {ratio}");
    }

    #[test]
    fn test_compressed_size_smaller() {
        let c = EmbeddingCompressor::new(384, 4, 42);
        let v = sample_vector(384);
        let compressed = c.compress(&v);
        let bytes = compressed.to_bytes();

        let original_bytes = 384 * 4; // f32 = 4 bytes each
        assert!(bytes.len() < original_bytes,
            "compressed {} >= original {}", bytes.len(), original_bytes);
    }
}

//! Sealed-memory primitives for private / shared Mnemonik memories (Wave 6+).
//!
//! Four co-operating sub-modules implement the sealing pipeline:
//!
//! * [`keys`] — Ed25519 ↔ X25519 key conversion for identity-derived
//!   encryption keys.
//! * [`content`] — Bucket-padded XChaCha20Poly1305 content encryption, plus
//!   key commitment.
//! * [`wrap`] — HPKE (X25519HkdfSha256 / HkdfSha256 / ChaCha20Poly1305)
//!   key-wrapping for per-recipient delivery of the content key.
//! * [`api`] — High-level seal / open / grant / link API (Task 3).

pub mod api;
pub mod content;
pub mod keys;
pub mod wrap;

pub use api::{
    link_fragment, make_grant, open_chunk, open_grant, open_memory, open_with_key,
    parse_link_fragment, seal_chunk, seal_memory, SealError, SealedArtifact, SealedChunk,
};
pub use content::{
    decrypt_content, encrypt_content, key_commitment, pad_to_bucket, unpad, BUCKET_SIZE,
};
pub use keys::{
    x25519_public_from_ed25519, x25519_secret_from_ed25519, x25519_secret_from_solana_keypair,
};
pub use wrap::{unwrap_key, wrap_key, WrapResult};

pub mod stream;

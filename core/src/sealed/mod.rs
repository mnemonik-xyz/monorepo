//! Sealed-memory primitives for private / shared Mnemonik memories (Wave 6+).
//!
//! Three co-operating sub-modules implement the sealing pipeline:
//!
//! * [`keys`] — Ed25519 ↔ X25519 key conversion for identity-derived
//!   encryption keys.
//! * [`content`] — Bucket-padded XChaCha20Poly1305 content encryption, plus
//!   key commitment.
//! * [`wrap`] — HPKE (X25519HkdfSha256 / HkdfSha256 / ChaCha20Poly1305)
//!   key-wrapping for per-recipient delivery of the content key.

pub mod content;
pub mod keys;
pub mod wrap;

pub use content::{
    decrypt_content, encrypt_content, key_commitment, pad_to_bucket, unpad, BUCKET_SIZE,
};
pub use keys::{x25519_public_from_ed25519, x25519_secret_from_ed25519};
pub use wrap::{unwrap_key, wrap_key, WrapResult};

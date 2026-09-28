//! A2A (Agent-to-Agent) protocol v1.0.0-rc codec bindings.
//!
//! Provides type-safe Rust mirrors of the A2A wire types with RFC 8785 JCS
//! canonicalization (`to_jcs_bytes`) and COSE_Sign1 envelope wrapping
//! (`to_canonical_envelope`), following Decision 1 and Decision 5 in
//! work/a2a-bridge/decisions.md.
//!
//! Enabled only with `--features a2a-experimental`.

pub mod artifact;
pub mod extension;
pub mod message;
pub mod part;
pub mod sealed_part;
pub mod task;

pub use artifact::A2aArtifact;
pub use extension::{
    build_x_mnemonic_extension, extract_x_mnemonic, verify_card_covers_extension,
    XMnemonicExtension, CONFORMANCE_VERSION, EXTENSION_URI,
};
pub use message::Message;
pub use part::Part;
pub use sealed_part::{
    build_sealed_data_part, extract_sealed_data_part, recipient_x25519_from_agent_card,
    SealedPartError, SealedPartPayload, SEALED_CBOR_MEDIA_TYPE,
};
pub use task::Task;

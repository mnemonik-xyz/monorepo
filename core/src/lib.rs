// WASM-compatible modules — pure-Rust, no I/O, no FFI.
pub mod codec;
pub mod compress;
pub mod identity;

// Native-only modules. These pull in rusqlite (bundled C SQLite), reqwest
// (native TLS), or fastembed (ONNX runtime), none of which compile for
// wasm32-unknown-unknown. Gating here keeps the WASM build minimal and means
// the `core/src/wasm/` surface composes only the cryptographic primitives.
#[cfg(not(target_arch = "wasm32"))]
pub mod arweave;
#[cfg(not(target_arch = "wasm32"))]
pub mod embed;
#[cfg(not(target_arch = "wasm32"))]
pub mod lineage;
#[cfg(not(target_arch = "wasm32"))]
pub mod solana;
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;

// Browser-facing wasm-bindgen wrappers. Compiled only when targeting
// wasm32-unknown-unknown AND the `wasm` feature is enabled. Native `cargo build
// --workspace` paths never see this module (architectural invariant).
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub mod wasm;

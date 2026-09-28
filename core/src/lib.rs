// WASM-compatible modules — pure-Rust, no I/O, no FFI.
pub mod codec;
pub mod compress;
pub mod identity;
// Per-owner Merkle commitments over content_hashes (verifiable recall, §16).
// Pure crypto — available on every target so clients can verify proofs.
pub mod merkle;

// Rebuild the recall index from stored signed artifacts (verifiable recall,
// §16) — makes "SQLite is a rebuildable cache, not a source of truth" real.
// Portable since work/arweave-as-source-of-truth Wave 2: it uses only `codec`,
// `compress` and `base64`, all of which are in the shared dependency table, so
// a browser client can restore its own index from Arweave without a native
// binary. The earlier "native-only (depends on the compressor)" gate was stale
// — `compress` is portable, listed above.
//
// Discovery stays native: `arweave::recovery` and `arweave::graphql` use
// `reqwest`. A wasm client enumerates its items with `fetch` in TypeScript and
// calls the per-artifact rebuild exported from `core::wasm`.
pub mod rebuild;

// Verifiable trajectories (work/verifiable-trajectories/): ordered, hash-linked,
// signed agent steps + independent verdicts. Pure (codec + merkle only), so the
// same chain/coverage verification runs client-side and against any backend —
// the decentralized, stateless-MCP direction. Experimental; opt-in feature.
#[cfg(feature = "trajectory-experimental")]
pub mod trajectory;

// Native-only modules. These pull in rusqlite (bundled C SQLite), reqwest
// (native TLS), or fastembed (ONNX runtime), none of which compile for
// wasm32-unknown-unknown. Gating here keeps the WASM build minimal and means
// the `core/src/wasm/` surface composes only the cryptographic primitives.
#[cfg(not(target_arch = "wasm32"))]
pub mod arweave;
#[cfg(not(target_arch = "wasm32"))]
pub mod embed;
// X25519 ECIES sealing for shared/private memories (Wave 6, §17). Native-only
// for now (crypto_box + OsRng); in-browser (wasm) decryption for the webapp
// client is a follow-up that must verify the wasm RNG wiring against the
// hard `cross-lang-build` gate before flipping this on.
#[cfg(not(target_arch = "wasm32"))]
pub mod encrypt;
// HPKE-based sealed memories (Wave 6+, §17): bucket-padded XChaCha20Poly1305
// content encryption + X25519HkdfSha256 key-wrapping, with Ed25519→X25519 key
// derivation from the identity keypair.  Native-only: uses OsRng and the hpke
// `std` feature; a wasm port would require a CSPRNG wiring audit first.
#[cfg(not(target_arch = "wasm32"))]
pub mod sealed;
#[cfg(not(target_arch = "wasm32"))]
pub mod lineage;
// Restore a recall index from Arweave (work/arweave-as-source-of-truth Wave 3).
// Native-only because enumeration and fetching use `reqwest`; the per-artifact
// rebuild it calls is portable and also exported to wasm.
#[cfg(not(target_arch = "wasm32"))]
pub mod restore;
#[cfg(not(target_arch = "wasm32"))]
pub mod solana;
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;

// Browser-facing wasm-bindgen wrappers. Compiled only when targeting
// wasm32-unknown-unknown AND the `wasm` feature is enabled. Native `cargo build
// --workspace` paths never see this module (architectural invariant).
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub mod wasm;

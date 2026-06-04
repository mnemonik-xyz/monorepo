//! Shared fixtures for `mcp/tests/*.rs` integration tests.
//!
//! Integration test files in Cargo are compiled as separate crates, so any
//! `pub`-ish helper in one file is invisible to siblings. The conventional
//! Rust workaround is `mod support;` at the top of each test file pointing
//! to this module, which then re-exports the fixtures.
//!
//! Currently only [`FailingEmbedder`] lives here (consolidating the inline
//! copies in `error_catalogue.rs` and `sign_memory_softfall.rs` per Task 5
//! round-2 test-reviewer finding F-4 — agent-native-distribution).

use std::sync::atomic::{AtomicUsize, Ordering};

use mnemonic_core::embed::Embedder;

/// Embedder fixture that returns `Vec::new()` on the configured call,
/// surfacing the `-32098 EmbedderInvalid` error catalogue row through the
/// production code path in `sign_memory_inline` (which treats an empty
/// vector as the failure signal).
///
/// `dim` is configurable so the test that drives the failure path can
/// match the compressor's dim — production-shape tests pass `dim=384` to
/// mirror the real `FastEmbedder` shape; soft-fall tests pass `dim=8` to
/// stay aligned with the workspace's deterministic 8-dim test compressor.
/// The dim never matters in practice — the only `embed()` call returns
/// `Vec::new()` and short-circuits before the compressor runs.
pub struct FailingEmbedder {
    /// Counter that, when reached, makes `embed()` start returning
    /// `Vec::new()`. The default (1) means "fail on the very first call".
    pub fail_on_call: AtomicUsize,
    /// Atomic call counter incremented on every `embed()` invocation.
    pub counter: AtomicUsize,
    dim: usize,
}

impl FailingEmbedder {
    /// Fail on the first call. Default dim = 8 (matches the rest of the
    /// integration suite's test compressor). Tests that want a different
    /// dim can use [`FailingEmbedder::with_dim`].
    pub fn always_fail() -> Self {
        Self {
            fail_on_call: AtomicUsize::new(1),
            counter: AtomicUsize::new(0),
            dim: 8,
        }
    }

    /// Fail on the first call with a specific dim. Defined for parity with
    /// the production `FastEmbedder` shape (`dim = 384`) when a future
    /// test needs a non-default dim; currently unused but kept on the
    /// public surface so the fixture is composable.
    #[allow(dead_code)]
    pub fn with_dim(dim: usize) -> Self {
        Self {
            fail_on_call: AtomicUsize::new(1),
            counter: AtomicUsize::new(0),
            dim,
        }
    }
}

impl Default for FailingEmbedder {
    fn default() -> Self {
        Self::always_fail()
    }
}

impl Embedder for FailingEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        if n + 1 >= self.fail_on_call.load(Ordering::SeqCst) {
            Vec::new()
        } else {
            vec![0.0; self.dim]
        }
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn provider_name(&self) -> &str {
        "failing"
    }
    fn model_id(&self) -> &str {
        "test/failing-embedder"
    }
}

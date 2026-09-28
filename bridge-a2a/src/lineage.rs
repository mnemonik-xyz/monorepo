//! In-memory lineage map: contextId → last attestation_id.
//!
//! Best-effort: if the bridge restarts, lineage continuation falls back to
//! `None` (documented in decisions.md Decision lineage).

use std::collections::HashMap;
use std::sync::Mutex;

/// Thread-safe map from `contextId` to the most recent `attestation_id` in
/// that context.
pub struct LineageMap {
    inner: Mutex<HashMap<String, String>>,
}

impl LineageMap {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Look up the previous attestation id for a context.
    pub fn get(&self, context_id: &str) -> Option<String> {
        let map = self.inner.lock().expect("lineage lock poisoned");
        map.get(context_id).cloned()
    }

    /// Advance the lineage: update `contextId → new_id`.
    ///
    /// Returns the previous value (useful for callers that need to pass
    /// `prev_id` to an `attest_*` call *before* updating).
    pub fn advance(&self, context_id: &str, new_id: &str) -> Option<String> {
        let mut map = self.inner.lock().expect("lineage lock poisoned");
        let prev = map.get(context_id).cloned();
        map.insert(context_id.to_string(), new_id.to_string());
        prev
    }
}

impl Default for LineageMap {
    fn default() -> Self {
        Self::new()
    }
}

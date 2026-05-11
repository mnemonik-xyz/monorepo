// T19 — Stub. Tracked as follow-up after T19 partial-ship lands.
// Seed IDB with 5 deterministic attestations (re-use the chatgpt-
// capture's IDB-seed pattern), open the recall overlay via the hotkey,
// type a query, assert the rendered hit list, click "Insert" and
// confirm the target textarea receives the chosen attestation's
// content. Uses setup.ts harness.

import { test } from "@playwright/test";

test.skip("recall_overlay_search_and_insert_into_target_textarea", () => {
  // TODO(T19-follow-up): mount the recall overlay content script
  // against a synthetic data:text/html page with a <textarea>, then
  // exercise the search → insert path. Use the existing recall-
  // embedder mock so the cosine sort is deterministic.
});

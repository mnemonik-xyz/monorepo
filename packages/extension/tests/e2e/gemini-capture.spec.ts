// T19 — Stub. Tracked as follow-up after T19 partial-ship lands.
// Clone the chatgpt-capture.spec.ts pattern with the Gemini fixture
// (tests/fixtures/gemini/code.html) and the Gemini adapter selectors
// (model-response / user-query elements). Asserts the same anchors:
// 4 turns, code-block fence preservation, IDB row by attestation_id.

import { test } from "@playwright/test";

test.skip("gemini_capture_full_chat_serialized_with_code_blocks", () => {
  // TODO(T19-follow-up): implement Gemini capture E2E. Mirror the
  // chatgpt-capture spec; swap fixture path + adapter selectors.
});

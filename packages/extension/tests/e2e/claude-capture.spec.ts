// T19 — Stub. Tracked as follow-up after T19 partial-ship lands.
// Clone the chatgpt-capture.spec.ts pattern with the Claude fixture
// (tests/fixtures/claude/code.html) and the Claude adapter selector
// (`[data-test-render-count]` rather than `[data-message-author-role]`).
// Same anchors: 4 turns extracted, code blocks fenced with the right
// language hint, IDB row persists keyed by attestation_id.

import { test } from "@playwright/test";

test.skip("claude_capture_full_chat_serialized_with_code_blocks", () => {
  // TODO(T19-follow-up): implement Claude capture E2E. Mirror the
  // chatgpt-capture spec; swap fixture path + adapter selectors.
});

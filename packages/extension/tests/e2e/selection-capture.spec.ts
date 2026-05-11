// T19 — Stub. Tracked as follow-up after T19 partial-ship lands.
// `data:text/html` fixture page with a <p> selection → trigger the
// extension's selection-capture content script via the context menu
// command → assert an IndexedDB row lands with the selected text and
// a `source:selection` tag. Uses setup.ts harness + server.ts mock.

import { test } from "@playwright/test";

test.skip("generic_page_selection_creates_idb_row", () => {
  // TODO(T19-follow-up): drive `chrome.contextMenus` from the popup
  // realm or message the SW directly to simulate the selection-save
  // command path. Assert the resulting IDB row content + tag.
});

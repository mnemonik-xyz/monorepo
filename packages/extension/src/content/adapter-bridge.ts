// Content-script bridge for the popup-realm `getActiveTabSelection` /
// `getActiveTabConversation` runtime helpers. The popup sends
// `ui:get-selection` and `ui:extract-conversation` via
// `chrome.tabs.sendMessage`; this module installs a single
// `chrome.runtime.onMessage` listener inside the content-script realm
// that handles both message types and returns the requested payload.
//
// The listener is registered at import time. Each adapter file (which
// is loaded as the per-origin content script by the manifest) imports
// this module so the listener is wired automatically on first script
// evaluation. The registration is idempotent — repeated imports across
// the FAB + recall-overlay + adapter content scripts won't double-fire
// because `chrome.runtime.onMessage.addListener` deduplicates by
// function identity on the platform side; we additionally guard with a
// module-level flag for clarity.

import { selectAdapter } from "../runtime/chat/registry.js";

let installed = false;

interface AdapterRequest {
  type?: unknown;
}

/** Install the bridge listener. Safe to call multiple times. Returns
 *  the listener function so callers (tests) can remove it explicitly. */
export function installAdapterBridge(): void {
  if (installed) return;
  if (typeof chrome === "undefined" || !chrome.runtime?.onMessage) return;
  installed = true;
  chrome.runtime.onMessage.addListener(
    (request: unknown, _sender, sendResponse) => {
      if (typeof request !== "object" || request === null) return false;
      const t = (request as AdapterRequest).type;
      if (t === "ui:get-selection") {
        try {
          const selection =
            typeof window !== "undefined"
              ? (window.getSelection?.()?.toString() ?? "")
              : "";
          sendResponse({ selectionText: selection });
        } catch {
          sendResponse({ selectionText: "" });
        }
        return true;
      }
      if (t === "ui:extract-conversation") {
        try {
          const adapter = selectAdapter(location.href);
          if (!adapter) {
            sendResponse({ turns: null });
            return true;
          }
          const turns = adapter.extractConversation(document);
          sendResponse({ turns });
        } catch {
          sendResponse({ turns: null });
        }
        return true;
      }
      return false;
    },
  );
}

// Side-effect install at import time. Adapter content scripts pick
// this up automatically (each adapter imports this module from its
// init footer). Idempotent — calling installAdapterBridge() again is a
// no-op after the first registration.
installAdapterBridge();

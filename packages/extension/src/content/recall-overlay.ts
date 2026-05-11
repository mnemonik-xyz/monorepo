/// <reference types="chrome" />

// Recall-overlay content-script entry. T13 owns the real overlay UI;
// this stub registers a no-op message listener so the SW → tab dispatch
// path (`sw:open-recall-overlay`) doesn't error on missing receivers.
//
// Per D11 the overlay is only injected on enumerated AI-chat domains
// via `content_scripts`; on generic pages it's invoked via the popup
// hotkey path (`activeTab` + scripting injection) and is therefore not
// in this file.

import { parseMsg } from "../messages.js";

if (typeof chrome !== "undefined" && chrome.runtime?.onMessage) {
  chrome.runtime.onMessage.addListener((raw: unknown) => {
    const msg = parseMsg(raw);
    if (msg?.type === "sw:open-recall-overlay") {
      // T13: open the overlay. Stub left intentionally inert.
    }
  });
}

export {};

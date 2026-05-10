/// <reference types="chrome" />

// MV3 service worker entry. Owns:
//   - install-time `chrome.contextMenus` registration (save-selection).
//   - the `chrome.contextMenus.onClicked` dispatcher — relays the
//     selected text + page URL to the active tab's content script
//     (generic page capture per D8 / D11; auto-capture stays opt-in per
//     D12 — this path is user-gesture initiated).
//   - the `chrome.commands.onCommand` hotkey dispatcher.
//   - the typed `chrome.runtime.onMessage` router (see `src/messages.ts`).
//   - the `chrome.alarms` cloud-sync-retry tick that drains the
//     `pending_uploads` queue via `runtime/sync/cloud-client.ts` (T18).
//
// Hard rules per the tech-spec:
//   - No `<all_urls>` content-script injection (D11).
//   - No persistent listeners that capture without user gesture (D12).
//   - Strict TypeScript: no `any` in public types — narrow `unknown` via
//     `parseMsg` from `src/messages.ts`.

import { IndexedDbStore } from "../runtime/store/indexeddb.js";
import { flushPending } from "../runtime/sync/cloud-client.js";
import { parseMsg, type Msg, type SaveSelectionPayload } from "../messages.js";

/** Context-menu item id — keep stable; `chrome.contextMenus.create`
 *  is idempotent on this id across SW restarts. */
const CTX_MENU_SAVE_SELECTION = "save-selection";

/** Alarm name for the cloud-sync queue drain. */
const ALARM_CLOUD_SYNC_RETRY = "cloud-sync-retry";

/** Period (minutes) — matches the tech-spec; tune via D7 follow-up if
 *  cloud-tier traffic grows. Five minutes balances battery / freshness. */
const ALARM_PERIOD_MIN = 5;

/**
 * Hooks the SW can swap out in tests. Production wiring uses the real
 * `chrome.*` globals + `IndexedDbStore`; tests inject mocks via
 * `installServiceWorker({ ... })`.
 */
export interface ServiceWorkerDeps {
  chrome: typeof chrome;
  /** Constructed lazily so unit tests can pass a fake-indexeddb store. */
  storeFactory: () => IndexedDbStore;
  /** Indirection so the alarm-drain test can spy without polluting the
   *  module-level binding. */
  flushPending: typeof flushPending;
  /** ISO-8601 clock — defaults to `Date.now`, swappable for deterministic
   *  tests. */
  now: () => string;
}

/** Production deps factory — keep cheap; called once per SW boot. */
function defaultDeps(): ServiceWorkerDeps {
  return {
    chrome,
    storeFactory: () => new IndexedDbStore(),
    flushPending,
    now: () => new Date().toISOString(),
  };
}

/**
 * Wire all listeners against `deps`. Exposed so tests can install the
 * SW against mocked `chrome.*` APIs. Safe to call once per SW boot —
 * Chrome dedupes `contextMenus.create` by id; alarms by name.
 */
export function installServiceWorker(deps: ServiceWorkerDeps): void {
  const { chrome: cr } = deps;

  // ── install / activation ─────────────────────────────────────────────
  cr.runtime.onInstalled.addListener(() => {
    cr.contextMenus.create({
      id: CTX_MENU_SAVE_SELECTION,
      title: "Save selection to Mnemonik",
      contexts: ["selection"],
    });
    cr.alarms.create(ALARM_CLOUD_SYNC_RETRY, {
      periodInMinutes: ALARM_PERIOD_MIN,
    });
  });

  // ── context-menu → active tab ────────────────────────────────────────
  cr.contextMenus.onClicked.addListener((info, tab) => {
    if (info.menuItemId !== CTX_MENU_SAVE_SELECTION) return;
    if (tab?.id === undefined) return;
    const selectionText = info.selectionText ?? "";
    const pageUrl = info.pageUrl ?? tab.url ?? "";
    if (!selectionText) return;
    const payload: SaveSelectionPayload = {
      selectionText,
      pageUrl,
      capturedAt: deps.now(),
      ...(tab.title !== undefined ? { pageTitle: tab.title } : {}),
    };
    void cr.tabs.sendMessage(tab.id, {
      type: "sw:save-selection",
      payload,
    });
  });

  // ── hotkeys ──────────────────────────────────────────────────────────
  cr.commands.onCommand.addListener((command, tab) => {
    if (command === "recall-overlay") {
      if (tab?.id === undefined) return;
      void cr.tabs.sendMessage(tab.id, {
        type: "sw:open-recall-overlay",
        payload: { trigger: "hotkey" },
      });
    }
    // `_execute_action` is handled directly by Chrome — no SW dispatch
    // needed; documented here so future maintainers don't re-add it.
  });

  // ── runtime message router ───────────────────────────────────────────
  cr.runtime.onMessage.addListener(
    (raw: unknown, _sender, sendResponse: (response: unknown) => void) => {
      const msg = parseMsg(raw);
      if (msg === null) {
        sendResponse({ ok: false, error: "unknown-message" });
        return false;
      }
      // Each branch is `async` — we return `true` to keep `sendResponse`
      // alive across the await (MV3 contract).
      void handleMsg(deps, msg)
        .then((result) => sendResponse({ ok: true, result }))
        .catch((err: unknown) => {
          const message = err instanceof Error ? err.message : String(err);
          sendResponse({ ok: false, error: message });
        });
      return true;
    }
  );

  // ── alarms: drain cloud-sync queue ───────────────────────────────────
  cr.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name !== ALARM_CLOUD_SYNC_RETRY) return;
    const store = deps.storeFactory();
    void deps.flushPending({ store }).catch((err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      console.warn("[mnemonik] flushPending failed:", message);
    });
  });
}

/**
 * Route a parsed message to its handler. Returns the handler's result
 * so the `onMessage` listener can ship it back via `sendResponse`.
 *
 * T11 / T13 / T18 will plug their concrete runtime calls into the
 * matching branches; for now most branches resolve to a stub object so
 * the contract is observable in tests.
 */
async function handleMsg(deps: ServiceWorkerDeps, msg: Msg): Promise<unknown> {
  switch (msg.type) {
    case "ui:flush-pending": {
      const store = deps.storeFactory();
      return deps.flushPending({ store });
    }
    case "ui:sign-memory":
      // T11 / T18: delegate to runtime/sign + runtime/store. The SW
      // does not embed business logic — it routes.
      return { deferred: "sign-memory" };
    case "ui:recall":
      return { deferred: "recall" };
    case "sw:open-recall-overlay":
    case "sw:save-selection":
    case "tab:capture-candidate":
      // These are inbound-from-tab / outbound-to-tab payloads; the SW
      // doesn't process them via the request/response channel (the
      // sender dispatched directly to the recipient).
      return { acknowledged: true };
  }
}

// Boot once the SW module is evaluated. Tests import
// `installServiceWorker` and call it explicitly against a mocked
// `chrome.*` — the boot below is guarded so test imports stay
// side-effect free.
if (typeof chrome !== "undefined" && typeof chrome.runtime !== "undefined") {
  installServiceWorker(defaultDeps());
}

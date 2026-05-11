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
import { CloudClient } from "../runtime/sync/cloud-client.js";
import {
  flushPending,
  type FlushPendingDeps,
  type SyncEvent,
} from "./cloud-sync.js";
import { parseMsg, type Msg, type SaveSelectionPayload } from "../messages.js";

/** Context-menu item id — keep stable; `chrome.contextMenus.create`
 *  is idempotent on this id across SW restarts. */
const CTX_MENU_SAVE_SELECTION = "save-selection";

/** Alarm name for the cloud-sync queue drain. */
const ALARM_CLOUD_SYNC_RETRY = "cloud-sync-retry";

/** Period (minutes) — matches the tech-spec; tune via D7 follow-up if
 *  cloud-tier traffic grows. Five minutes balances battery / freshness. */
const ALARM_PERIOD_MIN = 5;

/** Origins that are allowed to send `tab:*` messages from a content
 *  script. Mirrors `host_permissions` / `content_scripts.matches` in
 *  `manifest.json`. Keep in sync if a new AI-chat domain ships. */
const ALLOWED_TAB_ORIGINS: readonly string[] = [
  "https://chatgpt.com",
  "https://claude.ai",
  "https://gemini.google.com",
];

/**
 * Module-scoped guard for the cloud-sync-retry alarm. Prevents two
 * concurrent flushes when a previous run is still in flight (slow
 * network / paused tab). Closes review round-1 finding T10-S-07. */
let cloudSyncInFlight = false;

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
  /** Build a cloud client for one alarm tick — returns `null` when
   *  there is no current session (the drain short-circuits in that
   *  case and leaves the queue untouched). Defaults to reading
   *  `session.v1` out of `chrome.storage.local`. */
  cloudClientProvider?: () => Promise<CloudClient | null>;
  /** Sink the alarm-drain calls when it observes a sync event (401 /
   *  permanent failure). The default production wiring forwards to
   *  `chrome.runtime.sendMessage` so the popup can react. T18
   *  routes `re-auth-required` here. */
  emitSyncEvent?: (event: SyncEvent) => void;
  /** ISO-8601 clock — defaults to `() => new Date().toISOString()`;
   *  swappable for deterministic tests. */
  now: () => string;
}

/** Production deps factory — keep cheap; called once per SW boot. */
function defaultDeps(): ServiceWorkerDeps {
  return {
    chrome,
    storeFactory: () => new IndexedDbStore(),
    flushPending,
    cloudClientProvider: defaultCloudClientProvider,
    emitSyncEvent: defaultEmitSyncEvent,
    now: () => new Date().toISOString(),
  };
}

/**
 * Default `cloudClientProvider`. Reads the persisted T16 session
 * out of `chrome.storage.local`; returns `null` when no session is
 * present (or when the JWT has expired — `getSession` already
 * gates on `jwtExpiresAt`). Lazy-imports `auth/session.js` so the
 * SW boot path stays free of the JWT helpers when no alarm has
 * fired yet.
 */
async function defaultCloudClientProvider(): Promise<CloudClient | null> {
  try {
    const { getSession } = await import("../auth/session.js");
    const session = await getSession();
    if (!session) return null;
    return new CloudClient({ jwt: session.jwt });
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    console.warn(
      "[mnemonik] cloud-client provider failed; skipping drain:",
      message,
    );
    return null;
  }
}

/**
 * Default `emitSyncEvent`. Forwards to the popup via
 * `chrome.runtime.sendMessage`. The popup listens for
 * `ui:re-auth-required` and routes back to the T16 sign-in flow.
 * Wrapped in try/catch because `sendMessage` rejects when no popup
 * is open — that's expected and not an error.
 */
function defaultEmitSyncEvent(event: SyncEvent): void {
  try {
    void chrome.runtime
      .sendMessage({
        type: "ui:sync-event",
        payload: event,
      })
      .catch(() => {
        // No popup open / no listener registered — silently ignore.
      });
  } catch {
    // chrome.runtime unavailable (tests / cold-boot path).
  }
}

/**
 * Wire all listeners against `deps`. Exposed so tests can install the
 * SW against mocked `chrome.*` APIs. Safe to call once per SW boot:
 * `contextMenus.create` is preceded by `removeAll()` so update / reload
 * paths don't surface `"Cannot create item with duplicate id"` on
 * `chrome.runtime.lastError`; `alarms.create` is idempotent by name.
 */
export function installServiceWorker(deps: ServiceWorkerDeps): void {
  const { chrome: cr } = deps;

  // ── install / activation ─────────────────────────────────────────────
  cr.runtime.onInstalled.addListener(() => {
    // `contextMenus.create` does NOT dedupe by id — on the `update`
    // reason Chrome sets `chrome.runtime.lastError` ("Cannot create
    // item with duplicate id") because context menus persist across SW
    // restarts. `removeAll` clears any prior registration so the
    // subsequent create is always fresh. Closes review round-1 major
    // finding on this path.
    cr.contextMenus.removeAll(() => {
      cr.contextMenus.create({
        id: CTX_MENU_SAVE_SELECTION,
        title: "Save selection to Mnemonik",
        contexts: ["selection"],
      });
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
    (
      raw: unknown,
      sender: chrome.runtime.MessageSender,
      sendResponse: (response: unknown) => void,
    ) => {
      const msg = parseMsg(raw);
      if (msg === null) {
        sendResponse({ ok: false, error: "unknown-message" });
        return false;
      }
      // Sender validation closes review round-1 finding T10-S-02 (the
      // `_sender` parameter was previously ignored). Cross-extension
      // sends are already blocked by the absence of
      // `externally_connectable`, but a compromised content script on
      // one of the enumerated origins (chatgpt/claude/gemini) could
      // still emit a message at the SW. Verify the sender id matches
      // our extension, and that tab-origin messages come from one of
      // the host-permission origins. Popup / options page also live
      // under `chrome-extension://<id>/` so they pass the same id
      // check trivially. */
      if (!isAuthorisedSender(cr, sender, msg)) {
        console.warn(
          "[mnemonik] rejected message from unauthorised sender",
          msg.type,
          sender.id,
          sender.url ?? sender.tab?.url ?? "<no url>",
        );
        sendResponse({ ok: false, error: "unauthorized-sender" });
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
    },
  );

  // ── alarms: drain cloud-sync queue ───────────────────────────────────
  cr.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name !== ALARM_CLOUD_SYNC_RETRY) return;
    // In-flight guard: if a previous tick is still running, skip this
    // one rather than racing two concurrent flushes against the same
    // IndexedDB queue. T18 will inherit this contract.
    if (cloudSyncInFlight) {
      console.warn(
        "[mnemonik] cloud-sync-retry skipped: previous flush still in flight",
      );
      return;
    }
    cloudSyncInFlight = true;
    const store = deps.storeFactory();
    void runFlush(deps, store)
      .catch((err: unknown) => {
        const message = err instanceof Error ? err.message : String(err);
        console.warn("[mnemonik] flushPending failed:", message);
      })
      .finally(() => {
        // CRITICAL: reset the guard in `finally` so a thrown error in
        // any leg of the drain doesn't permanently wedge the alarm
        // (next tick would skip with "previous flush still in
        // flight"). Inherited from the T10 round-2 hardening.
        cloudSyncInFlight = false;
      });
  });
}

/**
 * Compose the per-tick `FlushPendingDeps` and call `deps.flushPending`.
 * Resolves the cloud client lazily so we don't burn a JWT decode on
 * empty queues. Shared by the alarm handler and the explicit
 * `ui:flush-pending` UI gesture so both paths emit `re-auth-required`
 * the same way.
 */
async function runFlush(
  deps: ServiceWorkerDeps,
  store: IndexedDbStore,
): Promise<unknown> {
  const cloudClient = deps.cloudClientProvider
    ? await deps.cloudClientProvider()
    : null;
  const args: FlushPendingDeps = {
    store,
    cloudClient,
    ...(deps.emitSyncEvent ? { emit: deps.emitSyncEvent } : {}),
  };
  return deps.flushPending(args);
}

/**
 * Authorise `sender` for a given message. Returns false when:
 *   - `sender.id` is not our own extension id (defends against the
 *     theoretical case of a co-installed extension forging messages
 *     even without `externally_connectable`).
 *   - For tab-origin (`tab:*`) messages: the tab origin must match one
 *     of the AI-chat origins listed in `host_permissions`.
 *   - For UI-origin (`ui:*`) messages: the sender must be a popup /
 *     options page served from our own extension origin (no tab id).
 *   - For SW-origin (`sw:*`) messages: these are dispatched by the SW
 *     itself and never received via `runtime.onMessage`; reject if
 *     they appear here.
 */
function isAuthorisedSender(
  cr: typeof chrome,
  sender: chrome.runtime.MessageSender,
  msg: Msg,
): boolean {
  // Our own extension id must match. `chrome.runtime.id` is the
  // canonical source of truth at runtime. If sender.id is set, it
  // MUST equal ownId — the previous `&& sender.id` short-circuit
  // silently allowed undefined/empty sender.id through, which is
  // the audit finding T10-N2-01.
  const ownId = cr.runtime?.id;
  if (
    typeof ownId === "string" &&
    sender.id !== undefined &&
    sender.id !== ownId
  ) {
    return false;
  }

  // `sw:*` messages are never inbound — the SW only emits them.
  if (
    msg.type === "sw:save-selection" ||
    msg.type === "sw:open-recall-overlay"
  ) {
    return false;
  }

  // Tab-origin: must come from a tab whose URL matches one of the
  // declared host permissions.
  if (msg.type === "tab:capture-candidate") {
    const tabUrl = sender.tab?.url ?? sender.url;
    if (typeof tabUrl !== "string") return false;
    return ALLOWED_TAB_ORIGINS.some((origin) =>
      tabUrl.startsWith(origin + "/"),
    );
  }

  // UI-origin (`ui:*`) — popup / options page. No `sender.tab`. We
  // require POSITIVE identification: either `sender.id` matches our
  // own extension id, or `sender.url` starts with our extension's
  // chrome-extension:// origin. Both being absent is rejected (was
  // the silent pass-through documented in audit T10-N2-01).
  if (
    msg.type === "ui:sign-memory" ||
    msg.type === "ui:recall" ||
    msg.type === "ui:flush-pending"
  ) {
    if (sender.tab !== undefined) return false;
    if (typeof ownId !== "string") return false;
    const idMatches = sender.id === ownId;
    const urlMatches =
      typeof sender.url === "string" &&
      sender.url.startsWith(`chrome-extension://${ownId}/`);
    return idMatches || urlMatches;
  }

  // Unknown msg.type would have been rejected by parseMsg; this branch
  // is unreachable but keeps the type-checker happy.
  return false;
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
      return runFlush(deps, store);
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

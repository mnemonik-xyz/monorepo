// T10 TDD anchors. D13 requires both to ship green on the PR's CI run:
//
//   1. context_menu_save_selection_emits_message — simulate
//      `chrome.contextMenus.onClicked` for the `save-selection` item →
//      assert the SW dispatches a `sw:save-selection` message to the
//      active tab with `selectionText` + `pageUrl`. Drives the
//      generic-capture flow that D11 keeps gated behind `activeTab` +
//      user gesture.
//
//   2. alarm_drains_pending_queue — populate `IndexedDbStore.pending`
//      via `enqueue(...)`, fire `chrome.alarms.onAlarm` with name
//      `cloud-sync-retry` → assert `cloud-client.flushPending` was
//      called with the store (T18 will swap the no-op body for a real
//      drain; the contract under test is the SW wiring).
//
// The chrome.* mock is hand-rolled (no `@webext-pegasus/transport-chrome`
// dep needed at this scope). Listeners are captured in arrays so the
// test can fire them at will.

import "fake-indexeddb/auto";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  installServiceWorker,
  type ServiceWorkerDeps,
} from "../../../src/background/service-worker.js";
import { IndexedDbStore } from "../../../src/runtime/store/indexeddb.js";

type Listener<Args extends unknown[]> = (...args: Args) => void;

/** Generic single-event mock matching `chrome.events.Event` shape. */
function makeEvent<Args extends unknown[]>() {
  const listeners: Listener<Args>[] = [];
  return {
    listeners,
    addListener: (cb: Listener<Args>) => {
      listeners.push(cb);
    },
    fire: (...args: Args) => {
      for (const cb of listeners) cb(...args);
    },
  };
}

/** Constant test extension id — `chrome.runtime.id` analogue. The
 *  service-worker validates `sender.id` against this; tests pass it as
 *  the synthetic id of their MessageSender. */
const TEST_EXT_ID = "mnemonik-test-extension-id";

interface ChromeMock {
  runtime: {
    id: string;
    onInstalled: ReturnType<typeof makeEvent<[{ reason: string }]>>;
    onStartup: ReturnType<typeof makeEvent<[]>>;
    onMessage: ReturnType<
      typeof makeEvent<
        [unknown, chrome.runtime.MessageSender, (response: unknown) => void]
      >
    >;
  };
  contextMenus: {
    create: ReturnType<typeof vi.fn>;
    removeAll: ReturnType<typeof vi.fn>;
    onClicked: ReturnType<
      typeof makeEvent<[chrome.contextMenus.OnClickData, chrome.tabs.Tab?]>
    >;
  };
  commands: {
    onCommand: ReturnType<typeof makeEvent<[string, chrome.tabs.Tab?]>>;
  };
  alarms: {
    create: ReturnType<typeof vi.fn>;
    onAlarm: ReturnType<typeof makeEvent<[chrome.alarms.Alarm]>>;
  };
  tabs: {
    sendMessage: ReturnType<typeof vi.fn>;
  };
}

function makeChromeMock(): ChromeMock {
  return {
    runtime: {
      id: TEST_EXT_ID,
      onInstalled: makeEvent<[{ reason: string }]>(),
      onStartup: makeEvent<[]>(),
      onMessage: makeEvent(),
    },
    contextMenus: {
      // `removeAll` invokes its callback synchronously so the SW's
      // subsequent `create` runs inside the same install handler.
      create: vi.fn(),
      removeAll: vi.fn((cb?: unknown): unknown => {
        if (typeof cb === "function") (cb as () => void)();
        return undefined;
      }),
      onClicked: makeEvent(),
    },
    commands: {
      onCommand: makeEvent(),
    },
    alarms: {
      create: vi.fn(),
      onAlarm: makeEvent(),
    },
    tabs: {
      sendMessage: vi.fn().mockResolvedValue(undefined),
    },
  };
}

/** Construct a MessageSender that passes the service-worker's
 *  `isAuthorisedSender` gate for `ui:*` messages (popup origin). */
function popupSender(): chrome.runtime.MessageSender {
  return {
    id: TEST_EXT_ID,
    url: `chrome-extension://${TEST_EXT_ID}/popup/index.html`,
  } as chrome.runtime.MessageSender;
}

function uniqueDbName(suffix: string): string {
  return `mnemonik-sw-test-${suffix}-${Math.random().toString(36).slice(2)}`;
}

function makeDeps(overrides: Partial<ServiceWorkerDeps> = {}): {
  deps: ServiceWorkerDeps;
  cr: ChromeMock;
  flushSpy: ReturnType<typeof vi.fn>;
  store: IndexedDbStore;
} {
  const cr = makeChromeMock();
  const store = new IndexedDbStore({ dbName: uniqueDbName("store") });
  const flushSpy = vi.fn().mockResolvedValue({ attempted: 0, flushed: 0 });
  const deps: ServiceWorkerDeps = {
    chrome: cr as unknown as typeof chrome,
    storeFactory: () => store,
    flushPending: flushSpy as unknown as ServiceWorkerDeps["flushPending"],
    now: () => "2026-05-11T00:00:00.000Z",
    ...overrides,
  };
  return { deps, cr, flushSpy, store };
}

describe("service-worker · install", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("creates the save-selection context menu and cloud-sync alarm on install", () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    cr.runtime.onInstalled.fire({ reason: "install" });
    expect(cr.contextMenus.create).toHaveBeenCalledWith({
      id: "save-selection",
      title: "Save selection to Mnemonik",
      contexts: ["selection"],
    });
    expect(cr.alarms.create).toHaveBeenCalledWith("cloud-sync-retry", {
      periodInMinutes: 5,
    });
  });
});

describe("service-worker · context_menu_save_selection_emits_message", () => {
  it("dispatches sw:save-selection to the active tab with selectionText + pageUrl", () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    cr.contextMenus.onClicked.fire(
      {
        menuItemId: "save-selection",
        editable: false,
        selectionText: "Verifiable memories ship today.",
        pageUrl: "https://example.com/post/1",
      } as chrome.contextMenus.OnClickData,
      {
        id: 42,
        title: "Example",
        url: "https://example.com/post/1",
      } as chrome.tabs.Tab,
    );

    expect(cr.tabs.sendMessage).toHaveBeenCalledTimes(1);
    const [tabId, message] = cr.tabs.sendMessage.mock.calls[0] ?? [];
    expect(tabId).toBe(42);
    expect(message).toMatchObject({
      type: "sw:save-selection",
      payload: {
        selectionText: "Verifiable memories ship today.",
        pageUrl: "https://example.com/post/1",
      },
    });
  });

  it("ignores empty selections so the tab side never sees a zero-byte payload", () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    cr.contextMenus.onClicked.fire(
      {
        menuItemId: "save-selection",
        editable: false,
        selectionText: "",
        pageUrl: "https://example.com/",
      } as chrome.contextMenus.OnClickData,
      { id: 7 } as chrome.tabs.Tab,
    );

    expect(cr.tabs.sendMessage).not.toHaveBeenCalled();
  });

  it("ignores non-save-selection menu items", () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    cr.contextMenus.onClicked.fire(
      {
        menuItemId: "something-else",
        editable: false,
        selectionText: "x",
        pageUrl: "https://example.com/",
      } as chrome.contextMenus.OnClickData,
      { id: 1 } as chrome.tabs.Tab,
    );

    expect(cr.tabs.sendMessage).not.toHaveBeenCalled();
  });
});

describe("service-worker · commands.onCommand", () => {
  it("dispatches sw:open-recall-overlay to the active tab on Ctrl+Shift+R", () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    cr.commands.onCommand.fire("recall-overlay", {
      id: 9,
    } as chrome.tabs.Tab);

    expect(cr.tabs.sendMessage).toHaveBeenCalledWith(9, {
      type: "sw:open-recall-overlay",
      payload: { trigger: "hotkey" },
    });
  });
});

describe("service-worker · alarm_drains_pending_queue", () => {
  it("calls flushPending with the store when the cloud-sync-retry alarm fires", async () => {
    const { deps, cr, flushSpy, store } = makeDeps();
    await store.enqueue("att-1");
    await store.enqueue("att-2");

    installServiceWorker(deps);
    cr.alarms.onAlarm.fire({
      name: "cloud-sync-retry",
      scheduledTime: Date.now(),
    } as chrome.alarms.Alarm);

    // Poll until the async flush handler chain has settled. Replaces a
    // pair of `await Promise.resolve()` ticks (review nit) — polling
    // doesn't depend on a specific number of microtasks and stays green
    // when T18 swaps in a real async drain step. Plain Promise-based loop
    // works under both vitest and bun:test (vi.waitFor is vitest-only).
    const deadline = Date.now() + 1000;
    while (flushSpy.mock.calls.length === 0 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    expect(flushSpy).toHaveBeenCalledTimes(1);
    const callArg = flushSpy.mock.calls[0]?.[0] as { store: IndexedDbStore };
    expect(callArg.store).toBe(store);
    const pending = await callArg.store.listPending();
    expect(pending.map((p) => p.attestation_id).sort()).toEqual([
      "att-1",
      "att-2",
    ]);
  });

  it("ignores alarms with other names", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);
    cr.alarms.onAlarm.fire({
      name: "some-other-alarm",
      scheduledTime: Date.now(),
    } as chrome.alarms.Alarm);
    await Promise.resolve();
    expect(flushSpy).not.toHaveBeenCalled();
  });

  // Regression for test-reviewer F2 + code-review T18-C-04: a second
  // alarm tick (or `ui:flush-pending` UI gesture) that fires while a
  // previous drain is still awaiting the network MUST no-op rather
  // than race a second drain against the same `pending_uploads` rows.
  it("skips a second alarm while the previous drain is in-flight", async () => {
    const { deps, cr } = makeDeps();
    // Use a deferred so we can hold the first drain open while we
    // fire the second alarm. The guard awaits flushPending; as long
    // as this promise is pending, the guard stays set.
    let release: (v: { attempted: number; flushed: number }) => void = () => {};
    const pending = new Promise<{ attempted: number; flushed: number }>(
      (resolve) => {
        release = resolve;
      },
    );
    const flushSpy = vi.fn().mockReturnValue(pending);
    deps.flushPending =
      flushSpy as unknown as ServiceWorkerDeps["flushPending"];

    installServiceWorker(deps);

    // First alarm — kicks off the drain (pending).
    cr.alarms.onAlarm.fire({
      name: "cloud-sync-retry",
      scheduledTime: Date.now(),
    } as chrome.alarms.Alarm);
    await new Promise((r) => setTimeout(r, 5));
    expect(flushSpy).toHaveBeenCalledTimes(1);

    // Second alarm — guard must skip.
    cr.alarms.onAlarm.fire({
      name: "cloud-sync-retry",
      scheduledTime: Date.now(),
    } as chrome.alarms.Alarm);
    await new Promise((r) => setTimeout(r, 5));
    expect(flushSpy).toHaveBeenCalledTimes(1);

    // Release the first drain so the guard clears.
    release({ attempted: 0, flushed: 0 });
    await new Promise((r) => setTimeout(r, 5));

    // After release, a fresh alarm tick proceeds normally.
    cr.alarms.onAlarm.fire({
      name: "cloud-sync-retry",
      scheduledTime: Date.now(),
    } as chrome.alarms.Alarm);
    await new Promise((r) => setTimeout(r, 5));
    expect(flushSpy).toHaveBeenCalledTimes(2);
  });

  it("ui:flush-pending also respects the in-flight guard (code-review T18-C-04)", async () => {
    const { deps, cr } = makeDeps();
    let release: (v: { attempted: number; flushed: number }) => void = () => {};
    const pending = new Promise<{ attempted: number; flushed: number }>(
      (resolve) => {
        release = resolve;
      },
    );
    const flushSpy = vi.fn().mockReturnValue(pending);
    deps.flushPending =
      flushSpy as unknown as ServiceWorkerDeps["flushPending"];

    installServiceWorker(deps);

    // Kick off via the alarm.
    cr.alarms.onAlarm.fire({
      name: "cloud-sync-retry",
      scheduledTime: Date.now(),
    } as chrome.alarms.Alarm);
    await new Promise((r) => setTimeout(r, 5));
    expect(flushSpy).toHaveBeenCalledTimes(1);

    // Concurrent UI-driven flush — must observe `{ skipped: true }`
    // and NOT call flushPending a second time.
    const captured = await new Promise<unknown>((resolve) => {
      cr.runtime.onMessage.fire(
        { type: "ui:flush-pending" },
        popupSender(),
        (response: unknown) => resolve(response),
      );
    });
    expect(flushSpy).toHaveBeenCalledTimes(1);
    expect(captured).toEqual({ ok: true, result: { skipped: true } });

    // Cleanup.
    release({ attempted: 0, flushed: 0 });
    await new Promise((r) => setTimeout(r, 5));
  });
});

describe("service-worker · runtime.onMessage router", () => {
  it("rejects messages with an unknown shape", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    const responses: unknown[] = [];
    cr.runtime.onMessage.fire(
      { type: "nope" },
      {} as chrome.runtime.MessageSender,
      (response: unknown) => responses.push(response),
    );

    expect(responses[0]).toEqual({ ok: false, error: "unknown-message" });
  });

  it("routes ui:flush-pending to flushPending", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);

    const captured = await new Promise<unknown>((resolve) => {
      cr.runtime.onMessage.fire(
        { type: "ui:flush-pending" },
        popupSender(),
        (response: unknown) => resolve(response),
      );
    });

    expect(flushSpy).toHaveBeenCalledTimes(1);
    expect(captured).toEqual({
      ok: true,
      result: { attempted: 0, flushed: 0 },
    });
  });

  it("rejects ui:* messages with no positive sender identification (T10-N2-01)", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);

    const captured = await new Promise<unknown>((resolve) => {
      cr.runtime.onMessage.fire(
        { type: "ui:flush-pending" },
        {} as chrome.runtime.MessageSender,
        (response: unknown) => resolve(response),
      );
    });

    expect(flushSpy).not.toHaveBeenCalled();
    expect(captured).toEqual({ ok: false, error: "unauthorized-sender" });
  });
});

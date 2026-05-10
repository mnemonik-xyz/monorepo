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

interface ChromeMock {
  runtime: {
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
      onInstalled: makeEvent<[{ reason: string }]>(),
      onStartup: makeEvent<[]>(),
      onMessage: makeEvent(),
    },
    contextMenus: {
      create: vi.fn(),
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
      } as chrome.tabs.Tab
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
      { id: 7 } as chrome.tabs.Tab
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
      { id: 1 } as chrome.tabs.Tab
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

    // Wait a microtask so the async flushPending handler resolves.
    await Promise.resolve();
    await Promise.resolve();

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
});

describe("service-worker · runtime.onMessage router", () => {
  it("rejects messages with an unknown shape", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    const responses: unknown[] = [];
    cr.runtime.onMessage.fire(
      { type: "nope" },
      {} as chrome.runtime.MessageSender,
      (response: unknown) => responses.push(response)
    );

    expect(responses[0]).toEqual({ ok: false, error: "unknown-message" });
  });

  it("routes ui:flush-pending to flushPending", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);

    const captured = await new Promise<unknown>((resolve) => {
      cr.runtime.onMessage.fire(
        { type: "ui:flush-pending" },
        {} as chrome.runtime.MessageSender,
        (response: unknown) => resolve(response)
      );
    });

    expect(flushSpy).toHaveBeenCalledTimes(1);
    expect(captured).toEqual({
      ok: true,
      result: { attempted: 0, flushed: 0 },
    });
  });
});

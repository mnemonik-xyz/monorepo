// T25b unit tests for the install-time embedder prewarm path. Three
// anchored behaviours:
//
//   1. `onInstalled` with `reason: "install"` → `prewarmEmbedder` is
//      called exactly once. Mirrors the production flow that kicks the
//      ORT WASM + ONNX download into the SW realm before first capture.
//
//   2. `onInstalled` with `reason: "browser_update"` → `prewarmEmbedder`
//      is NOT called. Chrome's own browser updates don't trigger an
//      extension install/update reason, so the SW must skip prewarm to
//      avoid burning ~35MB on every browser version bump.
//
//   3. Prewarm rejection (slow network, missing WebAssembly) → install
//      handler does NOT throw and downstream listeners are still wired
//      (e.g. context-menu registration completes).
//
// The chrome.* mock matches `service-worker.test.ts` so the wiring
// surface stays consistent.

import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  installServiceWorker,
  type ServiceWorkerDeps,
} from "../../../src/background/service-worker.js";
import { IndexedDbStore } from "../../../src/runtime/store/indexeddb.js";

type Listener<Args extends unknown[]> = (...args: Args) => void;

function makeEvent<Args extends unknown[]>(): {
  listeners: Listener<Args>[];
  addListener: (cb: Listener<Args>) => void;
  fire: (...args: Args) => void;
} {
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

const TEST_EXT_ID = "mnemonik-test-extension-id";

function makeChromeMock(): {
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
} {
  return {
    runtime: {
      id: TEST_EXT_ID,
      onInstalled: makeEvent<[{ reason: string }]>(),
      onStartup: makeEvent<[]>(),
      onMessage: makeEvent(),
    },
    contextMenus: {
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

function uniqueDbName(suffix: string): string {
  return `mnemonik-prewarm-test-${suffix}-${Math.random().toString(36).slice(2)}`;
}

function makeDeps(prewarm: () => Promise<void>): {
  deps: ServiceWorkerDeps;
  cr: ReturnType<typeof makeChromeMock>;
} {
  const cr = makeChromeMock();
  const store = new IndexedDbStore({ dbName: uniqueDbName("store") });
  const flushSpy = vi.fn().mockResolvedValue({ attempted: 0, flushed: 0 });
  const deps: ServiceWorkerDeps = {
    chrome: cr as unknown as typeof chrome,
    storeFactory: () => store,
    flushPending: flushSpy as unknown as ServiceWorkerDeps["flushPending"],
    now: () => "2026-05-11T00:00:00.000Z",
    prewarmEmbedder: prewarm,
  };
  return { deps, cr };
}

describe("service-worker · onInstalled prewarms the embedder", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls prewarmEmbedder exactly once for reason='install'", async () => {
    const prewarm = vi.fn().mockResolvedValue(undefined);
    const { deps, cr } = makeDeps(prewarm);
    installServiceWorker(deps);

    cr.runtime.onInstalled.fire({ reason: "install" });

    // Prewarm runs inside a void async IIFE — await a microtask hop so
    // the spy observes the call.
    const deadline = Date.now() + 500;
    while (prewarm.mock.calls.length === 0 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 2));
    }
    expect(prewarm).toHaveBeenCalledTimes(1);
  });

  it("calls prewarmEmbedder exactly once for reason='update'", async () => {
    const prewarm = vi.fn().mockResolvedValue(undefined);
    const { deps, cr } = makeDeps(prewarm);
    installServiceWorker(deps);

    cr.runtime.onInstalled.fire({ reason: "update" });

    const deadline = Date.now() + 500;
    while (prewarm.mock.calls.length === 0 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 2));
    }
    expect(prewarm).toHaveBeenCalledTimes(1);
  });

  it("does NOT call prewarmEmbedder for reason='browser_update'", async () => {
    const prewarm = vi.fn().mockResolvedValue(undefined);
    const { deps, cr } = makeDeps(prewarm);
    installServiceWorker(deps);

    cr.runtime.onInstalled.fire({ reason: "browser_update" });

    // Give any (incorrect) async prewarm a chance to land.
    await new Promise((resolve) => setTimeout(resolve, 25));
    expect(prewarm).not.toHaveBeenCalled();
  });

  it("does NOT call prewarmEmbedder for reason='chrome_update'", async () => {
    const prewarm = vi.fn().mockResolvedValue(undefined);
    const { deps, cr } = makeDeps(prewarm);
    installServiceWorker(deps);

    cr.runtime.onInstalled.fire({ reason: "chrome_update" });

    await new Promise((resolve) => setTimeout(resolve, 25));
    expect(prewarm).not.toHaveBeenCalled();
  });

  it("does NOT throw or break install when prewarmEmbedder rejects", async () => {
    const prewarm = vi
      .fn()
      .mockRejectedValue(new Error("simulated cold-init failure"));
    const { deps, cr } = makeDeps(prewarm);
    installServiceWorker(deps);

    // Firing the listener must not bubble the prewarm rejection.
    expect(() =>
      cr.runtime.onInstalled.fire({ reason: "install" }),
    ).not.toThrow();

    // Wait long enough for the void async IIFE to settle so the
    // unhandled-rejection guard inside the listener catches the error.
    await new Promise((resolve) => setTimeout(resolve, 25));

    // Sibling install side-effects still happened — context-menu
    // creation and alarm registration are the canary.
    expect(cr.contextMenus.create).toHaveBeenCalledWith({
      id: "save-selection",
      title: "Save selection to Mnemonik",
      contexts: ["selection"],
    });
    expect(cr.alarms.create).toHaveBeenCalledWith("cloud-sync-retry", {
      periodInMinutes: 5,
    });
    expect(prewarm).toHaveBeenCalledTimes(1);
  });

  it("skips prewarm entirely when no prewarmEmbedder dep is provided", async () => {
    const cr = makeChromeMock();
    const store = new IndexedDbStore({ dbName: uniqueDbName("store") });
    const deps: ServiceWorkerDeps = {
      chrome: cr as unknown as typeof chrome,
      storeFactory: () => store,
      flushPending: vi.fn().mockResolvedValue({
        attempted: 0,
        flushed: 0,
      }) as unknown as ServiceWorkerDeps["flushPending"],
      now: () => "2026-05-11T00:00:00.000Z",
    };
    installServiceWorker(deps);
    expect(() =>
      cr.runtime.onInstalled.fire({ reason: "install" }),
    ).not.toThrow();
    await new Promise((resolve) => setTimeout(resolve, 10));
    // Context-menu still registered.
    expect(cr.contextMenus.create).toHaveBeenCalled();
  });
});

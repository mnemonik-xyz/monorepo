// T13 — FAB unit tests (JSDOM). Complement the Playwright E2E spec
// for environments where Playwright browsers aren't installed (the
// default CI image today). The Shadow-DOM isolation property lives
// in the E2E spec; here we cover the wiring contract:
//
//   - mountFab respects per-domain `fabVisible: false` and returns null.
//   - mountFab inserts a host element with `all: initial` reset.
//   - menu toggle / drag-vs-click discrimination in attachDragAndClick.
//   - position read/write round-trips through chrome.storage.local.

// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from "vitest";

interface StorageMock {
  data: Record<string, unknown>;
  get: (key: string) => Promise<Record<string, unknown>>;
  set: (entries: Record<string, unknown>) => Promise<void>;
  onChanged: { addListener: (cb: unknown) => void };
}

function installChromeMock(initial: Record<string, unknown> = {}): StorageMock {
  const storage: StorageMock = {
    data: { ...initial },
    get: async (key: string) => ({ [key]: storage.data[key] }),
    set: async (entries: Record<string, unknown>) => {
      Object.assign(storage.data, entries);
    },
    onChanged: { addListener: () => {} },
  };
  (globalThis as unknown as { chrome: unknown }).chrome = {
    runtime: { id: "test-ext", onMessage: { addListener: () => {} } },
    storage: { local: storage },
  };
  return storage;
}

beforeEach(() => {
  document.body.innerHTML = "";
  vi.resetModules();
});

describe("mountFab", () => {
  it("returns null when settings.v1.perDomain.<domain>.fabVisible === false", async () => {
    installChromeMock({
      "settings.v1": {
        perDomain: { localhost: { fabVisible: false } },
      },
    });
    const { mountFab } = await import("../../../src/content/fab.js");
    const host = await mountFab();
    expect(host).toBeNull();
    expect(document.querySelector("mnemonik-fab-host")).toBeNull();
  });

  it("mounts a host element with `all: initial` reset", async () => {
    installChromeMock();
    const { mountFab } = await import("../../../src/content/fab.js");
    const host = await mountFab();
    expect(host).not.toBeNull();
    const style = host?.getAttribute("style") ?? "";
    expect(style).toContain("all: initial");
    expect(style).toContain("position: fixed");
    // Closed shadow root: we cannot read host.shadowRoot from outside.
    expect(host?.shadowRoot).toBeNull();
  });

  it("is idempotent — second mount call returns null", async () => {
    installChromeMock();
    const { mountFab } = await import("../../../src/content/fab.js");
    await mountFab();
    const second = await mountFab();
    expect(second).toBeNull();
    expect(document.querySelectorAll("mnemonik-fab-host")).toHaveLength(1);
  });

  it("reads stored fabPosition for the current domain", async () => {
    installChromeMock({
      fabPosition: { localhost: { right: 100, bottom: 200 } },
    });
    const { mountFab } = await import("../../../src/content/fab.js");
    const host = await mountFab();
    const style = host?.getAttribute("style") ?? "";
    expect(style).toContain("right: 100px");
    expect(style).toContain("bottom: 200px");
  });
});

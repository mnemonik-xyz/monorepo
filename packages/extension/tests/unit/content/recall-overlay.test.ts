// T13 — Recall overlay unit tests (JSDOM). Covers the runtime
// contract:
//
//   - mountOverlay creates a host with `all: initial` + closed
//     shadow root.
//   - second mountOverlay call is a no-op (single overlay at a time).
//   - sw:open-recall-overlay message triggers mount.
//   - ESC key destroys the overlay.

// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from "vitest";

interface ChromeRuntimeMock {
  id: string;
  onMessage: {
    listeners: Array<(raw: unknown) => void>;
    addListener: (cb: (raw: unknown) => void) => void;
  };
  sendMessage: (msg: unknown) => Promise<unknown>;
}

function installChrome(): ChromeRuntimeMock {
  const runtime: ChromeRuntimeMock = {
    id: "test-ext",
    onMessage: {
      listeners: [],
      addListener: (cb) => {
        runtime.onMessage.listeners.push(cb);
      },
    },
    sendMessage: async () => ({ ok: true, results: [] }),
  };
  (globalThis as unknown as { chrome: unknown }).chrome = {
    runtime,
    storage: { local: { get: async () => ({}), set: async () => {} } },
  };
  return runtime;
}

beforeEach(() => {
  document.body.innerHTML = "";
  vi.resetModules();
});

describe("mountOverlay", () => {
  it("mounts a closed-shadow host with `all: initial` reset", async () => {
    installChrome();
    const { mountOverlay } =
      await import("../../../src/content/recall-overlay.js");
    const host = mountOverlay();
    expect(host).not.toBeNull();
    expect(host?.getAttribute("style")).toContain("all: initial");
    expect(host?.shadowRoot).toBeNull();
  });

  it("is idempotent — second mount call returns the same host", async () => {
    installChrome();
    const { mountOverlay } =
      await import("../../../src/content/recall-overlay.js");
    const first = mountOverlay();
    const second = mountOverlay();
    expect(second).toBe(first);
    expect(document.querySelectorAll("mnemonik-overlay-host")).toHaveLength(1);
  });

  it("ESC key destroys the overlay", async () => {
    installChrome();
    const { mountOverlay } =
      await import("../../../src/content/recall-overlay.js");
    mountOverlay();
    expect(document.querySelector("mnemonik-overlay-host")).not.toBeNull();
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    expect(document.querySelector("mnemonik-overlay-host")).toBeNull();
  });

  it("sw:open-recall-overlay message triggers mount", async () => {
    const runtime = installChrome();
    await import("../../../src/content/recall-overlay.js");
    expect(runtime.onMessage.listeners.length).toBeGreaterThan(0);
    const cb = runtime.onMessage.listeners[0];
    if (!cb) throw new Error("missing listener");
    cb({ type: "sw:open-recall-overlay", payload: { trigger: "hotkey" } });
    expect(document.querySelector("mnemonik-overlay-host")).not.toBeNull();
  });
});

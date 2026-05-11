// Shared vitest setup. Loads `@testing-library/jest-dom` matchers when
// jsdom is the active environment (the component tests under
// tests/component/**) and stays a no-op otherwise so node-environment
// unit tests aren't slowed down. Also exposes a minimal `chrome.*`
// stub so popup code that calls `chrome.tabs.query` / `chrome.storage`
// doesn't blow up under jsdom.

import { afterEach } from "vitest";

if (typeof window !== "undefined") {
  // jest-dom matchers — only meaningful with a DOM.
  await import("@testing-library/jest-dom/vitest");
  const rtl = await import("@testing-library/react");
  afterEach(() => rtl.cleanup());

  // Minimal chrome.* stub. Individual tests override the per-method
  // implementations they care about; the defaults keep `await
  // chrome.*` calls from rejecting at import time.
  const noop = async (): Promise<unknown> => undefined;
  const chromeStub = {
    tabs: {
      query: async () => [] as chrome.tabs.Tab[],
      sendMessage: noop,
    },
    storage: {
      local: {
        get: async () => ({}),
        set: noop,
      },
      sync: {
        get: async () => ({}),
        set: noop,
      },
    },
    runtime: {
      sendMessage: noop,
      openOptionsPage: noop,
    },
  } as unknown as typeof chrome;
  (globalThis as { chrome?: typeof chrome }).chrome = chromeStub;
}

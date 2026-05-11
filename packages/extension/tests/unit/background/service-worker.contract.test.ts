// T23 SW-router contract tests.
//
// The audit found two bridge-level bugs that survived unit-level tests
// (each side mocked the boundary):
//
//   - AUD-C-05 / B5 — `tab:fab-*` messages were dropped by `parseMsg`
//     because the variants were missing from the `Msg` union. The
//     popup/FAB sender side and the SW receiver side each shipped
//     green tests against their own mocked surfaces; nothing
//     exercised the real `parseMsg` + real router together.
//
//   - AUD-C-04 / B4 — `ui:recall` resolved to a `{ deferred: "recall" }`
//     stub in the SW, so the recall overlay received zero results on
//     every query.
//
// This file drives the real `installServiceWorker` + real `parseMsg`
// against synthetic but realistic message senders. We assert:
//
//   1. Every inbound `Msg` variant the router accepts produces the
//      contracted response shape (and fires the right side effect).
//   2. The sender-authorisation perimeter rejects:
//        - `ui:*` messages whose sender carries a `tab` field
//          (popup-realm messages never have one);
//        - `tab:*` messages from a non-allowed origin (defends the
//          AUD-S "T2 hostile content script / extension" threat).
//        - `sw:*` messages received inbound (the SW only emits them).
//   3. Unknown / malformed payloads round-trip the documented error
//      response `{ ok: false, error: "unknown-message" }` rather than
//      crashing the SW.
//
// Mocking discipline: the chrome.* surface is hand-rolled rather than
// pulled from a fixture so the test is a single-file readable
// contract. `IndexedDbStore` and `flushPending` are passed through
// `installServiceWorker`'s `deps` parameter — the SW boot path is
// pure functions over `deps`, so we never patch the module-scope
// `chrome` global.

import "fake-indexeddb/auto";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  installServiceWorker,
  type ServiceWorkerDeps,
} from "../../../src/background/service-worker.js";
import { IndexedDbStore } from "../../../src/runtime/store/indexeddb.js";
import { __setEmbedderForTesting } from "../../../src/background/recall-embedder.js";
import type { Embedder } from "../../../src/runtime/embed/types.js";

type Listener<Args extends unknown[]> = (...args: Args) => void;

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

const TEST_EXT_ID = "mnemonik-test-ext-id-T23";

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
  storage: {
    local: {
      set: ReturnType<typeof vi.fn>;
      get: ReturnType<typeof vi.fn>;
      remove: ReturnType<typeof vi.fn>;
    };
  };
  action: {
    openPopup: ReturnType<typeof vi.fn>;
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
      create: vi.fn(),
      removeAll: vi.fn((cb?: unknown): unknown => {
        if (typeof cb === "function") (cb as () => void)();
        return undefined;
      }),
      onClicked: makeEvent(),
    },
    commands: { onCommand: makeEvent() },
    alarms: { create: vi.fn(), onAlarm: makeEvent() },
    tabs: { sendMessage: vi.fn().mockResolvedValue(undefined) },
    storage: {
      local: {
        set: vi.fn().mockResolvedValue(undefined),
        get: vi.fn().mockResolvedValue({}),
        remove: vi.fn().mockResolvedValue(undefined),
      },
    },
    action: { openPopup: vi.fn() },
  };
}

function uniqueDbName(suffix: string): string {
  return `mnemonik-sw-contract-${suffix}-${Math.random().toString(36).slice(2)}`;
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

/** Sender that passes `ui:*` authorisation (popup realm). */
function popupSender(): chrome.runtime.MessageSender {
  return {
    id: TEST_EXT_ID,
    url: `chrome-extension://${TEST_EXT_ID}/popup/index.html`,
  } as chrome.runtime.MessageSender;
}

/** Sender that passes `tab:*` authorisation (content-script realm on
 *  one of the AI-chat origins). */
function tabSender(
  url = "https://chatgpt.com/c/abc",
  id = 7,
): chrome.runtime.MessageSender {
  return {
    id: TEST_EXT_ID,
    tab: { id, url } as chrome.tabs.Tab,
    url,
  } as chrome.runtime.MessageSender;
}

/** Fire one message at the router and await the response sync’d via
 *  the `sendResponse` callback. Matches the MV3 `runtime.onMessage`
 *  contract: handlers either resolve synchronously or return `true`
 *  and call `sendResponse` later. */
function fireAndCapture(
  cr: ChromeMock,
  msg: unknown,
  sender: chrome.runtime.MessageSender,
): Promise<unknown> {
  return new Promise<unknown>((resolve) => {
    cr.runtime.onMessage.fire(msg, sender, (response: unknown) =>
      resolve(response),
    );
  });
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  // The SW-side recall handler imports `recall-embedder.js`, which
  // caches its Embedder instance at module scope. Reset it on every
  // teardown so a test that stubbed the embedder via
  // `__setEmbedderForTesting(...)` can't bleed into a later test
  // file's recall behaviour. Cheap (no-op when the cache is already
  // null).
  __setEmbedderForTesting(null);
});

// ── inbound message contract per variant ────────────────────────────

describe("service-worker contract · accepted inbound variants", () => {
  it("ui:flush-pending → flushPending called, returns { ok:true, result }", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      { type: "ui:flush-pending" },
      popupSender(),
    );
    expect(flushSpy).toHaveBeenCalledTimes(1);
    expect(out).toEqual({
      ok: true,
      result: { attempted: 0, flushed: 0 },
    });
  });

  it("ui:sign-memory → returns { deferred: 'sign-memory' } envelope", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "ui:sign-memory",
        payload: {
          content: "remember this",
          tags: ["t"],
        },
      },
      popupSender(),
    );
    // T11 / T18 will eventually plug a concrete runtime call into this
    // branch; the contract this test locks is the envelope shape: a
    // success response that carries the deferred token through to the
    // popup so the popup can correlate the eventual sign-callback.
    expect(out).toEqual({
      ok: true,
      result: { deferred: "sign-memory" },
    });
  });

  it("ui:recall → calls the SW embedder + IndexedDbStore.search (B4)", async () => {
    // Audit B4 / AUD-C-04 anchor: this used to return
    // `{ deferred: "recall" }` so the overlay saw zero results on
    // every query. The contract: ui:recall MUST embed the query and
    // hand off to IndexedDbStore.search before responding.
    const { deps, cr, store } = makeDeps();

    const embedderStub: Embedder = {
      embed: vi.fn().mockResolvedValue(new Float32Array(384)),
    } as unknown as Embedder;
    __setEmbedderForTesting(embedderStub);

    const searchSpy = vi.spyOn(store, "search").mockResolvedValue([
      {
        attestation_id: "att-1",
        owner_pubkey: "z6Mk",
        score: 0.9,
        tags: ["framework"],
        created_at: "2026-05-11T00:00:00.000Z",
        source: { kind: "user-text" },
      },
    ] as unknown as Awaited<ReturnType<typeof store.search>>);

    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "ui:recall",
        payload: {
          query: "react server components",
          ownerPubkey: "z6Mk",
          limit: 5,
          tags: ["framework"],
        },
      },
      popupSender(),
    );

    expect(embedderStub.embed).toHaveBeenCalledWith("react server components");
    expect(searchSpy).toHaveBeenCalledTimes(1);
    expect(searchSpy.mock.calls[0]?.[1]).toBe("z6Mk");
    expect(searchSpy.mock.calls[0]?.[2]).toBe(5);
    expect(searchSpy.mock.calls[0]?.[3]).toEqual({ tags: ["framework"] });

    // T23-C-N5 round-1 review: prefer toEqual over toMatchObject so
    // a future refactor that adds extra fields to the response
    // (e.g. a debug envelope) trips this assertion rather than
    // silently passing. The exact shape is the public contract the
    // recall overlay parses; keep it pinned.
    expect(out).toEqual({
      ok: true,
      result: {
        ok: true,
        results: [
          {
            attestation_id: "att-1",
            owner_pubkey: "z6Mk",
            score: 0.9,
            tags: ["framework"],
            created_at: "2026-05-11T00:00:00.000Z",
            source: { kind: "user-text" },
          },
        ],
      },
    });
    // Module-scoped embedder cache is cleared by the `afterEach`
    // hook so other test files start with a clean slate.
  });

  it("tab:fab-save-chat → persists pending_fab_action.v1 + tries openPopup (B5)", async () => {
    // Audit B5 / AUD-C-05 anchor: dropping these variants from
    // parseMsg silently killed every FAB menu action. The contract:
    // tab:fab-save-chat MUST stash the intent + best-effort open the
    // popup. Failure of openPopup is non-fatal — the intent persists
    // until the user opens the popup manually.
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    const out = await fireAndCapture(
      cr,
      {
        type: "tab:fab-save-chat",
        payload: { pageUrl: "https://chatgpt.com/c/abc" },
      },
      tabSender("https://chatgpt.com/c/abc"),
    );

    expect(cr.storage.local.set).toHaveBeenCalledTimes(1);
    const setArg = cr.storage.local.set.mock.calls[0]?.[0] as {
      "pending_fab_action.v1": Record<string, unknown>;
    };
    expect(setArg["pending_fab_action.v1"]).toMatchObject({
      kind: "save-chat",
      pageUrl: "https://chatgpt.com/c/abc",
    });
    expect(cr.action.openPopup).toHaveBeenCalledTimes(1);
    expect(out).toEqual({
      ok: true,
      result: { acknowledged: true },
    });
  });

  it("tab:fab-save-selection → persists selectionText + pageUrl in pending_fab_action.v1", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);

    const out = await fireAndCapture(
      cr,
      {
        type: "tab:fab-save-selection",
        payload: {
          selectionText: "verifiable memories",
          pageUrl: "https://claude.ai/chat/x",
        },
      },
      tabSender("https://claude.ai/chat/x"),
    );

    expect(cr.storage.local.set).toHaveBeenCalledTimes(1);
    const setArg = cr.storage.local.set.mock.calls[0]?.[0] as {
      "pending_fab_action.v1": Record<string, unknown>;
    };
    expect(setArg["pending_fab_action.v1"]).toMatchObject({
      kind: "save-selection",
      selectionText: "verifiable memories",
      pageUrl: "https://claude.ai/chat/x",
    });
    expect(out).toEqual({
      ok: true,
      result: { acknowledged: true },
    });
  });

  it("tab:fab-open-popup → calls chrome.action.openPopup", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      { type: "tab:fab-open-popup" },
      tabSender("https://gemini.google.com/app/abc"),
    );
    expect(cr.action.openPopup).toHaveBeenCalledTimes(1);
    expect(out).toEqual({
      ok: true,
      result: { acknowledged: true },
    });
  });

  it("tab:capture-candidate → acknowledged (no side effects)", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "tab:capture-candidate",
        payload: {
          platform: "claude",
          url: "https://claude.ai/chat/abc",
          content: "assistant turn",
        },
      },
      tabSender("https://claude.ai/chat/abc"),
    );
    expect(out).toEqual({
      ok: true,
      result: { acknowledged: true },
    });
    expect(cr.storage.local.set).not.toHaveBeenCalled();
  });
});

// ── sender-authorisation perimeter (T2 hostile-extension threat) ────

describe("service-worker contract · sender authorisation", () => {
  it("rejects ui:* messages whose sender carries a `tab` (popup never does)", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      { type: "ui:flush-pending" },
      // hostile: a content script masquerading as the popup
      {
        id: TEST_EXT_ID,
        tab: { id: 1, url: "https://chatgpt.com/c/abc" } as chrome.tabs.Tab,
        url: `chrome-extension://${TEST_EXT_ID}/popup/index.html`,
      } as chrome.runtime.MessageSender,
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
    expect(flushSpy).not.toHaveBeenCalled();
  });

  it("rejects ui:* messages with no positive sender identification", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      { type: "ui:flush-pending" },
      {} as chrome.runtime.MessageSender,
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
    expect(flushSpy).not.toHaveBeenCalled();
  });

  it("rejects ui:* messages from a foreign extension id", async () => {
    const { deps, cr, flushSpy } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(cr, { type: "ui:flush-pending" }, {
      id: "some-other-extension-id",
      url: "chrome-extension://some-other-extension-id/popup.html",
    } as chrome.runtime.MessageSender);
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
    expect(flushSpy).not.toHaveBeenCalled();
  });

  it("ui:recall is accepted from a content-script sender on an allowed origin (PR134-BLK-2 / BUG-01)", async () => {
    // The recall overlay (Ctrl+Shift+R) runs in the content-script
    // realm and fires `ui:recall` with `sender.tab` set to the host
    // chat tab. Previously this was rejected with "unauthorized-
    // sender" because the `ui:*` branch required `sender.tab ===
    // undefined`. The fix splits `ui:recall` into its own branch
    // that accepts EITHER popup origin (no tab) OR content-script
    // origin on an allowed AI-chat host.
    const { deps, cr, store } = makeDeps();
    const embedderStub: Embedder = {
      embed: vi.fn().mockResolvedValue(new Float32Array(384)),
    } as unknown as Embedder;
    __setEmbedderForTesting(embedderStub);
    vi.spyOn(store, "search").mockResolvedValue([]);
    installServiceWorker(deps);

    const out = await fireAndCapture(
      cr,
      {
        type: "ui:recall",
        payload: { query: "hello", ownerPubkey: "z6Mk", limit: 3 },
      },
      tabSender("https://chatgpt.com/c/xyz"),
    );
    expect(out).toMatchObject({ ok: true });
    expect(embedderStub.embed).toHaveBeenCalledWith("hello");
  });

  it("ui:recall from a non-allowed tab origin is rejected", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "ui:recall",
        payload: { query: "hello", ownerPubkey: "z6Mk", limit: 3 },
      },
      tabSender("https://evil.example.com/"),
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
  });

  it("tab:fab-save-chat from a non-allowed origin is rejected (tab_fab_messages_from_non_allowed_origin_rejected)", async () => {
    // TDD anchor from task 23: drives the T2 hostile-extension /
    // hostile-content-script threat. A content script on
    // https://evil.example.com — outside our host_permissions —
    // MUST NOT be able to plant a `pending_fab_action.v1` intent.
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "tab:fab-save-chat",
        payload: { pageUrl: "https://evil.example.com/" },
      },
      tabSender("https://evil.example.com/", 99),
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
    expect(cr.storage.local.set).not.toHaveBeenCalled();
    expect(cr.action.openPopup).not.toHaveBeenCalled();
  });

  it("tab:capture-candidate from a non-allowed origin is rejected", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "tab:capture-candidate",
        payload: {
          platform: "claude",
          url: "https://evil.example.com/x",
          content: "assistant turn",
        },
      },
      tabSender("https://evil.example.com/x", 42),
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
  });

  it("tab:* origin allowlist uses prefix-then-slash match (no startsWith bypass)", async () => {
    // Defends against a sub-domain confusion vector:
    // `https://chatgpt.com.evil.example` MUST NOT pass the allow-
    // list. The check appends `/` to the allowed origin so only the
    // path-boundary form matches.
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "tab:fab-save-chat",
        payload: { pageUrl: "https://chatgpt.com.evil.example/c/abc" },
      },
      tabSender("https://chatgpt.com.evil.example/c/abc", 88),
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
  });

  it("sw:* messages received inbound are rejected (SW only emits them)", async () => {
    // Defence-in-depth: even if a sender on an allowed origin tries
    // to forge an `sw:open-recall-overlay` toward the SW (with a
    // valid trigger), the router rejects because the discriminant
    // is reserved for outbound.
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "sw:open-recall-overlay",
        payload: { trigger: "popup" },
      },
      tabSender("https://chatgpt.com/c/abc"),
    );
    expect(out).toEqual({ ok: false, error: "unauthorized-sender" });
  });

  it("tab:fab-* messages from each AI-chat origin in the allowlist are accepted", async () => {
    // Forward-compat sanity: ensures the allowlist contains every
    // host_permission we declare. If a future PR removes one of
    // these origins from `manifest.json` without updating
    // `ALLOWED_TAB_ORIGINS`, the corresponding case here flips and
    // the test fails — surfacing the drift.
    for (const origin of [
      "https://chatgpt.com",
      "https://claude.ai",
      "https://gemini.google.com",
    ]) {
      const { deps, cr } = makeDeps();
      installServiceWorker(deps);
      const out = await fireAndCapture(
        cr,
        { type: "tab:fab-open-popup" },
        tabSender(`${origin}/some/path`),
      );
      expect(out).toEqual({
        ok: true,
        result: { acknowledged: true },
      });
    }
  });
});

// ── parser perimeter (unknown / malformed) ──────────────────────────

// Note on coverage scope: Symbols / functions / Maps / Sets are
// rejected by chrome.runtime's structured-clone before reaching this
// router (the IPC boundary throws DataCloneError), so they are not
// drilled here. The parser-layer guard against those primitives is
// covered exhaustively in `tests/unit/messages.contract.test.ts`
// (`returns null on null / undefined / primitives` and the array /
// no-`type`-field cases). This file's perimeter focuses on inputs
// the IPC will actually surface to the router.

describe("service-worker contract · unknown / malformed inputs", () => {
  it("returns { ok:false, error:'unknown-message' } for an unknown discriminant", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      { type: "ui:never-existed" },
      popupSender(),
    );
    expect(out).toEqual({ ok: false, error: "unknown-message" });
  });

  it("returns { ok:false, error:'unknown-message' } for a malformed ui:recall payload + does NOT touch the embedder/store (T23-T-N3)", async () => {
    // Defence-in-depth: hostile input MUST be rejected at parseMsg
    // before reaching the recall handler. If the parser drift ever
    // lets a malformed payload through, the embedder cold-start
    // path is a ~25MB model fetch — an unrejected hostile recall
    // would be an easy resource-exhaustion vector. This test pins
    // BOTH the response envelope AND the side-effect quiescence.
    const { deps, cr, store } = makeDeps();
    const embedderStub: Embedder = {
      embed: vi.fn().mockResolvedValue(new Float32Array(384)),
    } as unknown as Embedder;
    __setEmbedderForTesting(embedderStub);
    const searchSpy = vi.spyOn(store, "search").mockResolvedValue([]);

    installServiceWorker(deps);
    const out = await fireAndCapture(
      cr,
      {
        type: "ui:recall",
        payload: {
          query: "q",
          ownerPubkey: "z6Mk",
          // limit out-of-range — parseMsg rejects before authz runs.
          limit: 9999,
        },
      },
      popupSender(),
    );
    expect(out).toEqual({ ok: false, error: "unknown-message" });
    expect(embedderStub.embed).not.toHaveBeenCalled();
    expect(searchSpy).not.toHaveBeenCalled();
  });

  it("returns { ok:false, error:'unknown-message' } for non-object input", async () => {
    const { deps, cr } = makeDeps();
    installServiceWorker(deps);
    const out = await fireAndCapture(cr, "ui:flush-pending", popupSender());
    expect(out).toEqual({ ok: false, error: "unknown-message" });
  });
});

import { afterEach, describe, expect, it } from "vitest";
import {
  __setAdaptersForTesting,
  listAdapters,
  registerAdapter,
  selectAdapter,
} from "../../../src/runtime/chat/registry.js";
import type { ChatAdapter } from "../../../src/runtime/chat/types.js";

// Stub adapter helper — registry is meant to be opaque to the framework,
// so tests construct minimal adapters and inject them via the test-only
// setter that the production array stays empty in T06.
function stub(
  platform: string,
  hostPattern: RegExp,
): ChatAdapter {
  return {
    hostPattern,
    platform,
    extractConversation: () => [],
    findInputBox: () => null,
    getChatId: () => null,
  };
}

afterEach(() => {
  __setAdaptersForTesting([]);
});

describe("registry · selectAdapter", () => {
  // T06 TDD anchor (D13): unknown hosts must return null so the popup
  // falls back to generic page-selection capture (D8) — never mis-route
  // to a wrong adapter.
  it("unknown_host_returns_null", () => {
    __setAdaptersForTesting([
      stub("chatgpt", /^chatgpt\.com\//),
      stub("claude", /^claude\.ai\//),
    ]);
    expect(selectAdapter("https://example.com")).toBeNull();
  });

  it("matches the first adapter whose hostPattern hits hostname+pathname", () => {
    const chatgpt = stub("chatgpt", /^chatgpt\.com\//);
    const claude = stub("claude", /^claude\.ai\//);
    __setAdaptersForTesting([chatgpt, claude]);

    expect(selectAdapter("https://chatgpt.com/c/abc123")).toBe(chatgpt);
    expect(selectAdapter("https://claude.ai/chat/xyz")).toBe(claude);
  });

  it("respects registration order on overlap (first wins)", () => {
    const generic = stub("generic", /^chat\./);
    const specific = stub("chatgpt", /^chat\.openai\.com\//);
    // `generic` registered first should take precedence per D10's
    // explicit array-order tie-break, even though `specific` is a
    // narrower pattern.
    __setAdaptersForTesting([generic, specific]);
    expect(selectAdapter("https://chat.openai.com/")).toBe(generic);

    __setAdaptersForTesting([specific, generic]);
    expect(selectAdapter("https://chat.openai.com/")).toBe(specific);
  });

  it("matches against hostname + pathname (not the full URL)", () => {
    const adapter = stub("chatgpt", /^chatgpt\.com\/c\//);
    __setAdaptersForTesting([adapter]);
    // Query string + hash must not affect matching.
    expect(
      selectAdapter("https://chatgpt.com/c/abc?ref=foo#section"),
    ).toBe(adapter);
    // Wrong path: no match, even though the host is right.
    expect(selectAdapter("https://chatgpt.com/about")).toBeNull();
  });

  it("returns null on unparseable URLs without throwing", () => {
    __setAdaptersForTesting([stub("chatgpt", /^chatgpt\.com\//)]);
    expect(selectAdapter("not a url")).toBeNull();
    expect(selectAdapter("")).toBeNull();
  });

  it("ships empty in T06 — concrete adapters land in T07–T09", () => {
    // Sanity check that production state is untouched by tests.
    __setAdaptersForTesting([]);
    expect(listAdapters()).toEqual([]);
    expect(selectAdapter("https://chatgpt.com/")).toBeNull();
  });
});

describe("registry · registerAdapter", () => {
  it("appends adapters in call order", () => {
    const a = stub("chatgpt", /^chatgpt\.com\//);
    const b = stub("claude", /^claude\.ai\//);
    registerAdapter(a);
    registerAdapter(b);
    expect(listAdapters()).toEqual([a, b]);
  });

  it("is idempotent for the same instance — supports double-import + HMR", () => {
    const a = stub("chatgpt", /^chatgpt\.com\//);
    registerAdapter(a);
    registerAdapter(a);
    expect(listAdapters()).toEqual([a]);
  });

  it("matches the registered adapter via selectAdapter", () => {
    const adapter = stub("gemini", /^gemini\.google\.com\//);
    registerAdapter(adapter);
    expect(selectAdapter("https://gemini.google.com/app/abc")).toBe(adapter);
  });
});

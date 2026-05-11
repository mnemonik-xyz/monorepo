// Tests for the FAB message variants added by audit B5 / AUD-C-05.
// `parseMsg` previously rejected `tab:fab-*` types so every FAB menu
// action was a silent no-op. These tests assert the new per-variant
// guards accept well-formed payloads and reject malformed ones.

import { describe, it, expect } from "vitest";
import { parseMsg } from "../../src/messages.js";

describe("parseMsg — tab:fab-save-chat", () => {
  it("accepts a payload-less message", () => {
    const out = parseMsg({ type: "tab:fab-save-chat" });
    expect(out).toEqual({ type: "tab:fab-save-chat" });
  });

  it("accepts an optional pageUrl payload", () => {
    const out = parseMsg({
      type: "tab:fab-save-chat",
      payload: { pageUrl: "https://chatgpt.com/c/123" },
    });
    expect(out).toMatchObject({
      type: "tab:fab-save-chat",
      payload: { pageUrl: "https://chatgpt.com/c/123" },
    });
  });

  it("rejects a non-string pageUrl", () => {
    expect(
      parseMsg({ type: "tab:fab-save-chat", payload: { pageUrl: 42 } }),
    ).toBeNull();
  });
});

describe("parseMsg — tab:fab-save-selection", () => {
  it("accepts {selectionText, pageUrl}", () => {
    const out = parseMsg({
      type: "tab:fab-save-selection",
      payload: { selectionText: "hello", pageUrl: "https://claude.ai/chat" },
    });
    expect(out).toMatchObject({
      type: "tab:fab-save-selection",
      payload: { selectionText: "hello", pageUrl: "https://claude.ai/chat" },
    });
  });

  it("rejects a missing pageUrl", () => {
    expect(
      parseMsg({
        type: "tab:fab-save-selection",
        payload: { selectionText: "hello" },
      }),
    ).toBeNull();
  });

  it("rejects a non-string selectionText", () => {
    expect(
      parseMsg({
        type: "tab:fab-save-selection",
        payload: { selectionText: 42, pageUrl: "https://x" },
      }),
    ).toBeNull();
  });
});

describe("parseMsg — tab:fab-open-popup", () => {
  it("accepts a bare type discriminant (no payload)", () => {
    const out = parseMsg({ type: "tab:fab-open-popup" });
    expect(out).toEqual({ type: "tab:fab-open-popup" });
  });
});

describe("parseMsg — defensive", () => {
  it("returns null on hostile / unknown input", () => {
    expect(parseMsg(null)).toBeNull();
    expect(parseMsg(42)).toBeNull();
    expect(parseMsg({})).toBeNull();
    expect(parseMsg({ type: "unknown:thing" })).toBeNull();
  });
});

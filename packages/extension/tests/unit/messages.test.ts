// Message-parser tests. Pins the discriminated-union shape so contract
// drift between popup / content / SW realms is caught at CI time.

import { describe, it, expect } from "vitest";
import { parseMsg } from "../../src/messages.js";

describe("parseMsg — tab:fab-* messages (FAB menu items)", () => {
  it("accepts tab:fab-save-chat with a non-empty url", () => {
    const msg = parseMsg({
      type: "tab:fab-save-chat",
      payload: { url: "https://chatgpt.com/c/abc" },
    });
    expect(msg).not.toBeNull();
    expect(msg?.type).toBe("tab:fab-save-chat");
  });

  it("rejects tab:fab-save-chat with empty url", () => {
    const msg = parseMsg({
      type: "tab:fab-save-chat",
      payload: { url: "" },
    });
    expect(msg).toBeNull();
  });

  it("accepts tab:fab-save-selection with selectionText + pageUrl", () => {
    const msg = parseMsg({
      type: "tab:fab-save-selection",
      payload: {
        selectionText: "highlighted",
        pageUrl: "https://chatgpt.com/c/abc",
      },
    });
    expect(msg).not.toBeNull();
    expect(msg?.type).toBe("tab:fab-save-selection");
  });

  it("accepts tab:fab-save-selection with optional pageTitle", () => {
    const msg = parseMsg({
      type: "tab:fab-save-selection",
      payload: {
        selectionText: "highlighted",
        pageUrl: "https://chatgpt.com/c/abc",
        pageTitle: "ChatGPT — Some Conversation",
      },
    });
    expect(msg).not.toBeNull();
  });

  it("accepts tab:fab-open-popup with no payload", () => {
    const msg = parseMsg({ type: "tab:fab-open-popup" });
    expect(msg).not.toBeNull();
    expect(msg?.type).toBe("tab:fab-open-popup");
  });

  it("rejects unknown message types", () => {
    expect(parseMsg({ type: "tab:unknown" })).toBeNull();
    expect(parseMsg({})).toBeNull();
    expect(parseMsg(null)).toBeNull();
  });
});

describe("parseMsg — ui:recall ownerPubkey is optional", () => {
  it("accepts ui:recall without ownerPubkey", () => {
    const msg = parseMsg({
      type: "ui:recall",
      payload: { query: "hello", limit: 5 },
    });
    expect(msg).not.toBeNull();
    expect(msg?.type).toBe("ui:recall");
  });

  it("accepts ui:recall with ownerPubkey when present", () => {
    const msg = parseMsg({
      type: "ui:recall",
      payload: { query: "hello", limit: 5, ownerPubkey: "PubABC" },
    });
    expect(msg).not.toBeNull();
  });

  it("rejects ui:recall with empty ownerPubkey when present", () => {
    const msg = parseMsg({
      type: "ui:recall",
      payload: { query: "hello", limit: 5, ownerPubkey: "" },
    });
    expect(msg).toBeNull();
  });
});

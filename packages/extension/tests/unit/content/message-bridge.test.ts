// @vitest-environment jsdom

// Unit tests for the content-script message bridge (PR134-C-03 + D13).
// Covers the listener's three request types, the textContent
// false-positive guard (PR134-C-02), and the cross-extension sender
// rejection (PR134-S-03). The full adapter registry side-effects fire
// on import so we stub `selectAdapter` via the registry's exported
// helpers rather than re-running content-script bootstrap.

import { describe, it, expect, beforeEach, vi } from "vitest";

interface ChromeRuntimeMock {
  id: string;
  onMessage: {
    listeners: Array<
      (
        raw: unknown,
        sender: chrome.runtime.MessageSender,
        sendResponse: (response?: unknown) => void,
      ) => boolean
    >;
    addListener: (
      cb: (
        raw: unknown,
        sender: chrome.runtime.MessageSender,
        sendResponse: (response?: unknown) => void,
      ) => boolean,
    ) => void;
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
    sendMessage: async () => undefined,
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

describe("message-bridge", () => {
  it("ignores unknown message types and returns false", async () => {
    const runtime = installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const listener = mod.buildMessageListener();
    const sendResponse = vi.fn();
    const result = listener(
      { type: "ui:not-a-real-message" },
      { id: runtime.id } as chrome.runtime.MessageSender,
      sendResponse,
    );
    expect(result).toBe(false);
    expect(sendResponse).not.toHaveBeenCalled();
  });

  it("rejects messages from a different extension id (PR134-S-03)", async () => {
    const runtime = installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const listener = mod.buildMessageListener();
    const sendResponse = vi.fn();
    void runtime; // touch to silence unused-var
    const result = listener(
      { type: "ui:get-selection" },
      { id: "some-other-extension" } as chrome.runtime.MessageSender,
      sendResponse,
    );
    expect(result).toBe(false);
    expect(sendResponse).not.toHaveBeenCalled();
  });

  it("ui:get-selection returns the current window selection", async () => {
    installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const reply = mod.getSelectionText();
    // jsdom returns "" when no selection; that's the contract.
    expect(reply.selectionText).toBe("");
  });

  it("ui:extract-conversation returns null when no adapter matches the host", async () => {
    installChrome();
    // jsdom's default url is `http://localhost/` which does not match
    // any of the chatgpt/claude/gemini adapter host patterns.
    const mod = await import("../../../src/content/message-bridge.js");
    const result = mod.extractConversation();
    expect(result).toBeNull();
  });

  it("ui:insert-into-chat returns ok=false when no adapter matches the host", async () => {
    installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const result = mod.insertIntoChat("hello");
    expect(result.ok).toBe(false);
  });

  it("buildMessageListener wires extract-conversation through extractConversation", async () => {
    const runtime = installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const listener = mod.buildMessageListener();
    const sendResponse = vi.fn();
    listener(
      { type: "ui:extract-conversation" },
      { id: runtime.id } as chrome.runtime.MessageSender,
      sendResponse,
    );
    expect(sendResponse).toHaveBeenCalledTimes(1);
    // No adapter for `localhost` → null reply.
    expect(sendResponse.mock.calls[0]?.[0]).toBeNull();
  });

  it("buildMessageListener replies ok=false when insertIntoChat finds no adapter", async () => {
    const runtime = installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const listener = mod.buildMessageListener();
    const sendResponse = vi.fn();
    listener(
      { type: "ui:insert-into-chat", payload: { text: "hello" } },
      { id: runtime.id } as chrome.runtime.MessageSender,
      sendResponse,
    );
    expect(sendResponse).toHaveBeenCalledWith({ ok: false });
  });

  it("buildMessageListener replies ok=false when insert payload is not a string", async () => {
    const runtime = installChrome();
    const mod = await import("../../../src/content/message-bridge.js");
    const listener = mod.buildMessageListener();
    const sendResponse = vi.fn();
    listener(
      { type: "ui:insert-into-chat", payload: { text: 42 } },
      { id: runtime.id } as chrome.runtime.MessageSender,
      sendResponse,
    );
    expect(sendResponse).toHaveBeenCalledWith({ ok: false });
  });

  it("insertIntoChat textContent fallback verifies retention (PR134-C-02)", async () => {
    // Install chrome BEFORE import so the module's top-level
    // `chrome.runtime.onMessage.addListener` call succeeds.
    installChrome();
    // Stub the registry to return a synthetic adapter that exposes a
    // contenteditable input. ProseMirror-like behaviour is simulated
    // by overriding the innerText getter so it does NOT reflect the
    // textContent mutation — that's the silent-discard the live-debug
    // PR shipped and this fix catches.
    const registry = await import("../../../src/runtime/chat/registry.js");
    const editor = document.createElement("div");
    editor.contentEditable = "true";
    Object.defineProperty(editor, "isContentEditable", {
      configurable: true,
      get: () => true,
    });
    document.body.appendChild(editor);
    // Force every insertion strategy to FAIL by overriding innerText
    // to always return an empty string.
    Object.defineProperty(editor, "innerText", {
      configurable: true,
      get: () => "",
    });
    // execCommand returns false → first strategy bails.
    document.execCommand = () => false;

    registry.__setAdaptersForTesting([
      {
        hostPattern: /.*/u,
        platform: "test",
        supportsInsert: true,
        extractConversation: () => [],
        findInputBox: () => editor,
        getChatId: () => null,
      },
    ]);

    const mod = await import("../../../src/content/message-bridge.js");
    const result = mod.insertIntoChat("hello world");
    // textContent was set, but `innerText.includes` returned false →
    // the wrapper must report ok=false so the popup can offer the
    // clipboard fallback. Previous wiring returned ok=true blindly.
    expect(result.ok).toBe(false);

    registry.__setAdaptersForTesting([]);
  });

  it("insertIntoChat returns ok=true when innerText retains the text", async () => {
    installChrome();
    const registry = await import("../../../src/runtime/chat/registry.js");
    const editor = document.createElement("div");
    editor.contentEditable = "true";
    Object.defineProperty(editor, "isContentEditable", {
      configurable: true,
      get: () => true,
    });
    document.body.appendChild(editor);
    Object.defineProperty(editor, "innerText", {
      configurable: true,
      get: () => editor.textContent ?? "",
    });
    // execCommand insertText with a writable target works in jsdom:
    // fall through to the textContent fallback by returning false here.
    document.execCommand = () => false;

    registry.__setAdaptersForTesting([
      {
        hostPattern: /.*/u,
        platform: "test",
        supportsInsert: true,
        extractConversation: () => [],
        findInputBox: () => editor,
        getChatId: () => null,
      },
    ]);

    const mod = await import("../../../src/content/message-bridge.js");
    const result = mod.insertIntoChat("hello world");
    expect(result.ok).toBe(true);
    expect(editor.textContent).toContain("hello world");

    registry.__setAdaptersForTesting([]);
  });

  it("insertIntoChat sets value on a textarea adapter input", async () => {
    installChrome();
    const registry = await import("../../../src/runtime/chat/registry.js");
    const ta = document.createElement("textarea");
    document.body.appendChild(ta);

    registry.__setAdaptersForTesting([
      {
        hostPattern: /.*/u,
        platform: "test",
        supportsInsert: true,
        extractConversation: () => [],
        findInputBox: () => ta,
        getChatId: () => null,
      },
    ]);

    const mod = await import("../../../src/content/message-bridge.js");
    const result = mod.insertIntoChat("typed text");
    expect(result.ok).toBe(true);
    expect(ta.value).toBe("typed text");

    registry.__setAdaptersForTesting([]);
  });
});

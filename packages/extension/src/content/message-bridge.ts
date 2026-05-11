// Content-script message bridge — the missing handler for popup-driven
// reads of the current page: `ui:extract-conversation`, `ui:get-selection`,
// `ui:insert-into-chat`. Without this file the popup's "Save chat" path
// (`runtime.getActiveTabConversation`) silently returns null and the
// user sees "Could not read this chat. Open the conversation tab first."
//
// Architecture note: the adapter modules (chatgpt/claude/gemini) register
// themselves into the popup-realm registry via `registerAdapter()`. When
// they're listed in `manifest.json` content_scripts, the SAME module
// runs in BOTH the content-script realm and the popup realm — but the
// two are different JS contexts, so the registry the content script
// populates is local to that page. We re-use it here.

// Import each adapter for its side-effect of `registerAdapter(...)` at
// module load. vite bundles each `manifest.json` content_scripts entry
// separately, so the adapter's bundled registry is NOT visible to
// us unless we import them into our own bundle. With these imports,
// `selectAdapter(url)` below finds the right adapter for the current
// host.
import "../runtime/chat/adapters/chatgpt.adapter.js";
import "../runtime/chat/adapters/claude.adapter.js";
import "../runtime/chat/adapters/gemini.adapter.js";
import { selectAdapter } from "../runtime/chat/registry.js";
import type { ChatTurn } from "../runtime/chat/types.js";

type Msg =
  | { type: "ui:extract-conversation" }
  | { type: "ui:get-selection" }
  | { type: "ui:insert-into-chat"; payload: { text: string } };

function isMsg(raw: unknown): raw is Msg {
  if (typeof raw !== "object" || raw === null) return false;
  const t = (raw as { type?: unknown }).type;
  return (
    t === "ui:extract-conversation" ||
    t === "ui:get-selection" ||
    t === "ui:insert-into-chat"
  );
}

function extractConversation(): { turns: ChatTurn[] } | null {
  const adapter = selectAdapter(window.location.href);
  if (!adapter) return null;
  try {
    return { turns: adapter.extractConversation(document) };
  } catch (err) {
    console.error("[mnemonik] extractConversation failed", err);
    return null;
  }
}

function getSelectionText(): { selectionText: string } {
  return { selectionText: window.getSelection()?.toString() ?? "" };
}

function insertIntoChat(text: string): { ok: boolean } {
  const adapter = selectAdapter(window.location.href);
  console.log("[mnemonik] insertIntoChat: adapter =", adapter?.platform);
  if (!adapter || !adapter.supportsInsert) return { ok: false };
  const input = adapter.findInputBox(document);
  console.log(
    "[mnemonik] insertIntoChat: input found =",
    !!input,
    input?.tagName,
    input?.id,
    input?.className,
  );
  if (!input) return { ok: false };

  // Plain textarea — set value + dispatch input event.
  if (input instanceof HTMLTextAreaElement) {
    input.focus();
    input.value = text;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return { ok: true };
  }

  if (!input.isContentEditable) return { ok: false };

  // contenteditable. ChatGPT uses ProseMirror at `#prompt-textarea`.
  // ProseMirror ignores plain DOM mutations + execCommand on most
  // versions, but reliably handles synthetic paste events because its
  // paste plugin reads from `clipboardData`. Try in order:
  //   1. Move caret to end of editor + execCommand('insertText') —
  //      fires `beforeinput`, which lightweight editors accept.
  //   2. Dispatch a synthetic `paste` ClipboardEvent — ProseMirror /
  //      Lexical / Slate all intercept this.
  //   3. Fall back to `textContent =` + manual `input` event.
  input.focus();
  try {
    const sel = window.getSelection();
    if (sel) {
      const range = document.createRange();
      range.selectNodeContents(input);
      range.collapse(false);
      sel.removeAllRanges();
      sel.addRange(range);
    }
  } catch {
    // selection APIs occasionally throw on detached editors
  }

  const ok1 = (() => {
    try {
      return document.execCommand("insertText", false, text);
    } catch {
      return false;
    }
  })();
  console.log(
    "[mnemonik] insertIntoChat: execCommand ok =",
    ok1,
    "innerText match =",
    input.innerText.includes(text),
  );
  if (ok1 && input.innerText.includes(text)) return { ok: true };

  // Synthetic paste — works on ProseMirror, Lexical, Slate.
  try {
    const dt = new DataTransfer();
    dt.setData("text/plain", text);
    const paste = new ClipboardEvent("paste", {
      clipboardData: dt,
      bubbles: true,
      cancelable: true,
    });
    input.dispatchEvent(paste);
    console.log(
      "[mnemonik] insertIntoChat: paste dispatched, innerText match =",
      input.innerText.includes(text),
    );
    if (input.innerText.includes(text)) return { ok: true };
  } catch (err) {
    console.log("[mnemonik] insertIntoChat: paste failed", err);
  }

  // Last resort: replace textContent directly + fire input event.
  try {
    input.textContent = text;
    input.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        data: text,
        inputType: "insertText",
      }),
    );
    console.log(
      "[mnemonik] insertIntoChat: textContent set, innerText match =",
      input.innerText.includes(text),
    );
    return { ok: true };
  } catch {
    return { ok: false };
  }
}

chrome.runtime.onMessage.addListener(
  (
    raw: unknown,
    _sender: chrome.runtime.MessageSender,
    sendResponse: (response?: unknown) => void,
  ) => {
    if (!isMsg(raw)) return false;
    if (raw.type === "ui:extract-conversation") {
      sendResponse(extractConversation());
      return false;
    }
    if (raw.type === "ui:get-selection") {
      sendResponse(getSelectionText());
      return false;
    }
    if (raw.type === "ui:insert-into-chat") {
      const text = raw.payload?.text;
      if (typeof text !== "string") {
        sendResponse({ ok: false });
        return false;
      }
      sendResponse(insertIntoChat(text));
      return false;
    }
    return false;
  },
);

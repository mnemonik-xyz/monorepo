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

/** Debug-only logger. PR134-S-04 / PR134-C-08 / BUG-05: every diagnostic
 *  console.log from the live-debug session is gated behind
 *  `import.meta.env.DEV` so Vite tree-shakes them in production builds.
 *  Error-level logs (`console.error`) stay enabled — those are the
 *  operator signals the security audit explicitly approved. */
function debugLog(...args: unknown[]): void {
  // `import.meta.env.DEV` is statically true in dev and false in
  // production; Vite/Rollup constant-folds the entire branch away.
  if (import.meta.env.DEV) {
    // eslint-disable-next-line no-console
    console.log("[mnemonik]", ...args);
  }
}

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

export function extractConversation(): { turns: ChatTurn[] } | null {
  const adapter = selectAdapter(window.location.href);
  if (!adapter) return null;
  try {
    return { turns: adapter.extractConversation(document) };
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("[mnemonik] extractConversation failed", err);
    return null;
  }
}

export function getSelectionText(): { selectionText: string } {
  return { selectionText: window.getSelection()?.toString() ?? "" };
}

/**
 * Insert `text` at the active chat composer. Returns `{ok: true}`
 * ONLY when the resulting DOM actually contains the text — otherwise
 * the popup can fall back to a clipboard copy. PR134-C-02: the previous
 * `textContent =` last-resort fired and returned ok=true even on
 * ProseMirror, which silently reconciles the mutation away on the
 * next render; verifying via `innerText.includes(text)` closes that.
 */
export function insertIntoChat(text: string): { ok: boolean } {
  const adapter = selectAdapter(window.location.href);
  debugLog("insertIntoChat: adapter =", adapter?.platform);
  if (!adapter || !adapter.supportsInsert) return { ok: false };
  const input = adapter.findInputBox(document);
  debugLog(
    "insertIntoChat: input found =",
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
    return { ok: input.value.includes(text) };
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
  //   3. Fall back to `textContent =` + manual `input` event, with
  //      an innerText verification to catch ProseMirror reconciliation.
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
  debugLog(
    "insertIntoChat: execCommand ok =",
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
    debugLog(
      "insertIntoChat: paste dispatched, innerText match =",
      input.innerText.includes(text),
    );
    if (input.innerText.includes(text)) return { ok: true };
  } catch (err) {
    debugLog("insertIntoChat: paste failed", err);
  }

  // Last resort: replace textContent directly + fire input event.
  // PR134-C-02: verify that the editor actually retained the text
  // before returning `{ok: true}`. ProseMirror's reconciler runs
  // synchronously inside the same microtask on the input dispatch,
  // so reading `innerText` immediately after the dispatch reflects
  // the post-reconcile state. If the framework discarded the mutation,
  // return `{ok: false}` so the caller can offer a clipboard copy.
  try {
    input.textContent = text;
    input.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        data: text,
        inputType: "insertText",
      }),
    );
    const retained = input.innerText.includes(text);
    debugLog("insertIntoChat: textContent set, innerText match =", retained);
    return { ok: retained };
  } catch {
    return { ok: false };
  }
}

/**
 * Build the `chrome.runtime.onMessage` listener. Exposed for tests so
 * they can drive it directly with a synthetic sender + capture the
 * sendResponse value without monkey-patching the global.
 */
export function buildMessageListener(): (
  raw: unknown,
  sender: chrome.runtime.MessageSender,
  sendResponse: (response?: unknown) => void,
) => boolean {
  return (raw, sender, sendResponse) => {
    // PR134-S-03: defence-in-depth sender validation. Only messages
    // that originate from our own extension (popup / SW / other content
    // scripts) are processed. `externally_connectable` is absent from
    // manifest.json so this is belt-and-braces, but the audit flagged
    // the missing check explicitly.
    const ownId = chrome.runtime?.id;
    if (
      typeof ownId === "string" &&
      sender.id !== undefined &&
      sender.id !== ownId
    ) {
      return false;
    }
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
  };
}

chrome.runtime.onMessage.addListener(buildMessageListener());

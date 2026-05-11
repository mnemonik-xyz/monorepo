// ChatGPT (`chatgpt.com`) adapter — plugs into `runtime/chat/registry.ts`
// per the framework laid down in T06. DOM selectors target the 2026 layout:
// per-turn `[data-message-author-role]` nodes, a streaming-state
// `data-stream` attribute on the assistant turn while tokens are still
// arriving, and `textarea[data-id="root"]` for the prompt input. Verify
// the selectors on capture day — the live ChatGPT DOM is not stable.

import { registerAdapter } from "../registry.js";
import { domNodeToMarkdown } from "../serializer.js";
import type { ChatAdapter, ChatTurn } from "../types.js";

/** Path segment after `/c/` is the stable chat id (typically a UUID). */
const CHAT_ID_PATH = /^\/c\/([^/?#]+)/;

/** Streaming-state attribute on the assistant turn while tokens stream in. */
const STREAM_ATTR = "data-stream";

function inferRole(value: string | null): ChatTurn["role"] | null {
  if (value === "user" || value === "assistant" || value === "system") {
    return value;
  }
  // ChatGPT also surfaces `tool` / `function` roles internally; fold them
  // under `system` so the conversation still serialises rather than
  // dropping context. Adapters MAY choose differently — generic fold is
  // the conservative choice for cross-platform recall.
  return value ? "system" : null;
}

export const chatgptAdapter: ChatAdapter = {
  hostPattern: /^chatgpt\.com\//,
  platform: "chatgpt",
  supportsInsert: true,

  extractConversation(doc: Document): ChatTurn[] {
    const nodes = doc.querySelectorAll<HTMLElement>(
      "[data-message-author-role]"
    );
    const turns: ChatTurn[] = [];
    for (const node of Array.from(nodes)) {
      const role = inferRole(node.getAttribute("data-message-author-role"));
      if (!role) continue;
      const content = domNodeToMarkdown(node).trim();
      if (!content) continue;
      turns.push({ role, content });
    }
    return turns;
  },

  findInputBox(doc: Document): HTMLElement | null {
    // Primary selector — current 2026 ChatGPT prompt textarea.
    const textarea = doc.querySelector<HTMLTextAreaElement>(
      'textarea[data-id="root"]'
    );
    if (textarea) return textarea;
    // Fallback: contenteditable composer used during A/B rollouts.
    return doc.querySelector<HTMLElement>(
      '[contenteditable="true"][data-id="root"]'
    );
  },

  getChatId(_doc: Document, location: Location): string | null {
    const match = CHAT_ID_PATH.exec(location.pathname);
    return match?.[1] ?? null;
  },

  onNewAssistantTurn(doc: Document, cb: (turn: ChatTurn) => void): () => void {
    // Track which assistant turns we've already announced + which were
    // observed mid-stream. An emit fires when a `[data-stream]` turn
    // loses that attribute (streaming finished) OR a fresh assistant
    // turn appears already-settled (snapshot replay).
    const announced = new WeakSet<Element>();
    const wasStreaming = new WeakSet<Element>();

    const emit = (turn: Element): void => {
      if (announced.has(turn)) return;
      announced.add(turn);
      const role = inferRole(turn.getAttribute("data-message-author-role"));
      if (role !== "assistant") return;
      const content = domNodeToMarkdown(turn).trim();
      if (!content) return;
      cb({ role, content });
    };

    const visit = (turn: Element): void => {
      if (turn.getAttribute("data-message-author-role") !== "assistant") {
        return;
      }
      if (turn.hasAttribute(STREAM_ATTR)) {
        wasStreaming.add(turn);
        return;
      }
      // No stream attribute now — emit if we previously saw it streaming
      // (transition) or if it appeared settled from the start.
      emit(turn);
    };

    // Seed with any assistant turns already in the DOM at subscription time.
    for (const turn of Array.from(
      doc.querySelectorAll<HTMLElement>(
        '[data-message-author-role="assistant"]'
      )
    )) {
      visit(turn);
    }

    // Pull `MutationObserver` from the document's own window so the
    // adapter works under both real browser globals and a JSDOM-backed
    // unit test environment (where Node's globalThis lacks the API).
    const view =
      doc.defaultView ??
      (typeof globalThis !== "undefined" ? globalThis : undefined);
    const Observer =
      view &&
      (view as { MutationObserver?: typeof MutationObserver }).MutationObserver;
    if (!Observer) return () => {};

    const observer = new Observer((records) => {
      for (const record of records) {
        if (record.type === "childList") {
          for (const added of Array.from(record.addedNodes)) {
            if (added.nodeType !== 1 /* ELEMENT_NODE */) continue;
            const el = added as Element;
            if (el.matches?.("[data-message-author-role]")) {
              visit(el);
            }
            for (const inner of Array.from(
              el.querySelectorAll?.('[data-message-author-role="assistant"]') ??
                []
            )) {
              visit(inner);
            }
          }
        } else if (
          record.type === "attributes" &&
          record.attributeName === STREAM_ATTR
        ) {
          const target = record.target as Element;
          if (
            target.getAttribute("data-message-author-role") === "assistant" &&
            !target.hasAttribute(STREAM_ATTR) &&
            wasStreaming.has(target)
          ) {
            emit(target);
          }
        }
      }
    });

    const root = doc.body ?? doc.documentElement;
    if (root) {
      observer.observe(root, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: [STREAM_ATTR],
      });
    }

    return () => observer.disconnect();
  },
};

// Self-register on import. Bootstrap modules (popup, content script,
// service worker) opt in to ChatGPT support by importing this module
// (or `runtime/chat/adapters/index.ts`).
registerAdapter(chatgptAdapter);

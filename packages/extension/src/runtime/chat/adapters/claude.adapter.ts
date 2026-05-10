// Claude.ai (`claude.ai/chat/<uuid>`) DOM adapter — read-only Phase 1
// per D8 / D10 / D12. The page renders each turn under
// `[data-testid^="message-"]`; role is encoded in the testid suffix
// (`message-user-…`, `message-assistant-…`, `message-human-…`) with
// nested-class fallbacks for layout variants where the suffix is just
// an index. Content goes through `domNodeToMarkdown` so fenced code
// blocks survive the round-trip via the shared serializer (T06).

import { registerAdapter } from "../registry.js";
import { domNodeToMarkdown } from "../serializer.js";
import type { ChatAdapter, ChatTurn } from "../types.js";

const MESSAGE_SELECTOR = '[data-testid^="message-"]';
// Action buttons that only appear once an assistant turn finishes
// streaming. Copy is the most stable signal; regenerate / retry are
// secondary fallbacks for layout variants.
const ASSISTANT_DONE_SELECTOR = [
  '[data-testid="action-bar-copy"]',
  '[data-testid="copy-button"]',
  '[data-testid="regenerate-button"]',
  '[data-testid="retry-button"]',
  'button[aria-label*="Copy" i]',
  'button[aria-label*="Regenerate" i]',
  'button[aria-label*="Retry" i]',
].join(",");

const CHAT_ID_PATTERN =
  /^\/chat\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i;

function roleFromMessage(el: Element): ChatTurn["role"] | null {
  const testid = el.getAttribute("data-testid") ?? "";
  // `message-user-…`, `message-human-…`, `message-assistant-…`.
  const suffix = testid.slice("message-".length).toLowerCase();
  if (suffix.startsWith("assistant")) return "assistant";
  if (suffix.startsWith("user") || suffix.startsWith("human")) return "user";
  if (suffix.startsWith("system")) return "system";

  // Layout variants: testid suffix is just an index (e.g. `message-1`).
  // Fall back to nested data-testid markers and class hints.
  if (el.querySelector('[data-testid="user-message"]')) return "user";
  if (el.querySelector('[data-testid="assistant-message"]')) return "assistant";

  const className = el.getAttribute("class") ?? "";
  if (/\b(user|human)-message\b/i.test(className)) return "user";
  if (/\bassistant-message\b/i.test(className)) return "assistant";
  return null;
}

function extractContent(el: Element): string {
  // Prefer the nested message-content node when present (skips action
  // bars / avatars). Fall back to the message root so we never silently
  // drop turns whose layout doesn't match the canonical wrapper.
  const body =
    el.querySelector('[data-testid$="-message-content"]') ??
    el.querySelector('[data-testid="message-content"]') ??
    el;
  return domNodeToMarkdown(body).trim();
}

export const claudeAdapter: ChatAdapter = {
  hostPattern: /^claude\.ai\//,
  platform: "claude",

  extractConversation(doc: Document): ChatTurn[] {
    const nodes = Array.from(doc.querySelectorAll(MESSAGE_SELECTOR));
    const turns: ChatTurn[] = [];
    for (const node of nodes) {
      const role = roleFromMessage(node);
      if (!role) continue;
      const content = extractContent(node);
      if (!content) continue;
      turns.push({ role, content });
    }
    return turns;
  },

  findInputBox(): HTMLElement | null {
    // Phase 1 is read-only — paste-into-chat lands with the recall
    // overlay rework (backlog).
    return null;
  },

  getChatId(_doc: Document, location: Location): string | null {
    const match = CHAT_ID_PATTERN.exec(location.pathname);
    return match?.[1] ?? null;
  },

  onNewAssistantTurn(
    doc: Document,
    cb: (turn: ChatTurn) => void,
  ): () => void {
    const fired = new WeakSet<Element>();

    const fireIfReady = (msg: Element): void => {
      if (fired.has(msg)) return;
      if (roleFromMessage(msg) !== "assistant") return;
      if (!msg.querySelector(ASSISTANT_DONE_SELECTOR)) return;
      fired.add(msg);
      const content = extractContent(msg);
      if (!content) return;
      cb({ role: "assistant", content });
    };

    const scan = (root: ParentNode): void => {
      const messages = root.querySelectorAll(MESSAGE_SELECTOR);
      messages.forEach(fireIfReady);
    };

    // The chat container is virtualised, so attach to <body> rather
    // than guess at the scroll wrapper — cheap, and survives Claude
    // shipping a new layout class.
    const target = doc.body ?? doc.documentElement;
    const observer = new MutationObserver((mutations) => {
      for (const m of mutations) {
        if (m.type === "childList") {
          m.addedNodes.forEach((n) => {
            if (n.nodeType === 1) scan(n as Element);
          });
        }
        if (m.type === "attributes" || m.type === "characterData") {
          const node = m.target;
          if (node.nodeType === 1) {
            const msg = (node as Element).closest(MESSAGE_SELECTOR);
            if (msg) fireIfReady(msg);
          }
        }
      }
    });
    observer.observe(target, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["data-testid", "aria-label"],
    });
    // Fire for any assistant turns already settled at attach time.
    scan(doc);
    return () => observer.disconnect();
  },
};

registerAdapter(claudeAdapter);

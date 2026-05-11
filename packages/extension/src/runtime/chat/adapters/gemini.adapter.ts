// Gemini (gemini.google.com) `ChatAdapter`. Read-only Phase 1 — no
// `findInputBox` (returns null), no auto-paste UI yet. Drives D8/D10/D11.
//
// Gemini renders turns inside web components — typically `<user-query>`
// for user prompts and `<message-content>` for assistant responses — and
// the actual rendered text lives inside open shadow roots. The shared
// `domNodeToMarkdown` helper in `../serializer.ts` is intentionally
// shadow-blind (per T06 framework note: extend locally, don't bloat the
// shared helper), so this adapter walks shadow roots itself when
// extracting content.
//
// `getChatId` parses `https://gemini.google.com/app/<id>` (Gemini's stable
// per-conversation URL convention). The optional `/u/<n>/` account-prefix
// is tolerated. Pages that aren't scoped to a chat (root, settings, share
// landing pages without an `/app/<id>` segment) return `null` so the
// popup falls back to generic page-selection capture (D8).

import type { ChatAdapter, ChatTurn } from "../types.js";
import { registerAdapter } from "../registry.js";
import { domNodeToMarkdown } from "../serializer.js";

const USER_TAGS = new Set(["user-query", "user-query-content"]);
const ASSISTANT_TAGS = new Set(["message-content", "model-response"]);
const TURN_TAGS = new Set([...USER_TAGS, ...ASSISTANT_TAGS]);

/**
 * Recursive shadow-DOM walker. Yields every Element under `root`
 * including those nested inside open shadow roots. Closed shadow roots
 * are inaccessible by spec — Gemini ships open roots as of capture day,
 * so this is sufficient.
 *
 * Implementation note: the unit tests run in a `node` Vitest
 * environment with JSDOM-provided document instances, so the `Document`
 * / `ShadowRoot` global classes don't necessarily match the parent
 * window's. Branching on `nodeType` (9 = Document, 1 = Element) is
 * `instanceof`-free and works for both production and tests.
 */
function* walkAll(root: Document | Element | ShadowRoot): Generator<Element> {
  const start: Element | ShadowRoot =
    root.nodeType === 9 /* DOCUMENT_NODE */
      ? (root as Document).documentElement
      : (root as Element | ShadowRoot);
  if (!start) return;

  // Pre-order DFS using a LIFO stack — push children + shadow root in
  // reverse so the leftmost child pops first and document order is
  // preserved. Shadow root is pushed last (popped first after the host)
  // so shadow-DOM content appears immediately under its host, matching
  // the rendered tree.
  const stack: (Element | ShadowRoot)[] = [start];
  while (stack.length) {
    const node = stack.pop()!;
    const isElement = node.nodeType === 1; /* ELEMENT_NODE */
    if (isElement) yield node as Element;
    const children = Array.from(node.children);
    for (let i = children.length - 1; i >= 0; i--) {
      stack.push(children[i]!);
    }
    if (isElement && (node as Element).shadowRoot) {
      stack.push((node as Element).shadowRoot!);
    }
  }
}

/**
 * Shadow-aware variant of `domNodeToMarkdown`: when an element has an
 * open shadow root, recurse into it instead of `textContent` (which
 * skips shadow-DOM content). Falls back to the shared helper for the
 * pre/code-fence handling.
 */
function shadowAwareMarkdown(node: Node): string {
  if (node.nodeType === 3 /* TEXT_NODE */) {
    return node.textContent ?? "";
  }
  if (node.nodeType !== 1 /* ELEMENT_NODE */) {
    return "";
  }
  const el = node as Element;
  const tag = el.tagName.toLowerCase();

  // Code blocks: defer to the shared helper so language-* hint extraction
  // and the trailing-newline strip stay in one place.
  if (tag === "pre" || tag === "code") {
    return domNodeToMarkdown(el);
  }

  let out = "";
  if (el.shadowRoot) {
    for (const child of Array.from(el.shadowRoot.childNodes)) {
      out += shadowAwareMarkdown(child);
    }
  }
  for (const child of Array.from(el.childNodes)) {
    out += shadowAwareMarkdown(child);
  }
  return out;
}

function roleFor(tag: string): ChatTurn["role"] | null {
  if (USER_TAGS.has(tag)) return "user";
  if (ASSISTANT_TAGS.has(tag)) return "assistant";
  return null;
}

/**
 * `true` when `el` is nested inside another turn element (in either the
 * light or shadow tree). Prevents double-counting when Gemini wraps a
 * `<user-query-content>` inside a `<user-query>` — only the outer one
 * should produce a turn.
 */
function isNestedTurn(el: Element): boolean {
  let node: Element | null = el.parentElement;
  while (node) {
    if (TURN_TAGS.has(node.tagName.toLowerCase())) return true;
    node = node.parentElement;
  }
  // Climb out of shadow roots too — `parentElement` stops at the shadow
  // boundary, so check the host explicitly. ShadowRoot is a
  // DocumentFragment (nodeType 11) with a `host` back-reference; that
  // duck-type check avoids the `instanceof` cross-realm pitfall in
  // node-environment tests.
  const root = el.getRootNode() as Node & { host?: Element };
  if (root.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */ && root.host) {
    return (
      isNestedTurn(root.host) || TURN_TAGS.has(root.host.tagName.toLowerCase())
    );
  }
  return false;
}

export const geminiAdapter: ChatAdapter = {
  hostPattern: /^gemini\.google\.com\//,
  platform: "gemini",
  // Phase 1 is read-only — `findInputBox` returns null unconditionally.
  // Recall-overlay paste UI lands later; flip when it does.
  supportsInsert: false,

  extractConversation(doc: Document): ChatTurn[] {
    const turns: ChatTurn[] = [];
    for (const el of walkAll(doc)) {
      const tag = el.tagName.toLowerCase();
      const role = roleFor(tag);
      if (!role) continue;
      if (isNestedTurn(el)) continue;

      const content = shadowAwareMarkdown(el).trim();
      if (!content) continue;
      turns.push({ role, content });
    }
    return turns;
  },

  /** Read-only Phase 1 — paste UI lands later. */
  findInputBox(): HTMLElement | null {
    return null;
  },

  getChatId(_doc: Document, location: Location): string | null {
    // `/app/<id>` or `/u/<n>/app/<id>`. Trailing path segments / query /
    // hash are tolerated. Empty / non-`/app/` paths return null so the
    // caller drops to generic capture.
    const match = /\/app\/([A-Za-z0-9_-]+)/.exec(location.pathname);
    return match?.[1] ?? null;
  },

  onNewAssistantTurn(doc: Document, cb: (turn: ChatTurn) => void): () => void {
    // Resolve `MutationObserver` off the document's own window so unit
    // tests in a node Vitest environment (where the global is missing)
    // pick up the JSDOM-provided constructor instead.
    const MO: typeof MutationObserver | undefined =
      (doc.defaultView as { MutationObserver?: typeof MutationObserver } | null)
        ?.MutationObserver ??
      (globalThis as { MutationObserver?: typeof MutationObserver })
        .MutationObserver;
    if (!MO) return () => {};

    const seen = new WeakSet<Element>();
    const observers: MutationObserver[] = [];

    const handleElement = (el: Element): void => {
      const tag = el.tagName.toLowerCase();
      if (ASSISTANT_TAGS.has(tag) && !seen.has(el) && !isNestedTurn(el)) {
        seen.add(el);
        const content = shadowAwareMarkdown(el).trim();
        if (content) cb({ role: "assistant", content });
      }
      // New shadow roots may have appeared anywhere in the subtree —
      // attach an observer to each so subsequent mutations inside the
      // shadow tree fire. Idempotent via the WeakSet guard above.
      if (el.shadowRoot && !shadowsObserved.has(el.shadowRoot)) {
        attachObserver(el.shadowRoot);
      }
    };

    const shadowsObserved = new WeakSet<ShadowRoot>();

    const attachObserver = (root: Document | ShadowRoot): void => {
      const isDocument = root.nodeType === 9; /* DOCUMENT_NODE */
      if (!isDocument) shadowsObserved.add(root as ShadowRoot);
      const target: Element | ShadowRoot | Document = isDocument
        ? (root as Document).body ?? root
        : (root as ShadowRoot);
      const observer = new MO((records) => {
        for (const rec of records) {
          for (const added of Array.from(rec.addedNodes)) {
            if (added.nodeType !== 1 /* ELEMENT_NODE */) continue;
            for (const el of walkAll(added as Element)) handleElement(el);
            handleElement(added as Element);
          }
        }
      });
      observer.observe(target, { childList: true, subtree: true });
      observers.push(observer);
    };

    // Seed: walk the existing tree once so any shadow roots already
    // present get observers attached.
    for (const el of walkAll(doc)) {
      if (el.shadowRoot && !shadowsObserved.has(el.shadowRoot)) {
        attachObserver(el.shadowRoot);
      }
    }
    attachObserver(doc);

    return () => {
      for (const o of observers) o.disconnect();
      observers.length = 0;
    };
  },
};

registerAdapter(geminiAdapter);

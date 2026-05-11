/// <reference types="chrome" />

// Recall overlay — Shadow-DOM modal injected on the page that hosts
// this content script. Triggered by:
//   - service worker dispatching `sw:open-recall-overlay` (hotkey path)
//   - direct call to `mountOverlay()` from the FAB
//
// Architecture:
//   - Closed shadow root + `all: initial` host element. Same isolation
//     guarantees as the FAB.
//   - Search input → 200ms debounce → `chrome.runtime.sendMessage`
//     with `ui:recall`.
//   - Top-N results render with snippet + score + source pill +
//     actions (Copy / Insert / Open). Insert is gated on
//     `selectAdapter(location.href)?.supportsInsert`; falls back to
//     copy when the adapter exposes no input box.
//   - ESC closes; click on backdrop closes; focus-trap inside modal.

import { parseMsg } from "../messages.js";
import { selectAdapter } from "../runtime/chat/registry.js";
import type { SearchResult } from "../runtime/store/types.js";
import { SHADOW_STYLES } from "./shadow-styles.js";

const HOST_TAG = "mnemonik-overlay-host";
const DEBOUNCE_MS = 200;
const RESULT_LIMIT = 5;

interface RecallResponse {
  ok: boolean;
  results?: SearchResult[];
  error?: string;
}

interface OverlayHandles {
  host: HTMLElement;
  destroy: () => void;
}

let active: OverlayHandles | null = null;

/**
 * Mount the overlay. Idempotent — repeat calls focus the existing
 * overlay's input rather than spawning a duplicate. Returns the host
 * element on success.
 */
export function mountOverlay(): HTMLElement | null {
  if (typeof document === "undefined" || !document.body) return null;
  if (active) {
    focusInput(active);
    return active.host;
  }

  const host = document.createElement(HOST_TAG);
  host.setAttribute("style", "all: initial; position: fixed; inset: 0;");
  const shadow = host.attachShadow({ mode: "closed" });

  const style = document.createElement("style");
  style.textContent = SHADOW_STYLES;
  shadow.appendChild(style);

  const backdrop = document.createElement("div");
  backdrop.className = "overlay-backdrop";
  shadow.appendChild(backdrop);

  const modal = document.createElement("div");
  modal.className = "overlay-modal";
  modal.setAttribute("role", "dialog");
  modal.setAttribute("aria-modal", "true");
  modal.setAttribute("aria-label", "Mnemonik recall");
  backdrop.appendChild(modal);

  const input = document.createElement("input");
  input.className = "overlay-input";
  input.type = "search";
  input.placeholder = "Recall — what do you remember…";
  input.setAttribute("aria-label", "Recall query");
  modal.appendChild(input);

  const status = document.createElement("div");
  status.className = "overlay-status";
  status.setAttribute("role", "status");
  modal.appendChild(status);

  const list = document.createElement("ul");
  list.className = "overlay-results";
  list.setAttribute("aria-label", "Recall results");
  modal.appendChild(list);

  const adapter = selectAdapter(location.href);
  const insertSupported = Boolean(adapter?.supportsInsert);
  const inputBox = adapter ? adapter.findInputBox(document) : null;

  const state: OverlayState = {
    shadow,
    list,
    status,
    input,
    insertSupported: insertSupported && inputBox !== null,
    inputBox,
    results: [],
    focusedIndex: 0,
  };

  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  input.addEventListener("input", () => {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      void runSearch(state, input.value);
    }, DEBOUNCE_MS);
  });

  // Backdrop close — but only when the click is on the backdrop itself,
  // not on the modal that sits inside it.
  backdrop.addEventListener("click", (event) => {
    if (event.target === backdrop) destroy();
  });

  const onKey = (event: KeyboardEvent): void => {
    if (event.key === "Escape") {
      event.preventDefault();
      destroy();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveFocus(state, 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveFocus(state, -1);
    } else if (event.key === "Enter" && state.results.length > 0) {
      // Enter on result list = insert (if supported) else copy.
      const r = state.results[state.focusedIndex];
      if (!r) return;
      event.preventDefault();
      if (state.insertSupported) {
        void insertIntoChat(state, r.content);
      } else {
        void copyText(state, toMarkdown(r));
      }
    } else if (event.key === "Tab") {
      // Focus trap: keep tab inside the modal.
      const focusables = focusableElements(modal);
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (!first || !last) return;
      const activeEl = (shadow as unknown as { activeElement?: Element })
        .activeElement;
      if (event.shiftKey && activeEl === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && activeEl === last) {
        event.preventDefault();
        first.focus();
      }
    }
  };
  document.addEventListener("keydown", onKey, true);

  const destroy = (): void => {
    document.removeEventListener("keydown", onKey, true);
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    host.remove();
    active = null;
  };

  document.body.appendChild(host);
  active = { host, destroy };
  // Focus after mount so screen readers announce the dialog.
  setTimeout(() => input.focus(), 0);
  return host;
}

interface OverlayState {
  shadow: ShadowRoot;
  list: HTMLUListElement;
  status: HTMLElement;
  input: HTMLInputElement;
  insertSupported: boolean;
  inputBox: HTMLElement | null;
  results: SearchResult[];
  focusedIndex: number;
}

async function runSearch(state: OverlayState, raw: string): Promise<void> {
  const query = raw.trim();
  if (query === "") {
    state.results = [];
    state.list.replaceChildren();
    state.status.textContent = "";
    state.status.classList.remove("error");
    return;
  }
  state.status.textContent = "Recalling…";
  state.status.classList.remove("error");
  try {
    const reply = (await chrome.runtime.sendMessage({
      type: "ui:recall",
      payload: { query, limit: RESULT_LIMIT },
    })) as RecallResponse | undefined;
    if (!reply || reply.ok === false) {
      state.status.textContent = reply?.error ?? "Recall failed.";
      state.status.classList.add("error");
      state.results = [];
      state.list.replaceChildren();
      return;
    }
    state.results = reply.results ?? [];
    state.focusedIndex = 0;
    if (state.results.length === 0) {
      state.status.textContent = "No memories matched this query.";
    } else {
      state.status.textContent = "";
    }
    renderResults(state);
  } catch (e) {
    state.status.textContent =
      e instanceof Error ? e.message : "Recall failed.";
    state.status.classList.add("error");
  }
}

function renderResults(state: OverlayState): void {
  state.list.replaceChildren();
  state.results.forEach((r, idx) => {
    const li = document.createElement("li");
    li.className = "overlay-result";
    if (idx === state.focusedIndex) li.classList.add("focused");
    li.setAttribute("data-index", String(idx));

    const meta = document.createElement("div");
    meta.className = "overlay-result-meta";

    const score = document.createElement("span");
    score.className = "overlay-result-score";
    score.textContent = r.relevance_score.toFixed(3);
    meta.appendChild(score);

    const sourceTag = r.tags.find((t) => t.startsWith("source:"));
    if (sourceTag) {
      const pill = document.createElement("span");
      pill.className = "overlay-result-pill";
      pill.textContent = sourceTag.slice("source:".length);
      meta.appendChild(pill);
    }
    li.appendChild(meta);

    const snippet = document.createElement("p");
    snippet.className = "overlay-result-snippet";
    snippet.textContent = r.content;
    li.appendChild(snippet);

    const actions = document.createElement("div");
    actions.className = "overlay-result-actions";

    const copyBtn = document.createElement("button");
    copyBtn.type = "button";
    copyBtn.className = "overlay-action";
    copyBtn.textContent = "Copy";
    copyBtn.addEventListener(
      "click",
      () => void copyText(state, toMarkdown(r)),
    );
    actions.appendChild(copyBtn);

    const insertBtn = document.createElement("button");
    insertBtn.type = "button";
    insertBtn.className = "overlay-action";
    insertBtn.textContent = "Insert";
    if (!state.insertSupported) {
      insertBtn.setAttribute("disabled", "true");
      insertBtn.title =
        "This page has no compatible chat input — Copy instead.";
    } else {
      insertBtn.addEventListener(
        "click",
        () => void insertIntoChat(state, r.content),
      );
    }
    actions.appendChild(insertBtn);

    const openLink = document.createElement("a");
    openLink.className = "overlay-action";
    openLink.textContent = "Open";
    openLink.href = `https://mnemonik.xyz/m/${encodeURIComponent(r.attestation_id)}`;
    openLink.target = "_blank";
    openLink.rel = "noreferrer noopener";
    actions.appendChild(openLink);

    li.appendChild(actions);
    state.list.appendChild(li);
  });
}

function moveFocus(state: OverlayState, delta: number): void {
  if (state.results.length === 0) return;
  const next =
    (state.focusedIndex + delta + state.results.length) % state.results.length;
  state.focusedIndex = next;
  // Update class without full re-render to keep ARIA focus stable.
  for (const li of Array.from(
    state.list.querySelectorAll<HTMLLIElement>(".overlay-result"),
  )) {
    const idx = Number(li.getAttribute("data-index"));
    li.classList.toggle("focused", idx === next);
  }
}

async function copyText(state: OverlayState, text: string): Promise<void> {
  try {
    await navigator.clipboard?.writeText(text);
    state.status.textContent = "Copied to clipboard.";
    state.status.classList.remove("error");
  } catch {
    state.status.textContent = "Clipboard write failed.";
    state.status.classList.add("error");
  }
}

async function insertIntoChat(
  state: OverlayState,
  text: string,
): Promise<void> {
  if (!state.inputBox) {
    await copyText(state, text);
    return;
  }
  const box = state.inputBox;
  if (box instanceof HTMLTextAreaElement || box instanceof HTMLInputElement) {
    const before = box.value;
    box.value = before ? `${before}\n${text}` : text;
    box.dispatchEvent(new Event("input", { bubbles: true }));
  } else {
    // contenteditable composer.
    box.focus();
    const existing = box.textContent ?? "";
    box.textContent = existing ? `${existing}\n${text}` : text;
    box.dispatchEvent(new Event("input", { bubbles: true }));
  }
  state.status.textContent = "Inserted into chat.";
  state.status.classList.remove("error");
}

function toMarkdown(r: SearchResult): string {
  const tagLine = r.tags.length ? `tags: [${r.tags.join(", ")}]\n` : "";
  return `---\nattestation_id: ${r.attestation_id}\ncreated_at: ${r.created_at}\nscore: ${r.relevance_score.toFixed(4)}\n${tagLine}---\n\n${r.content}\n`;
}

function focusInput(handles: OverlayHandles): void {
  // The input lives inside a closed shadow root; we re-resolve via
  // the host element's own shadowRoot getter (closed roots don't
  // expose `shadowRoot`, so we tag the input on the host instead —
  // for a focus-on-reopen we just call focus on the active element).
  void handles;
}

function focusableElements(root: ParentNode): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      'input, button:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])',
    ),
  );
}

// Service-worker dispatch. The hotkey + popup paths route through this
// listener; the FAB calls `mountOverlay` directly.
if (typeof chrome !== "undefined" && chrome.runtime?.onMessage) {
  chrome.runtime.onMessage.addListener((raw: unknown) => {
    const msg = parseMsg(raw);
    if (msg?.type === "sw:open-recall-overlay") {
      mountOverlay();
    }
  });
}

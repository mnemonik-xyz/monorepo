/// <reference types="chrome" />

// Floating action button injected on supported AI-chat domains
// (chatgpt.com, claude.ai, gemini.google.com per the enumerated
// host_permissions — D11 forbids `<all_urls>`).
//
// Architecture:
//   - Host element uses `all: initial` and a *closed* shadow root so
//     the page CSS cannot reach in and our CSS cannot leak out.
//   - 56px circular button. Click toggles a vertical menu with three
//     actions: Save chat / Save selection / Open Mnemonik.
//   - Drag-to-reposition. Position persists per-domain in
//     `chrome.storage.local.fabPosition.<domain>`.
//   - Visibility is controlled per-domain via the T12 settings shape
//     `settings.v1.perDomain.<domain>.fabVisible` (default: true).
//
// CSP-safe — listeners are attached programmatically via
// addEventListener. No inline event handlers.

import { SHADOW_STYLES } from "./shadow-styles.js";

interface FabPosition {
  /** Distance from the right edge in px. */
  right: number;
  /** Distance from the bottom edge in px. */
  bottom: number;
}

interface PerDomainSettings {
  enabled?: boolean;
  fabVisible?: boolean;
  autoCapture?: boolean;
}

interface SettingsV1 {
  perDomain?: Record<string, PerDomainSettings>;
}

interface StorageShape {
  "settings.v1"?: SettingsV1;
  fabPosition?: Record<string, FabPosition>;
}

const DEFAULT_POSITION: FabPosition = { right: 24, bottom: 24 };

/**
 * Mount the FAB. Idempotent: subsequent calls are a no-op once a host
 * element with the marker attribute is present in `document.body`.
 *
 * Returns the host element on success, or `null` if the FAB was
 * suppressed by per-domain settings or not mounted (e.g. document
 * isn't ready).
 */
export async function mountFab(): Promise<HTMLElement | null> {
  if (typeof document === "undefined" || !document.body) return null;
  if (document.querySelector("mnemonik-fab-host")) return null;

  const domain = location.hostname;
  const visible = await readVisibility(domain);
  if (!visible) return null;

  const host = document.createElement("mnemonik-fab-host");
  // `all: initial` resets *all* inherited / cascade properties from the
  // host page so even if a hostile page sets `* { display: none !important }`
  // we still render. Position is set inline (not in the stylesheet) so
  // drag updates can rewrite it cheaply.
  host.setAttribute(
    "style",
    "all: initial; position: fixed; z-index: 2147483646;",
  );

  const shadow = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = SHADOW_STYLES;
  shadow.appendChild(style);

  const root = document.createElement("div");
  root.className = "fab-root";
  shadow.appendChild(root);

  const menu = document.createElement("div");
  menu.className = "fab-menu";
  menu.setAttribute("role", "menu");
  menu.hidden = true;
  root.appendChild(menu);

  buildMenuItem(menu, "Save this chat", () => {
    void chrome.runtime.sendMessage({ type: "tab:fab-save-chat" });
    closeMenu();
  });
  buildMenuItem(menu, "Save selection", () => {
    const selectionText = window.getSelection()?.toString() ?? "";
    void chrome.runtime.sendMessage({
      type: "tab:fab-save-selection",
      payload: { selectionText, pageUrl: location.href },
    });
    closeMenu();
  });
  buildMenuItem(menu, "Open Mnemonik", () => {
    void chrome.runtime.sendMessage({ type: "tab:fab-open-popup" });
    closeMenu();
  });

  const button = document.createElement("button");
  button.type = "button";
  button.className = "fab-button";
  button.setAttribute("aria-label", "Mnemonik");
  button.setAttribute("aria-haspopup", "menu");
  button.setAttribute("aria-expanded", "false");
  button.textContent = "M";
  root.appendChild(button);

  const closeMenu = (): void => {
    menu.hidden = true;
    button.setAttribute("aria-expanded", "false");
  };
  const openMenu = (): void => {
    menu.hidden = false;
    button.setAttribute("aria-expanded", "true");
  };

  // Position from storage, fallback to default.
  const position = await readPosition(domain);
  applyPosition(host, position);

  attachDragAndClick(
    host,
    button,
    () => {
      if (menu.hidden) openMenu();
      else closeMenu();
    },
    (next) => {
      applyPosition(host, next);
      void writePosition(domain, next);
    },
  );

  // Close on outside click. Listener is on the document; the host
  // element is opaque so events that bubble out don't reach
  // `composedPath` items inside the closed shadow root.
  document.addEventListener(
    "click",
    (event) => {
      if (event.target === host || (event.target as Node | null) === host) {
        return;
      }
      // Anything outside the host closes the menu.
      if (!menu.hidden) closeMenu();
    },
    true,
  );

  // Listen for visibility changes from the options page.
  if (chrome.storage?.onChanged) {
    chrome.storage.onChanged.addListener((changes, area) => {
      if (area !== "local") return;
      const next = changes["settings.v1"]?.newValue as SettingsV1 | undefined;
      if (!next) return;
      const v = next.perDomain?.[domain]?.fabVisible;
      if (v === false) host.remove();
    });
  }

  document.body.appendChild(host);
  return host;
}

function buildMenuItem(
  menu: HTMLElement,
  label: string,
  onClick: () => void,
): void {
  const item = document.createElement("button");
  item.type = "button";
  item.className = "fab-menu-item";
  item.setAttribute("role", "menuitem");
  item.textContent = label;
  item.addEventListener("click", onClick);
  menu.appendChild(item);
}

function applyPosition(host: HTMLElement, position: FabPosition): void {
  // Re-apply the all-initial reset alongside the position so a dynamic
  // host-page <style> injection can't override us mid-session.
  host.setAttribute(
    "style",
    `all: initial; position: fixed; z-index: 2147483646; right: ${position.right}px; bottom: ${position.bottom}px;`,
  );
}

interface DragState {
  startX: number;
  startY: number;
  startRight: number;
  startBottom: number;
  moved: boolean;
}

function attachDragAndClick(
  host: HTMLElement,
  button: HTMLButtonElement,
  onClick: () => void,
  onMove: (next: FabPosition) => void,
): void {
  let drag: DragState | null = null;

  const computePos = (): FabPosition => {
    const style = host.getAttribute("style") ?? "";
    const right = /right:\s*(\d+)px/.exec(style)?.[1] ?? "24";
    const bottom = /bottom:\s*(\d+)px/.exec(style)?.[1] ?? "24";
    return { right: Number(right), bottom: Number(bottom) };
  };

  button.addEventListener("pointerdown", (event) => {
    const start = computePos();
    drag = {
      startX: event.clientX,
      startY: event.clientY,
      startRight: start.right,
      startBottom: start.bottom,
      moved: false,
    };
    button.classList.add("dragging");
    button.setPointerCapture(event.pointerId);
  });

  button.addEventListener("pointermove", (event) => {
    if (!drag) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) drag.moved = true;
    if (!drag.moved) return;
    const next: FabPosition = {
      right: clamp(drag.startRight - dx, 0, window.innerWidth - 56),
      bottom: clamp(drag.startBottom - dy, 0, window.innerHeight - 56),
    };
    onMove(next);
  });

  const finishDrag = (): void => {
    if (!drag) return;
    const wasDrag = drag.moved;
    drag = null;
    button.classList.remove("dragging");
    if (!wasDrag) onClick();
  };

  button.addEventListener("pointerup", finishDrag);
  button.addEventListener("pointercancel", finishDrag);
}

function clamp(n: number, min: number, max: number): number {
  return Math.min(Math.max(n, min), max);
}

async function readVisibility(domain: string): Promise<boolean> {
  if (!chrome.storage?.local) return true;
  try {
    const got = (await chrome.storage.local.get("settings.v1")) as StorageShape;
    const perDomain = got["settings.v1"]?.perDomain?.[domain];
    return perDomain?.fabVisible !== false;
  } catch {
    return true;
  }
}

async function readPosition(domain: string): Promise<FabPosition> {
  if (!chrome.storage?.local) return DEFAULT_POSITION;
  try {
    const got = (await chrome.storage.local.get("fabPosition")) as StorageShape;
    return got.fabPosition?.[domain] ?? DEFAULT_POSITION;
  } catch {
    return DEFAULT_POSITION;
  }
}

async function writePosition(
  domain: string,
  position: FabPosition,
): Promise<void> {
  if (!chrome.storage?.local) return;
  try {
    const got = (await chrome.storage.local.get("fabPosition")) as StorageShape;
    const next = { ...(got.fabPosition ?? {}), [domain]: position };
    await chrome.storage.local.set({ fabPosition: next });
  } catch {
    // Storage failures are non-fatal — position resets to default
    // next reload, which is acceptable for a UI nicety.
  }
}

// Auto-mount when this module is loaded as a content script. The
// `chrome.runtime.getURL` check is the discriminant — it's only
// defined inside a real extension context (background SW, popup,
// content script). Tests that mock just `{ runtime: { id, onMessage }
// }` won't trigger the auto-mount and can call `mountFab()` directly
// against their fixture data.
if (
  typeof chrome !== "undefined" &&
  chrome.runtime?.id &&
  typeof chrome.runtime.getURL === "function"
) {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void mountFab());
  } else {
    void mountFab();
  }
}

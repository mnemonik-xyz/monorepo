// Typed message protocol shared between popup, content scripts, and the
// MV3 service worker. Discriminated union on `type` so handlers can switch
// without runtime guards leaking `any` into the rest of the codebase.
//
// All payloads are JSON-serialisable: `chrome.runtime.sendMessage` /
// `chrome.tabs.sendMessage` round-trip through structured clone, so we
// stick to plain data (strings, numbers, booleans, arrays, plain objects).
// Binary blobs (COSE bytes, embeddings) travel as base64 strings on this
// channel; the runtime modules own the binary view internally.
//
// Direction conventions encoded in the `type` prefix:
//   - `sw:*`   — service worker → tab content script
//   - `tab:*`  — tab content script → service worker
//   - `ui:*`   — popup / options page → service worker (or the reverse)
//
// The router in `src/background/service-worker.ts` narrows on `msg.type`
// and dispatches to `src/runtime/*` modules.

/** Generic page selection captured via the context menu (D8 / D11). */
export interface SaveSelectionPayload {
  /** Text the user had highlighted at the time of the gesture. */
  selectionText: string;
  /** URL of the page the gesture originated from. */
  pageUrl: string;
  /** Optional page title for the UI list. */
  pageTitle?: string;
  /** ISO-8601 timestamp the service worker stamped on dispatch. */
  capturedAt: string;
}

/** Hotkey-driven recall-overlay open request. */
export interface OpenRecallOverlayPayload {
  /** Source of the gesture — popup/hotkey/context-menu — for telemetry. */
  trigger: "hotkey" | "popup" | "context-menu";
}

/** Sign-memory dispatch from popup to runtime. */
export interface SignMemoryRequest {
  content: string;
  tags: string[];
  /** Optional platform metadata when the popup was opened on a chat page. */
  source?: {
    platform: string;
    url?: string;
    chatId?: string;
    model?: string;
  };
}

/** Recall request from popup or overlay. */
export interface RecallRequest {
  query: string;
  ownerPubkey: string;
  limit: number;
  tags?: string[];
}

/** Service-worker → tab: open recall overlay UI on the page. */
export type SwOpenRecallOverlay = {
  type: "sw:open-recall-overlay";
  payload: OpenRecallOverlayPayload;
};

/** Service-worker → tab: deliver a context-menu selection capture to the
 *  active tab so it can confirm / annotate before sign. */
export type SwSaveSelection = {
  type: "sw:save-selection";
  payload: SaveSelectionPayload;
};

/** UI → SW: sign a memory (delegated to runtime/sign + runtime/store). */
export type UiSignMemory = {
  type: "ui:sign-memory";
  payload: SignMemoryRequest;
};

/** UI → SW: recall query. */
export type UiRecall = {
  type: "ui:recall";
  payload: RecallRequest;
};

/** UI → SW: explicit "drain the pending queue now" gesture. */
export type UiFlushPending = {
  type: "ui:flush-pending";
};

/** Tab → SW: an adapter / selection script reports a new capture candidate
 *  (e.g. auto-capture mode dispatched an assistant turn). The SW decides
 *  whether to persist based on user opt-ins (D12). */
export type TabCaptureCandidate = {
  type: "tab:capture-candidate";
  payload: {
    platform: string;
    url: string;
    content: string;
    chatId?: string;
  };
};

/** Discriminated union over every message the router knows about. */
export type Msg =
  | SwOpenRecallOverlay
  | SwSaveSelection
  | UiSignMemory
  | UiRecall
  | UiFlushPending
  | TabCaptureCandidate;

/** Narrow `unknown` to `Msg`. Returns null when the shape doesn't match;
 *  callers decide whether to log / drop. Never throws on hostile input.
 *
 *  Per-variant payload shape is validated here so handlers can destructure
 *  fields without separate guards. Closes review round-1 finding T10-S-01
 *  / code-review minor: previously the function returned `input as Msg`
 *  after the type discriminant check alone, which would become a crash
 *  vector once T11/T18 populate the `ui:sign-memory` / `ui:recall`
 *  branches with real payload destructuring. */
export function parseMsg(input: unknown): Msg | null {
  if (!isPlainObject(input)) return null;
  const t = input["type"];
  if (typeof t !== "string") return null;
  switch (t) {
    case "sw:open-recall-overlay":
      return isOpenRecallOverlayMsg(input) ? input : null;
    case "sw:save-selection":
      return isSaveSelectionMsg(input) ? input : null;
    case "ui:sign-memory":
      return isUiSignMemoryMsg(input) ? input : null;
    case "ui:recall":
      return isUiRecallMsg(input) ? input : null;
    case "ui:flush-pending":
      // No payload — type discriminant is the whole contract.
      return { type: "ui:flush-pending" };
    case "tab:capture-candidate":
      return isTabCaptureCandidateMsg(input) ? input : null;
    default:
      return null;
  }
}

// ── per-variant payload guards ─────────────────────────────────────────

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every((x) => typeof x === "string");
}

function isOpenRecallOverlayMsg(
  input: Record<string, unknown>
): input is SwOpenRecallOverlay {
  const p = input["payload"];
  if (!isPlainObject(p)) return false;
  const trigger = p["trigger"];
  return (
    trigger === "hotkey" || trigger === "popup" || trigger === "context-menu"
  );
}

function isSaveSelectionMsg(
  input: Record<string, unknown>
): input is SwSaveSelection {
  const p = input["payload"];
  if (!isPlainObject(p)) return false;
  if (typeof p["selectionText"] !== "string") return false;
  if (typeof p["pageUrl"] !== "string") return false;
  if (typeof p["capturedAt"] !== "string") return false;
  if (p["pageTitle"] !== undefined && typeof p["pageTitle"] !== "string") {
    return false;
  }
  return true;
}

function isUiSignMemoryMsg(
  input: Record<string, unknown>
): input is UiSignMemory {
  const p = input["payload"];
  if (!isPlainObject(p)) return false;
  if (typeof p["content"] !== "string" || p["content"].length === 0) {
    return false;
  }
  if (!isStringArray(p["tags"])) return false;
  const src = p["source"];
  if (src !== undefined) {
    if (!isPlainObject(src)) return false;
    if (typeof src["platform"] !== "string") return false;
    for (const k of ["url", "chatId", "model"] as const) {
      if (src[k] !== undefined && typeof src[k] !== "string") return false;
    }
  }
  return true;
}

function isUiRecallMsg(input: Record<string, unknown>): input is UiRecall {
  const p = input["payload"];
  if (!isPlainObject(p)) return false;
  if (typeof p["query"] !== "string" || p["query"].length === 0) return false;
  if (typeof p["ownerPubkey"] !== "string" || p["ownerPubkey"].length === 0) {
    return false;
  }
  const lim = p["limit"];
  if (
    typeof lim !== "number" ||
    !Number.isInteger(lim) ||
    lim <= 0 ||
    lim > 100
  ) {
    return false;
  }
  if (p["tags"] !== undefined && !isStringArray(p["tags"])) return false;
  return true;
}

function isTabCaptureCandidateMsg(
  input: Record<string, unknown>
): input is TabCaptureCandidate {
  const p = input["payload"];
  if (!isPlainObject(p)) return false;
  if (typeof p["platform"] !== "string") return false;
  if (typeof p["url"] !== "string") return false;
  if (typeof p["content"] !== "string") return false;
  if (p["chatId"] !== undefined && typeof p["chatId"] !== "string")
    return false;
  return true;
}

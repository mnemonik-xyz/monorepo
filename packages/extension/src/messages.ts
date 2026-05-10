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
 *  callers decide whether to log / drop. Never throws on hostile input. */
export function parseMsg(input: unknown): Msg | null {
  if (typeof input !== "object" || input === null) return null;
  const candidate = input as { type?: unknown };
  if (typeof candidate.type !== "string") return null;
  switch (candidate.type) {
    case "sw:open-recall-overlay":
    case "sw:save-selection":
    case "ui:sign-memory":
    case "ui:recall":
    case "ui:flush-pending":
    case "tab:capture-candidate":
      return input as Msg;
    default:
      return null;
  }
}

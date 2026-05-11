// Capture tab — "Save chat" / "Save selection". Both flows funnel into
// the same `runtime.signMemory` call; the difference is just the
// content source (full conversation transcript vs. textarea selection).
//
// Per D8 + D11 the popup never assumes `<all_urls>`. Selection capture
// only works when the active tab matches an adapter (host_permission)
// or when the user has already triggered an `activeTab` gesture — the
// content script delivers the selection text via `chrome.runtime.send-
// Message`, this tab just reads what arrived.

import { useCallback, useEffect, useRef, useState, type JSX } from "react";
import type { ChatAdapter } from "../../runtime/chat/types.js";
import { getRuntime, type SignMemoryResult } from "../runtime.js";
import { Toast } from "../components/Toast.js";

export interface FabIntent {
  kind: "save-selection" | "save-chat";
  selectionText?: string;
  pageUrl?: string;
  capturedAt?: string;
}

export interface CaptureProps {
  adapter: ChatAdapter | null;
  prefilledSelection: string;
  /** FAB-driven intent the popup picked up out of
   *  `chrome.storage.local.pending_fab_action.v1` (PR134-BLK-3 /
   *  BUG-03 / BUG2-02). When `kind === "save-chat"` the Capture tab
   *  auto-triggers the conversation extraction + sign flow. Selection
   *  text is already merged into `prefilledSelection` by the parent. */
  fabIntent?: FabIntent | null;
  /** Acknowledge the FAB intent so the parent clears it from state. */
  onFabIntentConsumed?: () => void;
}

export function Capture(props: CaptureProps): JSX.Element {
  const { adapter, prefilledSelection, fabIntent, onFabIntentConsumed } = props;
  const [selection, setSelection] = useState(prefilledSelection);
  const [tagsRaw, setTagsRaw] = useState("");
  const [busy, setBusy] = useState(false);
  /** BUG2-08: surface embedder cold-start. Same pattern as Recall.tsx. */
  const [busyElapsedMs, setBusyElapsedMs] = useState(0);
  const [result, setResult] = useState<SignMemoryResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Pick up newly-arrived selection from the content script. Updating
  // local state only when the incoming value is non-empty avoids
  // clobbering edits the user already made.
  useEffect(() => {
    if (prefilledSelection && selection === "") {
      setSelection(prefilledSelection);
    }
    // selection intentionally excluded — we only want to seed on
    // prefilled-change, not echo back into ourselves.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [prefilledSelection]);

  // BUG2-08: tick `busyElapsedMs` every 500 ms while busy so the
  // cold-start hint appears after 5s. Same shape as Recall.tsx.
  useEffect(() => {
    if (!busy) {
      setBusyElapsedMs(0);
      return;
    }
    const start = Date.now();
    const id = setInterval(() => {
      setBusyElapsedMs(Date.now() - start);
    }, 500);
    return () => clearInterval(id);
  }, [busy]);

  const handleSaveSelection = async (): Promise<void> => {
    setError(null);
    setResult(null);
    const trimmed = selection.trim();
    if (trimmed === "") {
      setError("Add something to capture first.");
      return;
    }
    setBusy(true);
    try {
      const tags = parseTags(tagsRaw, adapter);
      const runtime = getRuntime();
      const res = await runtime.signMemory({
        content: trimmed,
        tags,
        ...(adapter
          ? {
              source: {
                platform: adapter.platform,
              },
            }
          : {}),
      });
      await runtime.signRemote({ content: trimmed, tags, ...res });
      setResult(res);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSaveChat = useCallback(async (): Promise<void> => {
    if (!adapter) return;
    setError(null);
    setResult(null);
    setBusy(true);
    try {
      const runtime = getRuntime();
      const turns = await runtime.getActiveTabConversation();
      if (!turns || turns.length === 0) {
        setError("Could not read this chat. Open the conversation tab first.");
        setBusy(false);
        return;
      }
      const transcript = turns
        .map((t) => `### ${t.role}\n\n${t.content}`)
        .join("\n\n");
      const tags = parseTags(tagsRaw, adapter);
      const res = await runtime.signMemory({
        content: transcript,
        tags,
        source: { platform: adapter.platform },
      });
      await runtime.signRemote({ content: transcript, tags, ...res });
      setResult(res);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }, [adapter, tagsRaw]);

  // PR134-BLK-3 / BUG-03: react to a FAB-driven "save-chat" intent by
  // auto-triggering the conversation extraction + sign flow. Selection
  // text is already merged into `prefilledSelection` by the parent.
  // Guarded by `consumedRef` so a re-render after `setBusy` does not
  // re-fire the same intent. `onFabIntentConsumed` is called as soon as
  // we observe the intent so the parent can clear it from state and
  // future renders see `fabIntent === null`.
  const consumedRef = useRef<FabIntent | null>(null);
  useEffect(() => {
    if (!fabIntent) return;
    if (consumedRef.current === fabIntent) return;
    consumedRef.current = fabIntent;
    if (fabIntent.kind === "save-chat" && adapter) {
      void handleSaveChat();
    }
    onFabIntentConsumed?.();
  }, [fabIntent, adapter, handleSaveChat, onFabIntentConsumed]);

  return (
    <div className="flex flex-col gap-3">
      {adapter ? (
        <div className="flex items-center gap-2 text-[10px] uppercase tracking-wide text-text-muted">
          <span>Detected platform:</span>
          <span className="text-accent-primary font-mono">
            {adapter.platform}
          </span>
        </div>
      ) : (
        <div className="text-[10px] uppercase tracking-wide text-text-muted">
          Selection capture available via activeTab.
        </div>
      )}

      <label className="flex flex-col gap-1 text-xs" htmlFor="capture-content">
        <span className="text-text-muted">Selection / content</span>
        <textarea
          id="capture-content"
          value={selection}
          onChange={(e) => setSelection(e.target.value)}
          rows={6}
          className="bg-black/40 border border-white/10 rounded p-2 text-xs font-mono focus:outline-none focus:border-accent-primary resize-none"
          placeholder="Paste or capture the snippet to sign…"
        />
      </label>

      <label className="flex flex-col gap-1 text-xs">
        <span className="text-text-muted">Tags (comma-separated)</span>
        <input
          value={tagsRaw}
          onChange={(e) => setTagsRaw(e.target.value)}
          type="text"
          aria-label="Tags"
          className="bg-black/40 border border-white/10 rounded p-2 text-xs font-mono focus:outline-none focus:border-accent-primary"
          placeholder="research, q4-2026"
        />
      </label>

      {busy && busyElapsedMs > 5000 ? (
        <div
          role="status"
          aria-live="polite"
          className="text-[11px] text-text-muted font-mono italic"
        >
          {busyElapsedMs > 30000
            ? "First-time setup: downloading embedding model (~25 MB). This can take up to 3 minutes on a slow connection."
            : "Loading embedding model — first sign / recall after install takes a moment."}
        </div>
      ) : null}

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handleSaveSelection}
          disabled={busy}
          className="flex-1 bg-accent-primary/20 hover:bg-accent-primary/30 disabled:opacity-50 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          {busy ? "Signing…" : "Sign"}
        </button>
        <button
          type="button"
          onClick={handleSaveChat}
          disabled={busy || !adapter}
          title={
            adapter
              ? "Capture the full conversation on the active tab"
              : "Open a supported AI-chat page first"
          }
          className="flex-1 bg-white/5 hover:bg-white/10 disabled:opacity-30 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Save chat
        </button>
      </div>

      {result ? (
        <Toast
          kind="success"
          message={`Signed → ${truncate(result.attestation_id, 18)}`}
          copyValue={result.attestation_id}
          onDismiss={() => setResult(null)}
        />
      ) : null}
      {error ? (
        <Toast kind="error" message={error} onDismiss={() => setError(null)} />
      ) : null}
    </div>
  );
}

function parseTags(raw: string, adapter: ChatAdapter | null): string[] {
  const explicit = raw
    .split(/[\s,]+/)
    .map((t) => t.trim())
    .filter(Boolean);
  const auto: string[] = [];
  if (adapter) auto.push(`source:${adapter.platform}`);
  return Array.from(new Set([...auto, ...explicit]));
}

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max - 1)}…`;
}

function formatError(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

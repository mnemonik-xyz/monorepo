// Capture tab — "Save chat" / "Save selection". Both flows funnel into
// the same `runtime.signMemory` call; the difference is just the
// content source (full conversation transcript vs. textarea selection).
//
// Per D8 + D11 the popup never assumes `<all_urls>`. Selection capture
// only works when the active tab matches an adapter (host_permission)
// or when the user has already triggered an `activeTab` gesture — the
// content script delivers the selection text via `chrome.runtime.send-
// Message`, this tab just reads what arrived.

import { useEffect, useState, type JSX } from "react";
import type { ChatAdapter } from "../../runtime/chat/types.js";
import { getRuntime, type SignMemoryResult } from "../runtime.js";
import { Toast } from "../components/Toast.js";

export interface CaptureProps {
  adapter: ChatAdapter | null;
  prefilledSelection: string;
}

export function Capture(props: CaptureProps): JSX.Element {
  const { adapter, prefilledSelection } = props;
  const [selection, setSelection] = useState(prefilledSelection);
  const [tagsRaw, setTagsRaw] = useState("");
  const [busy, setBusy] = useState(false);
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

  /**
   * T18 round-2: `runtime.signRemote` attempts a live push first
   * (Cloud tier) and only falls through to the alarm-drain queue on
   * transient failure. Surface the three error types distinctly:
   *   - `ReauthRequiredError`: runtime already dispatched the global
   *     `mnemonik:re-auth-required` event; App.tsx re-routes to
   *     onboarding. Local sign already succeeded — keep the success
   *     toast, no extra error noise.
   *   - `PermanentSyncError`: row was marked `sync_failed_permanent`
   *     by the runtime. Local sign succeeded — surface "Cloud sync
   *     rejected" alongside the success toast.
   *   - `TransientSyncError` / unknown: runtime swallowed it and left
   *     the row queued. No UI noise; drain retries on the next tick.
   */
  const surfaceRemoteError = async (
    err: unknown,
    res: SignMemoryResult,
  ): Promise<void> => {
    setResult(res);
    if (err === null || err === undefined) return;
    const cc = await import("../../runtime/sync/cloud-client.js");
    if (err instanceof cc.PermanentSyncError) {
      setError(`Cloud sync rejected: ${err.message}`);
      return;
    }
    // ReauthRequiredError + TransientSyncError + unknown: no toast.
  };

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
      // Detach the local success from the remote-push outcome — the
      // local sign already succeeded, the cloud push is best-effort.
      let remoteErr: unknown = null;
      try {
        await runtime.signRemote({ content: trimmed, tags, ...res });
      } catch (e) {
        remoteErr = e;
      }
      await surfaceRemoteError(remoteErr, res);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSaveChat = async (): Promise<void> => {
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
      let remoteErr: unknown = null;
      try {
        await runtime.signRemote({ content: transcript, tags, ...res });
      } catch (e) {
        remoteErr = e;
      }
      await surfaceRemoteError(remoteErr, res);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

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

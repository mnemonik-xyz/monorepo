// Recall tab — query input + result list. Hits `runtime.recall` which
// runs cosine search over the IndexedDB store under the current
// identity. "Insert into chat" is disabled when the active-tab adapter
// reports `supportsInsert: false` (T11 TDD anchor #2). "Copy markdown"
// + "Open" are always available.

import { useState, type JSX } from "react";
import type { ChatAdapter } from "../../runtime/chat/types.js";
import { getRuntime } from "../runtime.js";
import type { SearchResult } from "../../runtime/store/types.js";

export interface RecallProps {
  adapter: ChatAdapter | null;
}

export function Recall(props: RecallProps): JSX.Element {
  const { adapter } = props;
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [hasSearched, setHasSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Adapters declare insert support via a `supportsInsert: boolean`
  // flag (BLK-1 fix). Minifier-safe — no `Function.prototype.toString`
  // introspection.
  const insertSupported = Boolean(adapter?.supportsInsert);

  const handleSearch = async (): Promise<void> => {
    setError(null);
    const trimmed = query.trim();
    if (trimmed === "") {
      setResults([]);
      setHasSearched(false);
      return;
    }
    setBusy(true);
    try {
      const hits = await getRuntime().recall(trimmed, 5);
      setResults(hits);
      setHasSearched(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleInsert = async (text: string): Promise<void> => {
    if (!insertSupported) return;
    try {
      await chrome.runtime.sendMessage({
        type: "ui:insert-into-chat",
        payload: { text },
      });
    } catch (e) {
      // Surface the failure — the button is enabled, so a silent
      // no-op would be misleading. The toast lives in <Recall> via
      // the inline error block above the results list.
      setError(
        e instanceof Error
          ? `Insert failed — ${e.message}. Reload the chat tab and try again.`
          : "Insert failed — reload the chat tab and try again."
      );
    }
  };

  const handleCopy = async (md: string): Promise<void> => {
    await navigator.clipboard?.writeText(md);
  };

  return (
    <div className="flex flex-col gap-3">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void handleSearch();
        }}
        className="flex items-center gap-2"
      >
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          type="search"
          aria-label="Recall query"
          placeholder="What do you remember about…"
          className="flex-1 bg-black/40 border border-white/10 rounded px-2 py-1.5 text-xs font-mono focus:outline-none focus:border-accent-primary"
        />
        <button
          type="submit"
          disabled={busy}
          aria-busy={busy}
          className="bg-accent-primary/20 hover:bg-accent-primary/30 disabled:opacity-50 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded transition-colors"
        >
          {busy ? "Recalling" : "Find"}
        </button>
      </form>

      {error ? (
        <div className="text-xs text-error font-mono">{error}</div>
      ) : null}

      <ul className="flex flex-col gap-2" aria-label="Recall results">
        {hasSearched && results.length === 0 && !busy ? (
          <li className="text-[11px] text-text-muted italic">
            Recall returned no results for this query.
          </li>
        ) : null}
        {results.map((r) => (
          <li
            key={r.attestation_id}
            className="border border-white/10 rounded p-2 flex flex-col gap-1.5"
          >
            <div className="flex items-center gap-2 text-[10px] uppercase tracking-wide">
              <span className="text-accent-primary font-mono">
                {r.relevance_score.toFixed(3)}
              </span>
              <PlatformPill tags={r.tags} />
              <span
                className="text-text-muted font-mono ml-auto"
                title={r.attestation_id}
                aria-label={`Attestation ID: ${r.attestation_id}`}
              >
                {truncateMiddle(r.attestation_id, 6, 4)}
              </span>
            </div>
            <p className="text-xs text-text-primary line-clamp-3">
              {r.content}
            </p>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => void handleCopy(toMarkdown(r))}
                className="text-[10px] uppercase tracking-wide font-mono text-text-muted hover:text-accent-primary border border-white/10 hover:border-accent-primary/50 rounded px-2 py-0.5 transition-colors"
              >
                Copy markdown
              </button>
              <button
                type="button"
                onClick={() => void handleInsert(r.content)}
                disabled={!insertSupported}
                title={
                  insertSupported
                    ? "Insert this memory at the chat input"
                    : "This platform does not support insert-into-chat yet"
                }
                aria-label="Insert into chat"
                className="text-[10px] uppercase tracking-wide font-mono text-text-muted hover:text-accent-primary border border-white/10 hover:border-accent-primary/50 disabled:opacity-40 disabled:hover:text-text-muted disabled:hover:border-white/10 rounded px-2 py-0.5 transition-colors"
              >
                Insert into chat
              </button>
              <a
                href={`https://mnemonik.xyz/m/${encodeURIComponent(
                  r.attestation_id
                )}`}
                target="_blank"
                rel="noreferrer noopener"
                className="text-[10px] uppercase tracking-wide font-mono text-text-muted hover:text-accent-primary border border-white/10 hover:border-accent-primary/50 rounded px-2 py-0.5 transition-colors"
              >
                Open
              </a>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function PlatformPill({
  tags,
}: {
  tags: readonly string[];
}): JSX.Element | null {
  const source = tags.find((t) => t.startsWith("source:"));
  if (!source) return null;
  const platform = source.slice("source:".length);
  return (
    <span className="text-[10px] uppercase tracking-wide font-mono text-text-muted border border-white/10 rounded px-1.5 py-0">
      {platform}
    </span>
  );
}

function toMarkdown(r: SearchResult): string {
  const tagLine = r.tags.length ? `tags: [${r.tags.join(", ")}]\n` : "";
  return `---\nattestation_id: ${r.attestation_id}\ncreated_at: ${
    r.created_at
  }\nscore: ${r.relevance_score.toFixed(4)}\n${tagLine}---\n\n${r.content}\n`;
}

function truncateMiddle(value: string, head: number, tail: number): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}

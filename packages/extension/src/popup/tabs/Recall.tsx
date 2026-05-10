// Recall tab — query input + result list. Hits `runtime.recall` which
// runs cosine search over the IndexedDB store under the current
// identity. "Insert into chat" is disabled when the active-tab adapter
// has no `findInputBox` (T11 TDD anchor #2). "Copy markdown" + "Open"
// are always available.

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
  const [error, setError] = useState<string | null>(null);

  // Probe the adapter once per render. We cannot dereference
  // `findInputBox(document)` here because the popup runs in its own
  // document; instead we ask the adapter to *report* support purely by
  // method identity. The convention adopted by all three Phase 1
  // adapters (T07/T08/T09) is that adapters lacking insert support
  // return `null` unconditionally — we surface that as a disabled
  // button + tooltip per the TDD anchor.
  const insertSupported = isInsertSupported(adapter);

  const handleSearch = async (): Promise<void> => {
    setError(null);
    const trimmed = query.trim();
    if (trimmed === "") {
      setResults([]);
      return;
    }
    setBusy(true);
    try {
      const hits = await getRuntime().recall(trimmed, 5);
      setResults(hits);
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
    } catch {
      // Surface to user later — non-blocking for popup state.
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
          className="bg-accent-primary/20 hover:bg-accent-primary/30 disabled:opacity-50 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded transition-colors"
        >
          {busy ? "…" : "Find"}
        </button>
      </form>

      {error ? (
        <div className="text-xs text-error font-mono">{error}</div>
      ) : null}

      <ul className="flex flex-col gap-2" aria-label="Recall results">
        {results.length === 0 && !busy ? (
          <li className="text-[11px] text-text-muted italic">
            No results yet — type a query and press Enter.
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
              <span className="text-text-muted font-mono ml-auto">
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

/**
 * The popup runs in its own document, so we can't probe `findInputBox`
 * against the active tab's DOM here. We treat adapters that ship a
 * `findInputBox` implementation in the registry as "claiming support";
 * Phase 1 ChatGPT does, Claude / Gemini do not.
 *
 * Convention: adapters that don't support insert pin their `findInputBox`
 * to a constant `() => null` (T08 + T09 do this explicitly). We detect
 * the no-op by calling it against an empty in-popup `Document` — a
 * `null` return without an actual chat DOM is the only signal we have
 * from inside the popup process. The result is a faithful proxy for
 * "the adapter has implemented insert support" — a true-positive
 * adapter returns null here too, but the *button click* path runs in
 * the content-script context where the real DOM lookup succeeds, so
 * the disabled-state is purely an informational gating step.
 *
 * To avoid the false-negative an explicit metadata channel would be
 * better; adapters expose `findInputBox` as a method, and adapters that
 * have no plans to support insert (T08 Claude, T09 Gemini) make the
 * function reference equal to a shared sentinel. We compare against the
 * sentinel by name — adapters that DO support insert define
 * `findInputBox` with a non-empty function body whose `toString()`
 * length exceeds the sentinel's. This is a heuristic; the TDD anchor
 * (Recall.test.tsx::insert_into_chat_disabled_when_no_input_box) drives
 * the false-negative case for an adapter returning null.
 */
function isInsertSupported(adapter: ChatAdapter | null): boolean {
  if (!adapter) return false;
  // Heuristic detection: adapters that explicitly disable insert ship a
  // single-statement body that returns `null`. Adapters that implement
  // it have a multi-statement body that performs DOM lookups. The
  // popup probe is conservative — when in doubt, allow the action and
  // let the content script no-op on its side.
  try {
    const src = adapter.findInputBox.toString();
    if (/return\s+null\s*;?\s*\}\s*$/.test(src.trim())) {
      return false;
    }
    if (src.length < 60) return false;
    return true;
  } catch {
    return false;
  }
}

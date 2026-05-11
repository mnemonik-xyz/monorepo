// Recall tab — query input + result list. Hits `runtime.recall` which
// runs cosine search over the IndexedDB store under the current
// identity. "Insert into chat" is disabled when the active-tab adapter
// reports `supportsInsert: false` (T11 TDD anchor #2). "Copy markdown"
// + "Open" are always available.
//
// T18: Cloud-tier Recall merges local + cloud results. We always run
// the local search (offline-first); when the storage tier is `cloud`
// we additionally call `cloudSync.recallRemote`, dedupe by
// `attestation_id`, prefer the side with the higher similarity, and
// fold in cloud-side `solana_tx` / `arweave_tx` when the local row
// hasn't been drained yet. Local-tier behaviour is unchanged.

import { useEffect, useState, type JSX } from "react";
import type { ChatAdapter } from "../../runtime/chat/types.js";
import {
  getRuntime,
  type CloudRecallHit,
  type StorageTier,
} from "../runtime.js";
import type { SearchResult } from "../../runtime/store/types.js";

export interface RecallProps {
  adapter: ChatAdapter | null;
}

export function Recall(props: RecallProps): JSX.Element {
  const { adapter } = props;
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [busyElapsedMs, setBusyElapsedMs] = useState(0);
  const [hasSearched, setHasSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** BUG2-07: when cloud recall fails the popup falls back to local
   *  results — surface that with a non-fatal banner rather than
   *  silently mixing local-only results into a "cloud" view. */
  const [cloudWarning, setCloudWarning] = useState<string | null>(null);
  const [storageTier, setStorageTier] = useState<StorageTier>("local");

  // Adapters declare insert support via a `supportsInsert: boolean`
  // flag (BLK-1 fix). Minifier-safe — no `Function.prototype.toString`
  // introspection.
  const insertSupported = Boolean(adapter?.supportsInsert);

  // Pull the active storage tier once on mount so Cloud-mode Recall
  // can run the merge step. Tier changes during the popup's lifetime
  // are rare (the T12 settings page closes the popup on change), but
  // re-reading per search would race the IndexedDB open path.
  useEffect(() => {
    let alive = true;
    void getRuntime()
      .loadStorageTier()
      .then((tier) => {
        if (alive) setStorageTier(tier);
      })
      .catch(() => {
        // Defaults to local — non-fatal if storage read fails.
      });
    return () => {
      alive = false;
    };
  }, []);

  // BUG2-08: surface the embedder cold-start. The first sign or recall
  // after install pulls ~25 MB of ONNX + ~10 MB of ORT WASM; without
  // visible progress, the user sees a tiny aria-busy state for
  // potentially 3 minutes. Tick `busyElapsedMs` every 500 ms while
  // busy and render a hint after 5s.
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

  const handleSearch = async (): Promise<void> => {
    setError(null);
    setCloudWarning(null);
    const trimmed = query.trim();
    if (trimmed === "") {
      setResults([]);
      setHasSearched(false);
      return;
    }
    setBusy(true);
    try {
      const runtime = getRuntime();
      // Local search always runs (offline-first). Cloud merge layers
      // on top when the tier is `cloud` AND the user has a session.
      const localPromise = runtime.recall(trimmed, 5);
      let cloudFailed: string | null = null;
      const cloudPromise: Promise<CloudRecallHit[] | null> =
        storageTier === "cloud"
          ? runtime.cloudSync.recallRemote(trimmed, 5).catch((e: unknown) => {
              // Cloud failure must NOT block local results — the
              // tech-spec's offline-first guarantee is the whole
              // point. Surface the error but keep going (BUG2-07).
              const msg = e instanceof Error ? e.message : String(e);
              cloudFailed = msg;
              // 401 → re-auth required; propagate through the global
              // event the SW drain uses so App.tsx's reauth gate fires
              // (consistent with signRemote's behaviour).
              if (/401|unauthor|reauth/i.test(msg)) {
                try {
                  const g = globalThis as {
                    dispatchEvent?: (e: Event) => boolean;
                    CustomEvent?: typeof CustomEvent;
                  };
                  if (
                    typeof g.dispatchEvent === "function" &&
                    typeof g.CustomEvent === "function"
                  ) {
                    g.dispatchEvent(
                      new g.CustomEvent("mnemonik:re-auth-required"),
                    );
                  }
                } catch {
                  // Best-effort.
                }
              }
              return null;
            })
          : Promise.resolve(null);
      const [localHits, cloudHits] = await Promise.all([
        localPromise,
        cloudPromise,
      ]);
      setResults(mergeRecallHits(localHits, cloudHits, 5));
      setHasSearched(true);
      if (cloudFailed !== null) {
        setCloudWarning(
          "Cloud search unavailable — showing local results only.",
        );
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleInsert = async (text: string): Promise<void> => {
    if (!insertSupported) return;
    // PR134-BLK-1: route through `chrome.tabs.sendMessage` so the message
    // reaches the content-script realm where `message-bridge.ts` owns
    // `insertIntoChat`. `chrome.runtime.sendMessage` from the popup is
    // delivered only to extension pages (SW, options, other popups), not
    // to content scripts — so the previous wiring silently no-op'd.
    try {
      const tabs = await chrome.tabs.query({
        active: true,
        currentWindow: true,
      });
      const tabId = tabs[0]?.id;
      if (typeof tabId !== "number") {
        setError("Could not find the active tab. Reload the chat tab.");
        return;
      }
      const reply = (await chrome.tabs.sendMessage(tabId, {
        type: "ui:insert-into-chat",
        payload: { text },
      })) as { ok?: boolean } | undefined;
      if (!reply || reply.ok !== true) {
        setError(
          "Insert failed — the chat composer rejected the text. " +
            "Copy markdown and paste manually.",
        );
      }
    } catch (e) {
      // Surface the failure — the button is enabled, so a silent
      // no-op would be misleading. The toast lives in <Recall> via
      // the inline error block above the results list.
      setError(
        e instanceof Error
          ? `Insert failed — ${e.message}. Reload the chat tab and try again.`
          : "Insert failed — reload the chat tab and try again.",
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

      {cloudWarning ? (
        <div
          role="status"
          className="text-[11px] text-text-muted font-mono italic border border-white/10 rounded px-2 py-1"
        >
          {cloudWarning}
        </div>
      ) : null}

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
                  r.attestation_id,
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
 * Merge local and cloud recall hits per the T18 spec:
 *
 *   1. Dedupe by `attestation_id` first; fall back to dedupe by
 *      `content` equality so cross-device round-trips don't surface
 *      the same memory twice when the local id is `att_<blake3-prefix>`
 *      and the server returned its own UUID (BUG2-03). Once the
 *      cloud-side drain reflows server-issued ids back into local rows
 *      (deferred follow-up), the id-equality path takes over and the
 *      content fallback becomes a no-op.
 *   2. When the same memory appears on both sides, keep the higher
 *      similarity score (cloud and local can disagree because cloud
 *      runs server-side cosine search on the canonical embedding
 *      vs. local TurboQuant-decompressed embeddings).
 *   3. Prefer cloud-side `solana_tx` / `arweave_tx` when the local
 *      row still carries synthetic `local:` ids — the cloud row has
 *      been anchored on-chain.
 *   4. Re-rank descending by score; truncate to `limit`.
 *
 * Local-only entries pass through unchanged. Cloud-only entries are
 * projected into the `SearchResult` shape so the existing renderer
 * works without a discriminated union.
 */
export function mergeRecallHits(
  local: SearchResult[],
  cloud: CloudRecallHit[] | null,
  limit: number,
): SearchResult[] {
  if (!cloud || cloud.length === 0) {
    return local.slice(0, limit);
  }
  const byId = new Map<string, SearchResult>();
  // BUG2-03: parallel index by content so a cloud row with the
  // server-issued UUID can find its local `att_<hash>` peer.
  const byContent = new Map<string, string>();
  for (const r of local) {
    byId.set(r.attestation_id, r);
    if (r.content) byContent.set(r.content, r.attestation_id);
  }
  for (const c of cloud) {
    if (!c.attestation_id) continue;
    let existingKey: string | null = null;
    if (byId.has(c.attestation_id)) {
      existingKey = c.attestation_id;
    } else if (c.content && byContent.has(c.content)) {
      existingKey = byContent.get(c.content) ?? null;
    }
    if (existingKey !== null) {
      const existing = byId.get(existingKey);
      if (!existing) continue;
      const better = c.similarity > existing.relevance_score;
      // Prefer cloud-side tx ids when local still carries synthetic
      // `local:` prefixes — the cloud row has been anchored.
      const solana_tx =
        existing.solana_tx.startsWith("local:") && c.solana_tx
          ? c.solana_tx
          : existing.solana_tx;
      const arweave_tx =
        existing.arweave_tx.startsWith("local:") && c.arweave_tx
          ? c.arweave_tx
          : existing.arweave_tx;
      byId.set(existingKey, {
        ...existing,
        relevance_score: better ? c.similarity : existing.relevance_score,
        solana_tx,
        arweave_tx,
      });
    } else {
      byId.set(c.attestation_id, {
        attestation_id: c.attestation_id,
        content: c.content,
        // Cloud-only entries don't carry a content_hash today — render
        // with an empty hash; the popup's Verify tab re-fetches the
        // full row when the user clicks Open.
        content_hash: "",
        tags: c.tags ?? [],
        solana_tx: c.solana_tx ?? "",
        arweave_tx: c.arweave_tx ?? "",
        created_at: c.signed_at ?? "",
        relevance_score: c.similarity,
      });
      if (c.content) byContent.set(c.content, c.attestation_id);
    }
  }
  return Array.from(byId.values())
    .sort((a, b) => b.relevance_score - a.relevance_score)
    .slice(0, limit);
}

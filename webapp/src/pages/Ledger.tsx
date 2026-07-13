import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { Seo } from "../lib/seo";
import {
  fetchArtifacts,
  type Artifact,
  type ArtifactPage,
  type ArtifactSource,
  type WriteMode,
} from "../lib/ledger";
import { irysDataUrl, solanaTxUrl } from "../lib/links";

/**
 * Ledger page (`/ledger`).
 *
 * A forensic feed of recalled artifacts saved on the node — the protocol's
 * public "proof of work": real signed memories, their blake3 hashes, and their
 * on-chain anchors, rendered as receipt/evidence cards.
 *
 * The page never lies about provenance: data comes from the live `/artifacts`
 * endpoint and a fetch failure shows an error state — never fabricated rows.
 * On-chain rows ("participate") link to Solana and the Irys data gateway; `local:`
 * / unanchored rows render as plain text, never as links (Decision 6 surfaces
 * public rows only).
 */

type ModeFilter = "all" | WriteMode;
type Status = "loading" | "ready" | "error";

// The public endpoint defaults to 50 rows. Chain recovery can legitimately
// contribute more than that, so use the server's bounded maximum for the
// Ledger; otherwise recovered anchors can fill the first page before any
// on-node memories reach the client-side mode filter.
const LEDGER_PAGE_LIMIT = 200;

export default function Ledger() {
  // `query` is the committed search term that drives the fetch; `draft` is the
  // controlled input value awaiting submit.
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<ModeFilter>("all");
  const [page, setPage] = useState<ArtifactPage | null>(null);
  const [status, setStatus] = useState<Status>("loading");

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    const fetchPage = (): Promise<ArtifactPage> => {
      const common = { q: query, limit: LEDGER_PAGE_LIMIT };
      if (mode !== "all") {
        const source: ArtifactSource =
          mode === "local" ? "on_node" : "on_chain";
        return fetchArtifacts({ ...common, source });
      }
      return Promise.all([
        fetchArtifacts({ ...common, source: "on_node" }),
        fetchArtifacts({ ...common, source: "on_chain" }),
      ]).then(([onNode, onChain]) => ({
        artifacts: [...onNode.artifacts, ...onChain.artifacts].sort((a, b) =>
          b.created_at.localeCompare(a.created_at),
        ),
        total: onNode.total + onChain.total,
      }));
    };

    fetchPage()
      .then((res) => {
        if (cancelled) return;
        setPage(res);
        setStatus("ready");
      })
      .catch(() => {
        if (cancelled) return;
        setPage(null);
        setStatus("error");
      });
    return () => {
      cancelled = true;
    };
  }, [query, mode]);

  const artifacts = page?.artifacts ?? [];
  const visible =
    mode === "all" ? artifacts : artifacts.filter((a) => a.write_mode === mode);

  return (
    <div className="relative min-h-screen overflow-hidden">
      <Seo
        title="Ledger"
        description="A forensic feed of recalled artifacts saved on the node — signed memories, blake3 hashes, Solana anchors, and Irys data receipts."
        canonical="/ledger"
      />
      <SiteHeader />

      <main className="mx-auto max-w-5xl px-4 py-16 sm:px-6 md:py-20">
        <PageHeader />

        <Controls
          draft={draft}
          onDraftChange={setDraft}
          onSubmit={() => setQuery(draft.trim())}
          mode={mode}
          onModeChange={setMode}
        />

        <div className="mt-8">
          {status === "loading" && <LoadingState />}
          {status === "error" && <ErrorState />}
          {status === "ready" && visible.length === 0 && <EmptyState />}
          {status === "ready" && visible.length > 0 && (
            <ul
              role="list"
              aria-label="Recalled artifacts on the node"
              className="space-y-5"
            >
              {visible.map((a) => (
                <li key={a.id}>
                  <ArtifactCard artifact={a} />
                </li>
              ))}
            </ul>
          )}
        </div>
      </main>

      <SiteFooter />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                  Header                                     */
/* -------------------------------------------------------------------------- */

function PageHeader() {
  return (
    <header className="mb-10 space-y-4">
      <Link
        to="/"
        className="inline-flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted transition-colors hover:text-accent-primary"
      >
        <span aria-hidden="true">←</span> Back
      </Link>
      <p className="font-mono text-[11px] uppercase tracking-[0.22em] text-accent-primary">
        Evidence ledger
      </p>
      <h1 className="text-balance text-4xl font-bold leading-tight tracking-tight text-text-primary sm:text-5xl">
        Recalled artifacts{" "}
        <span className="font-display italic text-accent-primary">
          on the node
        </span>
        .
      </h1>
      <p className="max-w-2xl text-text-muted">
        Public, signed memories saved on the node. Each carries a blake3 content
        hash and — when minted on-chain — a Solana anchor plus independently
        retrievable signed bytes on Irys.
      </p>
    </header>
  );
}

/* -------------------------------------------------------------------------- */
/*                          Search + write_mode filter                        */
/* -------------------------------------------------------------------------- */

const MODE_CHIPS: Array<{ value: ModeFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "local", label: "On-node" },
  { value: "participate", label: "On-chain" },
];

function Controls({
  draft,
  onDraftChange,
  onSubmit,
  mode,
  onModeChange,
}: {
  draft: string;
  onDraftChange: (v: string) => void;
  onSubmit: () => void;
  mode: ModeFilter;
  onModeChange: (m: ModeFilter) => void;
}) {
  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
      <form
        role="search"
        className="flex w-full max-w-md items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit();
        }}
      >
        <input
          type="search"
          aria-label="Recall by meaning"
          placeholder="Recall by meaning…"
          value={draft}
          onChange={(e) => onDraftChange(e.target.value)}
          className="w-full rounded-md border border-white/10 bg-white/[0.03] px-3 py-2 font-mono text-sm text-text-primary placeholder:text-text-faint focus-visible:border-accent-primary focus-visible:outline-none"
        />
        <button
          type="submit"
          className="shrink-0 rounded-md border border-accent-primary/40 bg-accent-primary/10 px-3 py-2 font-mono text-[11px] uppercase tracking-[0.16em] text-accent-primary transition-colors hover:border-accent-primary hover:bg-accent-primary/20"
        >
          Recall
        </button>
      </form>

      <div
        role="group"
        aria-label="Filter by write mode"
        className="flex items-center gap-1.5"
      >
        {MODE_CHIPS.map((chip) => {
          const active = chip.value === mode;
          return (
            <button
              key={chip.value}
              type="button"
              aria-pressed={active}
              onClick={() => onModeChange(chip.value)}
              className={`rounded-full border px-3 py-1 font-mono text-[10px] uppercase tracking-[0.16em] transition-colors ${
                active
                  ? "border-accent-primary/60 bg-accent-primary/15 text-accent-primary"
                  : "border-white/10 text-text-muted hover:border-white/25 hover:text-text-primary"
              }`}
            >
              {chip.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                Artifact card                               */
/* -------------------------------------------------------------------------- */

function ArtifactCard({ artifact: a }: { artifact: Artifact }) {
  const [expanded, setExpanded] = useState(false);
  const onChain = a.write_mode === "participate";
  const solUrl = solanaTxUrl(a.solana_tx);
  const dataUrl = irysDataUrl(a.arweave_tx);

  return (
    <article className="relative overflow-hidden rounded-md border border-white/10 bg-panel/60 p-5 transition-colors hover:border-accent-primary/30">
      <div className="grain pointer-events-none absolute inset-0 -z-10 opacity-50" />

      <div className="flex items-start justify-between gap-3">
        <ModeBadge mode={a.write_mode} />
        <div className="flex items-center gap-3">
          <time
            dateTime={a.created_at}
            className="font-mono text-[11px] tabular-nums text-text-faint"
          >
            {formatDate(a.created_at)}
          </time>
          {onChain && <SealStamp />}
        </div>
      </div>

      <p
        className={`mt-4 whitespace-pre-wrap text-[15px] leading-relaxed text-text-primary ${
          expanded ? "" : "line-clamp-6"
        }`}
      >
        {a.content}
      </p>

      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((value) => !value)}
        className="mt-2 font-mono text-[10px] uppercase tracking-[0.14em] text-accent-primary underline-offset-2 hover:underline"
      >
        {expanded ? "Collapse memory" : "Show full memory"}
      </button>

      <HashRow hash={a.content_hash} />

      {a.tags.length > 0 && (
        <ul className="mt-3 flex flex-wrap gap-1.5" aria-label="Tags">
          {a.tags.map((t) => (
            <li
              key={t}
              className="rounded-sm bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.12em] text-text-muted"
            >
              {t}
            </li>
          ))}
        </ul>
      )}

      <dl className="mt-4 grid grid-cols-1 gap-x-6 gap-y-2 border-t border-white/5 pt-3 sm:grid-cols-2">
        <AnchorRow label="Solana" tx={a.solana_tx} url={solUrl} />
        <AnchorRow label="Irys data" tx={a.arweave_tx} url={dataUrl} />
      </dl>
    </article>
  );
}

function ModeBadge({ mode }: { mode: WriteMode }) {
  if (mode === "participate") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-sm border border-accent-secondary/40 bg-accent-secondary/10 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.16em] text-accent-secondary">
        On-chain
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-sm border border-accent-primary/40 bg-accent-primary/10 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.16em] text-accent-primary">
      On-node mint
    </span>
  );
}

function SealStamp() {
  return (
    <span
      aria-label="Sealed on-chain"
      className="animate-stamp select-none rounded-sm border border-stamp/50 px-1.5 py-0.5 font-mono text-[10px] font-bold uppercase tracking-[0.2em] text-stamp"
    >
      Sealed
    </span>
  );
}

function HashRow({ hash }: { hash: string }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(hash);
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      }
    } catch {
      // Clipboard unavailable or denied — leave the hash visible to copy by hand.
    }
  }

  return (
    <div className="mt-3 flex items-center gap-2">
      <code className="min-w-0 flex-1 truncate rounded-sm bg-black/30 px-2 py-1 font-mono text-[11px] text-text-muted">
        {hash}
      </code>
      <button
        type="button"
        onClick={copy}
        aria-label="Copy content hash"
        className="shrink-0 rounded-sm border border-white/10 px-2 py-1 font-mono text-[10px] uppercase tracking-[0.14em] text-text-muted transition-colors hover:border-accent-primary hover:text-accent-primary focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent-primary"
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function AnchorRow({
  label,
  tx,
  url,
}: {
  label: string;
  tx: string | null;
  url: string | null;
}) {
  const display = tx ? truncateMiddle(tx, 10, 8) : "not anchored";

  return (
    <div className="flex min-w-0 items-baseline gap-2">
      <dt className="shrink-0 font-mono text-[10px] uppercase tracking-[0.16em] text-text-faint">
        {label}
      </dt>
      <dd className="min-w-0 flex-1 font-mono text-[11px]">
        {url ? (
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            title={tx ?? undefined}
            className="inline-flex max-w-full items-baseline gap-1.5 text-accent-primary underline-offset-2 hover:underline"
          >
            <span className="min-w-0 truncate">{display}</span>
            <span aria-hidden="true" className="shrink-0">
              ↗
            </span>
          </a>
        ) : (
          // `local:` / unanchored rows are NOT links — plain text only.
          <span className="text-text-faint">{display}</span>
        )}
      </dd>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                  States                                    */
/* -------------------------------------------------------------------------- */

function LoadingState() {
  return (
    <div
      data-testid="ledger-loading"
      role="status"
      aria-live="polite"
      className="space-y-5"
    >
      <span className="sr-only">Loading artifacts…</span>
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          aria-hidden="true"
          className="h-36 animate-pulse rounded-md border border-white/5 bg-white/[0.02]"
        />
      ))}
    </div>
  );
}

function EmptyState() {
  return (
    <div
      data-testid="ledger-empty"
      className="rounded-md border border-dashed border-white/10 px-6 py-16 text-center"
    >
      <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-faint">
        No artifacts
      </p>
      <p className="mt-2 text-sm text-text-muted">
        No public memories match this recall. Try a different query or clear the
        filter.
      </p>
    </div>
  );
}

function ErrorState() {
  return (
    <div
      data-testid="ledger-error"
      role="alert"
      className="rounded-md border border-error/30 bg-error/[0.06] px-6 py-12 text-center"
    >
      <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-error">
        Ledger unavailable
      </p>
      <p className="mt-2 text-sm text-text-muted">
        The ledger could not be loaded. Check your connection and try again.
      </p>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                  Helpers                                   */
/* -------------------------------------------------------------------------- */

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "2-digit",
  });
}

function truncateMiddle(value: string, start: number, end: number): string {
  if (value.length <= start + end + 1) return value;
  return `${value.slice(0, start)}...${value.slice(-end)}`;
}

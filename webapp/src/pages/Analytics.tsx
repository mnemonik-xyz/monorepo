import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import TimelineChart from "../components/TimelineChart";
import {
  fetchArtifacts,
  fetchAttestationTimeline,
  type Artifact,
  type AttestationTimeline,
  type TimeRange,
} from "../lib/ledger";
import { irysDataUrl, solanaTxUrl } from "../lib/links";
import { Seo } from "../lib/seo";

/**
 * `/analytics` — attestations over time (Scenario 2 of the Evidence Ledger
 * rethink). A bespoke SVG time-series splits on-node (mint) from on-chain
 * (Solana-purple), with a range toggle and summary counters.
 *
 * Data comes from the live `fetchAttestationTimeline` endpoint; a fetch failure
 * shows an error state, never fabricated data. Tone per ux-guidelines: factual,
 * no marketing, no emojis.
 */

const RANGES: { value: TimeRange; label: string; window: string }[] = [
  { value: "30d", label: "30d", window: "last 30 days" },
  { value: "90d", label: "90d", window: "last 90 days" },
  { value: "12m", label: "12m", window: "last 12 months" },
  { value: "all", label: "All", window: "all time" },
];

type Status = "loading" | "ready" | "error";

const numberFmt = new Intl.NumberFormat("en-US");

export default function Analytics() {
  const [range, setRange] = useState<TimeRange>("30d");
  const [data, setData] = useState<AttestationTimeline | null>(null);
  const [anchored, setAnchored] = useState<Artifact[]>([]);
  const [status, setStatus] = useState<Status>("loading");
  const [anchoredStatus, setAnchoredStatus] = useState<Status>("loading");

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    fetchAttestationTimeline(range)
      .then((result) => {
        // Last-write-wins: a newer range switch invalidates this response.
        if (cancelled) return;
        setData(result);
        setStatus("ready");
      })
      .catch(() => {
        if (cancelled) return;
        setStatus("error");
      });
    return () => {
      cancelled = true;
    };
  }, [range]);

  useEffect(() => {
    let cancelled = false;
    setAnchoredStatus("loading");
    fetchArtifacts({ limit: 24, source: "on_chain" })
      .then((page) => {
        if (cancelled) return;
        setAnchored(page.artifacts.filter(isAnchored).slice(0, 6));
        setAnchoredStatus("ready");
      })
      .catch(() => {
        if (cancelled) return;
        setAnchored([]);
        setAnchoredStatus("error");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const windowLabel =
    RANGES.find((r) => r.value === range)?.window ?? "selected range";

  return (
    <div className="relative min-h-screen bg-background text-text-primary">
      <Seo
        title="Analytics — attestations over time"
        description="Attestations recorded by the Mnemonic Protocol over time, split by on-node and on-chain anchoring. A live forensic view of the memory ledger."
        canonical="/analytics"
        jsonLd={{
          "@context": "https://schema.org",
          "@type": "Dataset",
          name: "Mnemonic Protocol attestation timeline",
          description:
            "Counts of agent-memory attestations over time, split into on-node and on-chain anchored records.",
          creator: { "@type": "Organization", name: "Mnemonic Protocol" },
        }}
      />
      <SiteHeader />

      <main className="mx-auto max-w-5xl px-4 py-10 sm:px-6 sm:py-14">
        <Link
          to="/"
          className="text-sm text-text-muted transition-colors hover:text-text-primary"
        >
          ← Back
        </Link>

        <header className="mt-8 space-y-3">
          <p className="font-mono text-[11px] uppercase tracking-[0.22em] text-accent-primary">
            Network analytics
          </p>
          <h1 className="text-balance text-4xl font-bold leading-tight tracking-tight text-text-primary sm:text-5xl">
            Attestations <span className="text-accent-primary">over time</span>.
          </h1>
          <p className="max-w-2xl text-text-muted">
            Every signed memory is an attestation. On-node records live on the
            operator&rsquo;s node; on-chain records are additionally anchored on
            Solana, with signed bytes retrievable from Irys. The split below
            tracks both across the{" "}
            {windowLabel}.
          </p>
        </header>

        <div className="mt-8 flex items-center justify-between gap-4">
          <h2 className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted">
            Range
          </h2>
          <RangeToggle range={range} onChange={setRange} />
        </div>

        <section className="mt-4">
          <ChartPanel status={status} data={data} windowLabel={windowLabel} />
        </section>

        <SummaryGrid data={data} />

        <AnchoredReferences status={anchoredStatus} artifacts={anchored} />

        <Legend />
      </main>

      <SiteFooter />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                           Anchored references                              */
/* -------------------------------------------------------------------------- */

function AnchoredReferences({
  status,
  artifacts,
}: {
  status: Status;
  artifacts: Artifact[];
}) {
  return (
    <section
      aria-labelledby="anchored-references"
      className="mt-8 border-t border-white/5 pt-6"
    >
      <div className="mb-4 flex items-center justify-between gap-4">
        <h2
          id="anchored-references"
          className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted"
        >
          Anchored records
        </h2>
        <Link
          to="/ledger"
          className="font-mono text-[11px] uppercase tracking-[0.16em] text-accent-primary underline-offset-2 hover:underline"
        >
          Ledger →
        </Link>
      </div>

      {status === "loading" && (
        <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted">
          Loading anchors…
        </p>
      )}
      {status === "error" && (
        <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-error">
          Could not load anchored records.
        </p>
      )}
      {status === "ready" && artifacts.length === 0 && (
        <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted">
          No anchored records available.
        </p>
      )}
      {status === "ready" && artifacts.length > 0 && (
        <ul role="list" className="grid gap-3 sm:grid-cols-2">
          {artifacts.map((artifact) => (
            <li key={artifact.id}>
              <AnchoredReference artifact={artifact} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function AnchoredReference({ artifact }: { artifact: Artifact }) {
  const solUrl = solanaTxUrl(artifact.solana_tx);
  const arUrl = irysDataUrl(artifact.arweave_tx);

  return (
    <article className="rounded-md border border-white/5 bg-white/[0.02] p-4">
      <p className="line-clamp-2 text-sm leading-relaxed text-text-primary">
        {artifact.content}
      </p>
      <dl className="mt-3 space-y-1.5">
        <ReferenceLink label="Solana" tx={artifact.solana_tx} url={solUrl} />
        <ReferenceLink label="Irys data" tx={artifact.arweave_tx} url={arUrl} />
      </dl>
    </article>
  );
}

function ReferenceLink({
  label,
  tx,
  url,
}: {
  label: string;
  tx: string | null;
  url: string | null;
}) {
  if (!url || !tx) return null;
  return (
    <div className="flex min-w-0 items-baseline gap-2">
      <dt className="shrink-0 font-mono text-[10px] uppercase tracking-[0.16em] text-text-faint">
        {label}
      </dt>
      <dd className="min-w-0 flex-1 font-mono text-[11px]">
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          title={tx}
          className="inline-flex max-w-full items-baseline gap-1.5 text-accent-primary underline-offset-2 hover:underline"
        >
          <span className="min-w-0 truncate">{truncateMiddle(tx, 10, 8)}</span>
          <span aria-hidden="true" className="shrink-0">
            ↗
          </span>
        </a>
      </dd>
    </div>
  );
}

function isAnchored(artifact: Artifact): boolean {
  return Boolean(
    solanaTxUrl(artifact.solana_tx) || irysDataUrl(artifact.arweave_tx),
  );
}

function truncateMiddle(value: string, start: number, end: number): string {
  if (value.length <= start + end + 1) return value;
  return `${value.slice(0, start)}...${value.slice(-end)}`;
}

/* -------------------------------------------------------------------------- */
/*                               Range toggle                                 */
/* -------------------------------------------------------------------------- */

function RangeToggle({
  range,
  onChange,
}: {
  range: TimeRange;
  onChange: (r: TimeRange) => void;
}) {
  return (
    <div
      role="group"
      aria-label="Select time range"
      className="inline-flex rounded-md border border-white/10 bg-white/[0.02] p-0.5"
    >
      {RANGES.map((r) => {
        const active = r.value === range;
        return (
          <button
            key={r.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(r.value)}
            className={`rounded-[5px] px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.18em] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary ${
              active
                ? "bg-accent-primary/15 text-accent-primary"
                : "text-text-muted hover:text-text-primary"
            }`}
          >
            {r.label}
          </button>
        );
      })}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                Chart panel                                 */
/* -------------------------------------------------------------------------- */

function ChartPanel({
  status,
  data,
  windowLabel,
}: {
  status: Status;
  data: AttestationTimeline | null;
  windowLabel: string;
}) {
  const panel = "rounded-md border border-white/5 bg-white/[0.02] p-4 sm:p-6";

  if (status === "loading" && !data) {
    return (
      <div className={`${panel} flex h-72 items-center justify-center`}>
        <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted">
          Loading timeline…
        </p>
      </div>
    );
  }

  if (status === "error") {
    return (
      <div className={`${panel} flex h-72 items-center justify-center`}>
        <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-error">
          Could not load the attestation timeline.
        </p>
      </div>
    );
  }

  if (!data || data.buckets.length === 0) {
    return (
      <div className={`${panel} flex h-72 items-center justify-center`}>
        <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted">
          No attestations recorded in this range.
        </p>
      </div>
    );
  }

  const ariaLabel = `Attestations over the ${windowLabel}: ${data.total_on_node} on node, ${data.total_on_chain} on chain.`;

  return (
    <div className={panel} data-testid="timeline-chart">
      <TimelineChart buckets={data.buckets} ariaLabel={ariaLabel} />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                  Summary                                   */
/* -------------------------------------------------------------------------- */

function SummaryGrid({ data }: { data: AttestationTimeline | null }) {
  return (
    <dl className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-3">
      <Stat
        testId="stat-on-node"
        label="On node"
        value={data?.total_on_node}
        caption="Saved"
        tone="primary"
      />
      <Stat
        testId="stat-on-chain"
        label="On chain"
        value={data?.total_on_chain}
        caption="Anchored"
        tone="secondary"
      />
      <Stat
        testId="stat-users"
        label="Owners"
        value={data?.unique_users}
        caption="Distinct"
        tone="muted"
      />
    </dl>
  );
}

function Stat({
  testId,
  label,
  value,
  caption,
  tone,
}: {
  testId: string;
  label: string;
  value: number | undefined;
  caption: string;
  tone: "primary" | "secondary" | "muted";
}) {
  const accent =
    tone === "primary"
      ? "text-accent-primary/80"
      : tone === "secondary"
        ? "text-accent-secondary/80"
        : "text-text-muted";
  return (
    <div className="flex flex-col gap-1 rounded-md border border-white/5 bg-white/[0.02] px-4 py-3">
      <dt className="font-mono text-[10px] uppercase tracking-[0.18em] text-text-muted">
        {label}
      </dt>
      <dd
        data-testid={testId}
        className="font-mono text-2xl font-semibold tabular-nums tracking-tight text-text-primary"
      >
        {typeof value === "number" ? numberFmt.format(value) : "—"}
      </dd>
      <span
        className={`font-mono text-[10px] uppercase tracking-[0.16em] ${accent}`}
      >
        {caption}
      </span>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                              Legend + banner                               */
/* -------------------------------------------------------------------------- */

function Legend() {
  return (
    <div className="mt-6 flex flex-wrap items-center gap-x-6 gap-y-2 font-mono text-[11px] uppercase tracking-[0.16em] text-text-muted">
      <span className="inline-flex items-center gap-2">
        <span
          aria-hidden="true"
          className="inline-block h-0.5 w-5 bg-accent-primary"
        />
        On node
      </span>
      <span className="inline-flex items-center gap-2">
        <span
          aria-hidden="true"
          className="inline-block h-0.5 w-5 bg-accent-secondary"
        />
        On chain
      </span>
    </div>
  );
}

import { Link } from "react-router-dom";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import {
  ROADMAP_ITEMS,
  type RoadmapItem,
  type RoadmapStatus,
} from "../lib/roadmap.generated";

const COLUMNS: { status: RoadmapStatus; title: string; subtitle: string }[] = [
  {
    status: "shipped",
    title: "Shipped",
    subtitle: "Merged to dev, in production",
  },
  {
    status: "in_progress",
    title: "In progress",
    subtitle: "Spec approved, tasks landing",
  },
  {
    status: "planned",
    title: "Planned",
    subtitle: "Drafted, not started",
  },
];

/**
 * `/roadmap` — three-column status board driven by work/<feature>/ specs.
 *
 * Items are generated at build time by `scripts/generate-roadmap.mjs` from
 * tech-spec.md + tasks/*.md frontmatter. Editing the page does not change
 * the items; edit the specs or re-run `npm run roadmap`.
 */
export default function Roadmap() {
  return (
    <div className="min-h-screen bg-background text-text-primary">
      <SiteHeader />
      <main className="px-4 py-10 sm:py-14">
        <div className="mx-auto max-w-6xl space-y-10">
          <header className="space-y-3">
            <Link
              to="/"
              className="text-sm text-text-muted transition-colors hover:text-text-primary"
            >
              ← Back
            </Link>
            <h1 className="text-3xl font-bold tracking-tight text-text-primary sm:text-4xl">
              Roadmap
            </h1>
            <p className="max-w-2xl text-sm leading-relaxed text-text-muted">
              Drawn directly from the spec-driven workflow in{" "}
              <code className="font-mono text-accent-primary">work/</code>. Each
              card maps to one feature folder; status is computed from its
              task-file frontmatter.
            </p>
          </header>

          <section
            aria-label="Roadmap status board"
            className="grid gap-6 md:grid-cols-3"
            data-testid="roadmap-board"
          >
            {COLUMNS.map((col) => (
              <RoadmapColumn
                key={col.status}
                title={col.title}
                subtitle={col.subtitle}
                status={col.status}
                items={ROADMAP_ITEMS.filter((i) => i.status === col.status)}
              />
            ))}
          </section>
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}

function RoadmapColumn({
  title,
  subtitle,
  status,
  items,
}: {
  title: string;
  subtitle: string;
  status: RoadmapStatus;
  items: readonly RoadmapItem[];
}) {
  return (
    <div
      data-testid={`roadmap-column-${status}`}
      className="flex flex-col gap-3"
    >
      <header className="border-b border-white/5 pb-2">
        <div className="flex items-baseline justify-between gap-2">
          <h2 className="font-mono text-xs uppercase tracking-[0.18em] text-accent-primary">
            {title}
          </h2>
          <span className="font-mono text-[11px] text-text-muted/70">
            {items.length}
          </span>
        </div>
        <p className="mt-1 text-[11px] text-text-muted/70">{subtitle}</p>
      </header>
      {items.length === 0 ? (
        <p className="font-mono text-[11px] text-text-muted/50">— nothing —</p>
      ) : (
        <ul className="flex flex-col gap-3">
          {items.map((item) => (
            <li key={item.id}>
              <RoadmapCard item={item} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function RoadmapCard({ item }: { item: RoadmapItem }) {
  return (
    <article className="rounded-lg border border-white/10 bg-white/[0.02] p-4 transition-colors hover:border-accent-primary/30">
      <h3 className="font-mono text-sm font-semibold tracking-tight text-text-primary">
        {item.title}
      </h3>
      <p className="mt-2 text-[13px] leading-relaxed text-text-muted">
        {item.description}
      </p>
      <p className="mt-3 font-mono text-[10px] uppercase tracking-[0.14em] text-text-muted/50">
        {item.sourcePath}
      </p>
    </article>
  );
}

// Options page root. Renders a left-rail tab list (six sections) and a
// single panel for the active section. Follows the WAI-ARIA APG tabs
// pattern (`role=tablist`, `role=tab`, `aria-selected`, `aria-controls`,
// `role=tabpanel`) and supports arrow / Home / End keyboard navigation
// — matching what the popup ships in T11 round 2.
//
// Each section reads its own data from the OptionsRuntime facade, so
// switching tabs is essentially free (no shared state).

import {
  useCallback,
  useRef,
  useState,
  type JSX,
  type KeyboardEvent,
} from "react";
import { Storage } from "./sections/Storage.js";
import { Identity } from "./sections/Identity.js";
import { Security } from "./sections/Security.js";
import { PerDomain } from "./sections/PerDomain.js";
import { Telemetry } from "./sections/Telemetry.js";
import { About } from "./sections/About.js";

type Section =
  | "storage"
  | "identity"
  | "security"
  | "per-domain"
  | "telemetry"
  | "about";

const SECTION_LABELS: Record<Section, string> = {
  storage: "Storage",
  identity: "Identity",
  security: "Security",
  "per-domain": "Per-domain",
  telemetry: "Telemetry",
  about: "About",
};

const SECTION_ORDER: Section[] = [
  "storage",
  "identity",
  "security",
  "per-domain",
  "telemetry",
  "about",
];

export function App(): JSX.Element {
  const [active, setActive] = useState<Section>("storage");
  const tabRefs = useRef<Partial<Record<Section, HTMLButtonElement | null>>>(
    {},
  );

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLButtonElement>, current: Section) => {
      const idx = SECTION_ORDER.indexOf(current);
      let next: Section | null = null;
      if (e.key === "ArrowDown" || e.key === "ArrowRight") {
        next = SECTION_ORDER[(idx + 1) % SECTION_ORDER.length] ?? null;
      } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
        next =
          SECTION_ORDER[
            (idx - 1 + SECTION_ORDER.length) % SECTION_ORDER.length
          ] ?? null;
      } else if (e.key === "Home") {
        next = SECTION_ORDER[0] ?? null;
      } else if (e.key === "End") {
        next = SECTION_ORDER[SECTION_ORDER.length - 1] ?? null;
      }
      if (next) {
        e.preventDefault();
        setActive(next);
        tabRefs.current[next]?.focus();
      }
    },
    [],
  );

  return (
    <main className="bg-background text-text-primary min-h-screen">
      <div className="max-w-4xl mx-auto p-6 flex flex-col gap-6">
        <header className="flex items-baseline justify-between gap-4">
          <h1 className="text-lg font-mono text-accent-primary">
            Mnemonik · Options
          </h1>
          <span className="text-[10px] uppercase tracking-wide text-text-muted font-mono">
            verifiable memory · MV3
          </span>
        </header>

        <div className="grid grid-cols-[180px_1fr] gap-6">
          <nav
            role="tablist"
            aria-orientation="vertical"
            aria-label="Options sections"
            className="flex flex-col gap-1 border-r border-white/10 pr-2"
          >
            {SECTION_ORDER.map((s) => {
              const selected = active === s;
              return (
                <button
                  key={s}
                  ref={(el) => {
                    tabRefs.current[s] = el;
                  }}
                  type="button"
                  role="tab"
                  id={`opt-tab-${s}`}
                  aria-selected={selected}
                  aria-controls={`opt-panel-${s}`}
                  tabIndex={selected ? 0 : -1}
                  onClick={() => setActive(s)}
                  onKeyDown={(e) => onKeyDown(e, s)}
                  className={`text-left text-xs font-mono uppercase tracking-wide px-3 py-2 rounded border-l-2 transition-colors ${
                    selected
                      ? "border-accent-primary text-accent-primary bg-accent-primary/10"
                      : "border-transparent text-text-muted hover:text-text-primary"
                  }`}
                >
                  {SECTION_LABELS[s]}
                </button>
              );
            })}
          </nav>

          <section className="min-w-0">
            {SECTION_ORDER.map((s) => (
              <div
                key={s}
                role="tabpanel"
                id={`opt-panel-${s}`}
                aria-labelledby={`opt-tab-${s}`}
                tabIndex={0}
                hidden={active !== s}
              >
                {active === s ? <SectionView section={s} /> : null}
              </div>
            ))}
          </section>
        </div>
      </div>
    </main>
  );
}

function SectionView({ section }: { section: Section }): JSX.Element {
  switch (section) {
    case "storage":
      return <Storage />;
    case "identity":
      return <Identity />;
    case "security":
      return <Security />;
    case "per-domain":
      return <PerDomain />;
    case "telemetry":
      return <Telemetry />;
    case "about":
      return <About />;
  }
}

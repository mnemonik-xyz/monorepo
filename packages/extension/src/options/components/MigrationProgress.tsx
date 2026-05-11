// Live migration progress bar. Renders an `aria-live` announcement so
// screen-reader users hear the milestone updates without grabbing
// focus. The progress shape comes from `OptionsRuntime.cloudSync
// .subscribeProgress` events; until T18 wires the SW alarm the events
// are synthetic (single done-event).

import type { JSX } from "react";
import type { MigrationProgressEvent } from "../runtime.js";

export interface MigrationProgressProps {
  progress: MigrationProgressEvent | null;
}

export function MigrationProgress(props: MigrationProgressProps): JSX.Element {
  const { progress } = props;
  const total = progress?.total ?? 0;
  const flushed = progress?.flushed ?? 0;
  const pct =
    total > 0 ? Math.min(100, Math.round((flushed / total) * 100)) : 0;
  const label = progress?.error
    ? `Error: ${progress.error}`
    : progress?.done
      ? "Migration complete"
      : total > 0
        ? `Uploading ${flushed} of ${total}`
        : "Preparing migration…";

  return (
    <div
      role="status"
      aria-live="polite"
      className="border border-accent-primary/40 rounded p-3 flex flex-col gap-2"
    >
      <div className="flex items-center justify-between text-[11px] font-mono">
        <span className="text-accent-primary uppercase tracking-wide">
          Cloud migration
        </span>
        <span className="text-text-muted">{label}</span>
      </div>
      <div
        className="h-1.5 bg-white/5 rounded overflow-hidden"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        aria-label="Cloud migration progress"
      >
        <div
          className="h-full bg-accent-primary transition-[width] duration-300"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

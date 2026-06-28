import type { CSSProperties } from "react";
import type { TimelineBucket } from "../lib/ledger";

/**
 * Bespoke SVG time-series for the attestation analytics page.
 *
 * Per tech-spec Decision 3 there is no charting library: the geometry is a
 * hand-built linear scale rendered into a fixed `viewBox`, which keeps the
 * forensic "Evidence Ledger" aesthetic and adds zero bundle weight. The
 * component is pure — buckets + a descriptive label in, SVG out — so the scale
 * math stays easy to reason about.
 *
 * Two series are drawn: on-node (mint) and on-chain (Solana-purple), each as a
 * line over a faint area fill. The line draw-in uses the shared `.animate-draw`
 * utility (index.css): we set `--draw-len` and `stroke-dasharray` to the exact
 * polyline length so the dash sweep reveals the path. The utility already sets
 * `animation: none` under `prefers-reduced-motion`, and because we never set a
 * static `stroke-dashoffset`, reduced-motion simply shows the finished line.
 */

const VIEW_W = 720;
const VIEW_H = 280;
const PAD = { top: 16, right: 16, bottom: 28, left: 44 } as const;

const PLOT_W = VIEW_W - PAD.left - PAD.right;
const PLOT_H = VIEW_H - PAD.top - PAD.bottom;

const Y_TICKS = 4;

export interface TimelineChartProps {
  buckets: TimelineBucket[];
  /** Drives `role="img"` + the screen-reader summary of the trend. */
  ariaLabel: string;
}

interface Point {
  x: number;
  y: number;
}

/** X position for bucket `i` of `n`. Centres a lone point, no divide-by-zero. */
function xAt(i: number, n: number): number {
  if (n <= 1) return PAD.left + PLOT_W / 2;
  return PAD.left + (i / (n - 1)) * PLOT_W;
}

/** Y position for a count, given the series max (>= 1 to avoid NaN). */
function yAt(value: number, max: number): number {
  return PAD.top + PLOT_H * (1 - value / max);
}

function toPoints(
  buckets: TimelineBucket[],
  pick: (b: TimelineBucket) => number,
  max: number,
): Point[] {
  return buckets.map((b, i) => ({
    x: xAt(i, buckets.length),
    y: yAt(pick(b), max),
  }));
}

/** Polyline length, summed segment-by-segment — exact for straight segments. */
function polylineLength(points: Point[]): number {
  let len = 0;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1];
    const b = points[i];
    if (!a || !b) continue;
    len += Math.hypot(b.x - a.x, b.y - a.y);
  }
  return len;
}

function linePath(points: Point[]): string {
  return points
    .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x.toFixed(2)} ${p.y.toFixed(2)}`)
    .join(" ");
}

/** Area path: the line, closed down to the baseline and back. */
function areaPath(points: Point[]): string {
  const first = points[0];
  const last = points[points.length - 1];
  if (!first || !last) return "";
  const baseline = PAD.top + PLOT_H;
  return `${linePath(points)} L ${last.x.toFixed(2)} ${baseline.toFixed(
    2,
  )} L ${first.x.toFixed(2)} ${baseline.toFixed(2)} Z`;
}

/** Evenly spaced x-axis date labels (first, last, and a few between). */
function xLabelIndices(n: number): number[] {
  if (n <= 1) return [0];
  const target = Math.min(n, 5);
  const idx = new Set<number>();
  for (let i = 0; i < target; i++) {
    idx.add(Math.round((i / (target - 1)) * (n - 1)));
  }
  return [...idx].sort((a, b) => a - b);
}

function formatDate(iso: string): string {
  // ISO day -> "Jun 27". UTC so it never drifts across the local timezone.
  const d = new Date(`${iso}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

function drawStyle(length: number): CSSProperties {
  // `--draw-len` feeds @keyframes draw; dasharray hides the line until the
  // sweep fills it. Cast covers the CSS custom property.
  return {
    "--draw-len": `${length}`,
    strokeDasharray: length || undefined,
  } as CSSProperties;
}

export default function TimelineChart({
  buckets,
  ariaLabel,
}: TimelineChartProps) {
  const n = buckets.length;
  const maxCount = Math.max(
    1,
    ...buckets.map((b) => Math.max(b.on_node, b.on_chain)),
  );

  const nodePoints = toPoints(buckets, (b) => b.on_node, maxCount);
  const chainPoints = toPoints(buckets, (b) => b.on_chain, maxCount);

  const nodeLen = polylineLength(nodePoints);
  const chainLen = polylineLength(chainPoints);

  const yTicks = Array.from({ length: Y_TICKS + 1 }, (_, i) => {
    const value = Math.round((maxCount * i) / Y_TICKS);
    // Key on the tick index, not the value: at small maxCount the rounded
    // values can collide (e.g. 0,0,1,1,1) and duplicate keys are unsupported.
    return { key: i, value, y: yAt(value, maxCount) };
  });

  const xLabels = xLabelIndices(n).flatMap((i) => {
    const bucket = buckets[i];
    return bucket ? [{ label: formatDate(bucket.date), x: xAt(i, n) }] : [];
  });

  return (
    <svg
      role="img"
      aria-label={ariaLabel}
      viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
      preserveAspectRatio="none"
      className="h-72 w-full"
    >
      <defs>
        <linearGradient id="tl-node-fill" x1="0" y1="0" x2="0" y2="1">
          <stop
            offset="0%"
            stopColor="var(--color-accent-primary)"
            stopOpacity="0.18"
          />
          <stop
            offset="100%"
            stopColor="var(--color-accent-primary)"
            stopOpacity="0"
          />
        </linearGradient>
      </defs>

      {/* Horizontal gridlines + y-axis count labels (hairlines). */}
      <g>
        {yTicks.map((t) => (
          <g key={t.key}>
            <line
              x1={PAD.left}
              y1={t.y}
              x2={VIEW_W - PAD.right}
              y2={t.y}
              stroke="currentColor"
              strokeWidth="0.5"
              className="text-white/10"
            />
            <text
              x={PAD.left - 8}
              y={t.y}
              textAnchor="end"
              dominantBaseline="middle"
              className="fill-text-muted font-mono text-[9px] tabular-nums"
            >
              {t.value}
            </text>
          </g>
        ))}
      </g>

      {/* X-axis date labels. */}
      <g>
        {xLabels.map((l) => (
          <text
            key={`${l.label}-${l.x}`}
            x={l.x}
            y={VIEW_H - 8}
            textAnchor="middle"
            className="fill-text-muted font-mono text-[9px]"
          >
            {l.label}
          </text>
        ))}
      </g>

      {/* On-node series: faint area + drawn line (mint). */}
      <path d={areaPath(nodePoints)} fill="url(#tl-node-fill)" stroke="none" />
      <path
        d={linePath(nodePoints)}
        fill="none"
        stroke="var(--color-accent-primary)"
        strokeWidth="2"
        strokeLinejoin="round"
        strokeLinecap="round"
        className="animate-draw"
        style={drawStyle(nodeLen)}
      />

      {/* On-chain series: drawn line only (Solana-purple). */}
      <path
        d={linePath(chainPoints)}
        fill="none"
        stroke="var(--color-accent-secondary)"
        strokeWidth="2"
        strokeLinejoin="round"
        strokeLinecap="round"
        className="animate-draw"
        style={drawStyle(chainLen)}
      />

      {/* A lone bucket has no line to draw — mark the points so it is visible. */}
      {n === 1 && nodePoints[0] && chainPoints[0] && (
        <>
          <circle
            cx={nodePoints[0].x}
            cy={nodePoints[0].y}
            r="3"
            fill="var(--color-accent-primary)"
          />
          <circle
            cx={chainPoints[0].x}
            cy={chainPoints[0].y}
            r="3"
            fill="var(--color-accent-secondary)"
          />
        </>
      )}
    </svg>
  );
}

// TTY-aware output formatter (Decision 10).
//
// Three modes selected by the top-level CLI flags:
//   default    — human-readable, ANSI color when stdout is a TTY
//   --json     — `JSON.stringify(data, null, 2)` to stdout; hints (info/warn)
//                go to stderr and are NOT formatted as JSON
//   --quiet    — only the JSON payload is emitted (if --json was set);
//                otherwise nothing
//   --no-color — disable color even on a TTY
//
// Stdout is reserved for the data payload (so users can pipe `mnemonic recall
// --json | jq`). Stderr carries progress, hints, and warnings so they can be
// silenced with `2>/dev/null` independently.
//
// Note: `kleur` is imported lazily only inside `colorize` so the help-text
// path doesn't pay for an unused dep at startup.

import kleur from "kleur";

export interface OutputOptions {
  json?: boolean;
  quiet?: boolean;
  noColor?: boolean;
  /** Top-level `--verbose` flag — gates {@link verbose} output. */
  verbose?: boolean;
}

/** Resolve effective color setting (TTY + flag + env). */
export function colorEnabled(opts: OutputOptions): boolean {
  if (opts.noColor) return false;
  if (process.env.NO_COLOR && process.env.NO_COLOR.length > 0) return false;
  return Boolean(process.stdout.isTTY);
}

/**
 * Print a result payload to stdout.
 *
 *   --json mode:    JSON.stringify(data, null, 2) regardless of what
 *                   `humanRender` would have produced.
 *   --quiet + json: same as --json.
 *   --quiet alone:  nothing.
 *   default:        humanRender(data) if provided, else JSON-pretty.
 */
export function format(
  data: unknown,
  opts: OutputOptions,
  humanRender?: (d: unknown, color: boolean) => string
): void {
  if (opts.json) {
    process.stdout.write(`${JSON.stringify(data, null, 2)}\n`);
    return;
  }
  if (opts.quiet) {
    // Suppress non-JSON output entirely.
    return;
  }
  const color = colorEnabled(opts);
  const rendered = humanRender ? humanRender(data, color) : prettyJson(data);
  process.stdout.write(`${rendered}\n`);
}

/**
 * Print a human-facing hint (progress, "open this URL", etc.) to stderr.
 * Suppressed under --quiet (regardless of --json).
 */
export function hint(message: string, opts: OutputOptions): void {
  if (opts.quiet) return;
  process.stderr.write(`${message}\n`);
}

/**
 * Print a verbose diagnostic line to stderr when `--verbose` is set.
 *
 * Verbose output is for self-diagnostic of auth issues (issue #27): identity
 * vs JWT.sub comparison, redacted HTTP request/response context, OAuth flow
 * checkpoints. Distinct from {@link hint} (always-on progress) and
 * {@link warn} (always-on warnings) — verbose stays silent in the default
 * case. Honors `--quiet` too (`--verbose --quiet` is an explicit contradiction
 * that resolves to quiet).
 */
export function verbose(message: string, opts: OutputOptions): void {
  if (!opts.verbose) return;
  if (opts.quiet) return;
  process.stderr.write(`[verbose] ${message}\n`);
}

/** Print a warning to stderr (always — even under --quiet). */
export function warn(message: string, opts: OutputOptions): void {
  const color = colorEnabled(opts);
  const tag = color ? kleur.yellow("warning:") : "warning:";
  process.stderr.write(`${tag} ${message}\n`);
}

/** Default human renderer when a command does not provide its own. */
function prettyJson(data: unknown): string {
  if (data === undefined) return "";
  if (typeof data === "string") return data;
  return JSON.stringify(data, null, 2);
}

/**
 * Tiny color helper exposed to commands that want their own renderer.
 * `kleur` doesn't expose a runtime "disable" — we just bypass it when color
 * is disabled.
 */
export function paint(
  s: string,
  color: boolean,
  fn: (s: string) => string
): string {
  return color ? fn(s) : s;
}

export const colors = {
  green: (s: string, c: boolean) => paint(s, c, kleur.green),
  red: (s: string, c: boolean) => paint(s, c, kleur.red),
  yellow: (s: string, c: boolean) => paint(s, c, kleur.yellow),
  cyan: (s: string, c: boolean) => paint(s, c, kleur.cyan),
  dim: (s: string, c: boolean) => paint(s, c, kleur.dim),
  bold: (s: string, c: boolean) => paint(s, c, kleur.bold),
};

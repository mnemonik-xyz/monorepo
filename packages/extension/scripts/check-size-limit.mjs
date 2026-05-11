#!/usr/bin/env node
// TA-MIN5 mitigation (audit follow-up, work/chrome-extension/decisions.md
// 2026-05-11 round-2 deferrals): size-limit 12.x has no
// `--fail-if-not-found` flag. When none of the configured globs match
// (e.g. crxjs renamed the popup chunk after a build-toolchain bump),
// size-limit happily reports `size: 0` + `passed: true` and the
// budget gate silently degrades to a no-op.
//
// This wrapper runs `size-limit --json` and asserts that EVERY
// configured entry has at least one byte of matched payload. If any
// entry reports `size === 0`, we exit non-zero with a diagnostic that
// names the failing entry — the operator's next step is to inspect
// `packages/extension/dist/` and update `.size-limit.json` to match
// the current build output globs.
//
// Usage:
//   node scripts/check-size-limit.mjs
//   # or via the workspace runner:
//   bun run size:check
//
// The script intentionally has zero npm dependencies: it shells out
// to the workspace-installed `size-limit` binary via `npx` so it
// works under bun, node, and pnpm without further wiring.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = resolve(here, "..");

const child = spawnSync("npx", ["--no", "size-limit", "--json"], {
  cwd: pkgRoot,
  encoding: "utf8",
  // size-limit prints its progress UI to stderr; we forward it so the
  // CI log stays interpretable.
  stdio: ["ignore", "pipe", "inherit"],
});

// Note: we ALWAYS try to parse the JSON, even when size-limit's own
// exit code is non-zero. Two reasons:
//   1) size-limit 12.x has a quirk where some "no match" / glob-edge
//      cases exit non-zero with a clean-but-zero JSON report. Our
//      diagnostic below is more actionable than its raw status.
//   2) If size-limit exits with a budget overrun (a real failure),
//      our parse loop will still see `passed: false` and exit
//      non-zero — surfacing the same outcome with a clearer message.
let report = null;
let parseErr = null;
try {
  report = JSON.parse(child.stdout);
} catch (err) {
  parseErr = err;
}

if (parseErr !== null || report === null) {
  process.stderr.write(
    `[size-limit-check] could not parse size-limit JSON output ` +
      `(size-limit exit status: ${String(child.status)})\n`,
  );
  if (parseErr !== null) {
    process.stderr.write(`parse error: ${String(parseErr)}\n`);
  }
  process.stderr.write(`raw output:\n${child.stdout}\n`);
  process.exit(1);
}

if (!Array.isArray(report) || report.length === 0) {
  process.stderr.write(
    `[size-limit-check] size-limit returned no entries — refusing to pass a budget gate that measured nothing\n`,
  );
  process.exit(1);
}

const zeroEntries = report.filter(
  (entry) => typeof entry.size !== "number" || entry.size === 0,
);

if (zeroEntries.length > 0) {
  process.stderr.write(
    `[size-limit-check] the following size-limit entries matched zero bytes — ` +
      `the configured globs in .size-limit.json likely do not match the ` +
      `current dist/ layout. Update the globs and re-run:\n`,
  );
  for (const e of zeroEntries) {
    process.stderr.write(`  - ${String(e.name ?? "<unnamed>")}\n`);
  }
  process.exit(1);
}

// Budget overrun (size-limit's primary contract). We re-check
// `passed` ourselves so the gate works even if a future size-limit
// release decouples the exit code from the per-entry `passed` flag.
const overruns = report.filter((entry) => entry.passed === false);
if (overruns.length > 0) {
  process.stderr.write(
    `[size-limit-check] the following size-limit entries exceeded their budget:\n`,
  );
  for (const e of overruns) {
    process.stderr.write(
      `  - ${String(e.name ?? "<unnamed>")}: ${String(e.size)} bytes\n`,
    );
  }
  process.exit(1);
}

process.stdout.write(
  `[size-limit-check] OK — ${String(report.length)} entries, each matched non-zero bytes\n`,
);
for (const e of report) {
  process.stdout.write(
    `  - ${String(e.name)}: ${String(e.size)} bytes (passed=${String(e.passed)})\n`,
  );
}

// Unit tests for `scripts/check-size-limit.mjs` — the TA-MIN5
// mitigation wrapper. Drives the three failure modes the script
// guards against:
//
//   1. `size: 0` (silent-skip when globs don't match dist layout) →
//      exit 1.
//   2. `passed: false` (budget overrun) → exit 1.
//   3. Non-array / empty report → exit 1.
//
// Approach: spawn the script with a fake `npx` on PATH that prints a
// canned JSON to stdout, so we test the script's parsing + diagnostic
// logic without needing a built dist or a working `size-limit` binary.
// The fake-`npx` shim lives in `tests/fixtures/check-size-limit/` —
// each subdir is a different scenario; the test prepends that subdir
// to PATH before spawning the script.

import { describe, it, expect, beforeAll } from "vitest";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  writeFileSync,
  chmodSync,
  rmSync,
  existsSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = resolve(here, "..", "..", "..");
const scriptPath = join(pkgRoot, "scripts", "check-size-limit.mjs");

/** Build a temp PATH dir hosting a fake `npx` that echoes the
 *  supplied JSON to stdout and exits 0. */
function fakeNpxPathDir(suffix: string, jsonStdout: string): string {
  const dir = join(
    tmpdir(),
    `t23-fake-npx-${suffix}-${Math.random().toString(36).slice(2)}`,
  );
  if (existsSync(dir)) rmSync(dir, { recursive: true });
  mkdirSync(dir, { recursive: true });
  const shim = join(dir, "npx");
  // POSIX shell shim — heredoc preserves the JSON byte-for-byte.
  writeFileSync(
    shim,
    `#!/bin/sh\ncat <<'NPX_FAKE_EOF'\n${jsonStdout}\nNPX_FAKE_EOF\n`,
    { mode: 0o755 },
  );
  chmodSync(shim, 0o755);
  return dir;
}

function runScript(env: NodeJS.ProcessEnv): {
  status: number | null;
  stdout: string;
  stderr: string;
} {
  const out = spawnSync("node", [scriptPath], {
    env,
    encoding: "utf8",
  });
  return {
    status: out.status,
    stdout: out.stdout ?? "",
    stderr: out.stderr ?? "",
  };
}

beforeAll(() => {
  // Sanity: the script we test must exist.
  if (!existsSync(scriptPath)) {
    throw new Error(`check-size-limit.mjs not found at ${scriptPath}`);
  }
});

describe("check-size-limit.mjs", () => {
  it("exits 0 when every entry has non-zero size and passed=true", () => {
    const pathDir = fakeNpxPathDir(
      "ok",
      JSON.stringify([
        {
          name: "popup-initial-js",
          passed: true,
          size: 12_345,
          loading: 0,
        },
      ]),
    );
    const env = {
      ...process.env,
      PATH: `${pathDir}:${process.env.PATH ?? ""}`,
    };
    const out = runScript(env);
    expect(out.status).toBe(0);
    expect(out.stdout).toMatch(/OK — 1 entries/);
  });

  it("exits 1 with a 'matched zero bytes' diagnostic when size === 0 (TA-MIN5 silent-skip)", () => {
    // This is the audit-flagged case: globs don't match dist/ layout
    // so size-limit reports `size: 0, passed: true`. The wrapper MUST
    // surface this as a hard failure.
    const pathDir = fakeNpxPathDir(
      "zero",
      JSON.stringify([
        {
          name: "popup-initial-js",
          passed: true,
          size: 0,
          loading: 0,
        },
      ]),
    );
    const env = {
      ...process.env,
      PATH: `${pathDir}:${process.env.PATH ?? ""}`,
    };
    const out = runScript(env);
    expect(out.status).toBe(1);
    expect(out.stderr).toMatch(/matched zero bytes/);
    expect(out.stderr).toMatch(/popup-initial-js/);
  });

  it("exits 1 when an entry reports passed=false (real budget overrun)", () => {
    const pathDir = fakeNpxPathDir(
      "overrun",
      JSON.stringify([
        {
          name: "popup-initial-js",
          passed: false,
          size: 99_999,
          loading: 0,
        },
      ]),
    );
    const env = {
      ...process.env,
      PATH: `${pathDir}:${process.env.PATH ?? ""}`,
    };
    const out = runScript(env);
    expect(out.status).toBe(1);
    expect(out.stderr).toMatch(/exceeded their budget/);
  });

  it("exits 1 when size-limit returns an empty array", () => {
    const pathDir = fakeNpxPathDir("empty", "[]");
    const env = {
      ...process.env,
      PATH: `${pathDir}:${process.env.PATH ?? ""}`,
    };
    const out = runScript(env);
    expect(out.status).toBe(1);
    expect(out.stderr).toMatch(/no entries/);
  });

  it("exits 1 when size-limit output is not valid JSON", () => {
    const pathDir = fakeNpxPathDir("garbage", "not-json-at-all");
    const env = {
      ...process.env,
      PATH: `${pathDir}:${process.env.PATH ?? ""}`,
    };
    const out = runScript(env);
    expect(out.status).toBe(1);
    expect(out.stderr).toMatch(/could not parse size-limit JSON output/);
  });
});

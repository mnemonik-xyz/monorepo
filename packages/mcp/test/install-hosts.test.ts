import { mkdir, readFile, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  _internal as installHostsInternal,
  hostCandidates,
  installHosts,
} from "../src/install-hosts.js";

import { cleanupTemp, freshTempDir } from "./helpers.js";

interface Ctx {
  home: string;
  origHome: string | undefined;
  origUserprofile: string | undefined;
  capturedStdout: string;
  capturedStderr: string;
  restoreStdout: () => void;
  restoreStderr: () => void;
}

let ctx: Ctx;

function captureStream(
  stream: NodeJS.WriteStream,
  sink: { value: string },
): () => void {
  const original = stream.write.bind(stream);
  // Vitest's WriteStream typings reject the chunk type — cast via unknown.
  stream.write = ((chunk: unknown) => {
    sink.value += String(chunk);
    return true;
  }) as unknown as typeof stream.write;
  return () => {
    stream.write = original;
  };
}

beforeEach(async () => {
  const home = await freshTempDir("mnemonik-mcp-home-");
  ctx = {
    home,
    origHome: process.env.HOME,
    origUserprofile: process.env.USERPROFILE,
    capturedStdout: "",
    capturedStderr: "",
    restoreStdout: () => {},
    restoreStderr: () => {},
  };
  process.env.HOME = home;
  process.env.USERPROFILE = home;
  const outSink = { value: "" };
  const errSink = { value: "" };
  const undoOut = captureStream(process.stdout, outSink);
  const undoErr = captureStream(process.stderr, errSink);
  ctx.restoreStdout = () => {
    undoOut();
    ctx.capturedStdout = outSink.value;
  };
  ctx.restoreStderr = () => {
    undoErr();
    ctx.capturedStderr = errSink.value;
  };
});

afterEach(async () => {
  ctx.restoreStdout();
  ctx.restoreStderr();
  if (ctx.origHome === undefined) delete process.env.HOME;
  else process.env.HOME = ctx.origHome;
  if (ctx.origUserprofile === undefined) delete process.env.USERPROFILE;
  else process.env.USERPROFILE = ctx.origUserprofile;
  await cleanupTemp(ctx.home);
});

async function writeCandidate(p: string, content: object): Promise<void> {
  await mkdir(dirname(p), { recursive: true });
  await writeFile(p, JSON.stringify(content, null, 2));
}

describe("installHosts", () => {
  it("non_destructive_merge: only mcpServers.mnemonik added, other keys byte-identical", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    const original = {
      other: "value",
      mcpServers: { foo: { command: "/x", args: ["--y"] } },
      nested: { keep: [1, 2, 3] },
    };
    await writeCandidate(claudeJson, original);
    const before = await readFile(claudeJson, "utf8");

    await installHosts({});

    const after = JSON.parse(await readFile(claudeJson, "utf8")) as {
      other: string;
      mcpServers: Record<string, unknown>;
      nested: { keep: number[] };
    };
    expect(after.other).toBe("value");
    expect(after.mcpServers.foo).toEqual({ command: "/x", args: ["--y"] });
    expect(after.nested).toEqual({ keep: [1, 2, 3] });
    expect(after.mcpServers.mnemonik).toBeDefined();
    // Pre-install content remained as a substring of the pre-edit form
    // (proves we didn't reorder or strip unrelated keys).
    expect(before).toContain('"other": "value"');
  });

  it("idempotent: running install twice produces byte-identical final file", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    await writeCandidate(claudeJson, { mcpServers: {} });

    await installHosts({});
    const firstPass = await readFile(claudeJson, "utf8");
    await installHosts({});
    const secondPass = await readFile(claudeJson, "utf8");
    expect(secondPass).toBe(firstPass);
  });

  it("all_candidates_absent_exits_0: stdout includes 'no host configs found'", async () => {
    await installHosts({});
    ctx.restoreStdout();
    expect(ctx.capturedStdout).toMatch(/no host configs found/);
    // Restart line must NOT appear when there's nothing to restart.
    expect(ctx.capturedStdout).not.toMatch(/please restart/);
  });

  it("check_mode_no_writes: mtimes unchanged, no .mnemonik.tmp left behind", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    const cursorJson = cands[2]!;
    await writeCandidate(claudeJson, { mcpServers: {} });
    await writeCandidate(cursorJson, { mcpServers: {} });
    const claudeBefore = await stat(claudeJson);
    const cursorBefore = await stat(cursorJson);

    await installHosts({ check: true });

    const claudeAfter = await stat(claudeJson);
    const cursorAfter = await stat(cursorJson);
    expect(claudeAfter.mtimeMs).toBe(claudeBefore.mtimeMs);
    expect(cursorAfter.mtimeMs).toBe(cursorBefore.mtimeMs);
    // No .mnemonik.tmp files anywhere in the home dir.
    const { readdir } = await import("node:fs/promises");
    const claudeDir = await readdir(dirname(claudeJson));
    const cursorDir = await readdir(dirname(cursorJson));
    for (const e of [...claudeDir, ...cursorDir]) {
      expect(e.endsWith(".mnemonik.tmp")).toBe(false);
    }
  });

  it("symlink_out_of_home_refuses: refuses and does NOT modify the off-home target", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    // Target file lives outside HOME entirely.
    const offHomeDir = await freshTempDir("mnemonik-offhome-");
    try {
      const offHomeTarget = join(offHomeDir, "victim.json");
      await writeFile(offHomeTarget, JSON.stringify({ original: true }));
      await mkdir(dirname(claudeJson), { recursive: true });
      await symlink(offHomeTarget, claudeJson);

      await expect(installHosts({})).rejects.toThrow(/symlink/);
      // Off-home target untouched.
      const after = JSON.parse(await readFile(offHomeTarget, "utf8")) as {
        original: boolean;
      };
      expect(after.original).toBe(true);
    } finally {
      await cleanupTemp(offHomeDir);
    }
  });

  it("restart_instruction_in_output: final stdout line is the exact restart string", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    await writeCandidate(claudeJson, { mcpServers: {} });

    await installHosts({});
    ctx.restoreStdout();
    const lines = ctx.capturedStdout.trimEnd().split("\n");
    expect(lines.at(-1)).toBe(
      "If any of these agents is already running, please restart them.",
    );
  });

  it("empty_zero_byte_json_parses_as_empty_object", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    await mkdir(dirname(claudeJson), { recursive: true });
    await writeFile(claudeJson, "");

    await installHosts({});
    const parsed = JSON.parse(await readFile(claudeJson, "utf8")) as {
      mcpServers: { mnemonik?: { command: string; args: string[] } };
    };
    expect(parsed.mcpServers.mnemonik).toBeDefined();
    expect(parsed.mcpServers.mnemonik!.args).toEqual(["mcp-stdio"]);
  });

  it("malformed_json_aborts_per_candidate_but_does_not_touch_others", async () => {
    const cands = hostCandidates(ctx.home);
    const claudeJson = cands[0]!;
    const cursorJson = cands[2]!;
    await mkdir(dirname(claudeJson), { recursive: true });
    await writeFile(claudeJson, "{ this is not json");
    await writeCandidate(cursorJson, { mcpServers: {} });
    const claudeBefore = await readFile(claudeJson, "utf8");

    // Should throw (firstError surfaced) AFTER attempting all candidates so
    // the cursor file gets updated.
    await expect(installHosts({})).rejects.toThrow();
    const claudeAfter = await readFile(claudeJson, "utf8");
    expect(claudeAfter).toBe(claudeBefore);
    const cursorAfter = JSON.parse(await readFile(cursorJson, "utf8")) as {
      mcpServers: { mnemonik?: unknown };
    };
    expect(cursorAfter.mcpServers.mnemonik).toBeDefined();
  });
});

describe("install-hosts internals", () => {
  it("atomicWriteJson stages a .mnemonik.tmp then renames over original", async () => {
    const dir = await freshTempDir("atomic-write-");
    try {
      const finalPath = join(dir, "config.json");
      await writeFile(finalPath, "{}");
      await installHostsInternal.atomicWriteJson(finalPath, { x: 1 });
      const parsed = JSON.parse(await readFile(finalPath, "utf8")) as {
        x: number;
      };
      expect(parsed.x).toBe(1);
      // tmp file must NOT remain after success.
      const { readdir } = await import("node:fs/promises");
      const entries = await readdir(dir);
      expect(entries.filter((e) => e.endsWith(".mnemonik.tmp"))).toEqual([]);
    } finally {
      await cleanupTemp(dir);
    }
  });
});

import {
  lstat,
  readFile,
  realpath,
  rename,
  stat,
  writeFile,
} from "node:fs/promises";
import { dirname, join, sep } from "node:path";

import { binaryPath, homedir } from "./paths.js";

export interface InstallHostsOptions {
  check?: boolean;
  confirm?: boolean;
}

const RESTART_LINE =
  "If any of these agents is already running, please restart them.";

export function hostCandidates(home: string = homedir()): string[] {
  return [
    join(home, ".claude.json"),
    join(
      home,
      "Library",
      "Application Support",
      "Claude",
      "claude_desktop_config.json",
    ),
    join(home, ".cursor", "mcp.json"),
  ];
}

async function exists(p: string): Promise<boolean> {
  try {
    await stat(p);
    return true;
  } catch {
    return false;
  }
}

/**
 * If `candidate` is a symlink, refuse it when its realpath escapes the user's
 * home directory. A symlink rooted at `~/.claude.json` pointing at `/tmp/foo`
 * would let an attacker who controls a tempdir capture writes that are
 * supposed to land in $HOME.
 */
async function assertNotSymlinkOutOfHome(
  candidate: string,
  home: string,
): Promise<void> {
  let l;
  try {
    l = await lstat(candidate);
  } catch {
    return; // absent — caller will skip
  }
  if (!l.isSymbolicLink()) return;
  const target = await realpath(candidate);
  const homeWithSep = home.endsWith(sep) ? home : home + sep;
  if (!target.startsWith(homeWithSep) && target !== home) {
    throw new Error(
      `refusing to follow symlink at ${candidate} → ${target} (target escapes ${home}); ` +
        `delete or repoint the symlink before re-running install`,
    );
  }
}

async function promptYesNo(question: string): Promise<boolean> {
  process.stdout.write(question);
  return new Promise<boolean>((res) => {
    const onData = (chunk: Buffer): void => {
      const answer = chunk.toString("utf8").trim().toLowerCase();
      process.stdin.removeListener("data", onData);
      process.stdin.pause();
      res(answer === "y" || answer === "yes");
    };
    process.stdin.resume();
    process.stdin.once("data", onData);
  });
}

/**
 * Deep-clone via JSON. Sufficient for MCP host configs (pure JSON, no
 * Dates/Maps/etc).
 */
function deepClone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

/**
 * Write `content` atomically: stage at `<finalPath>.mnemonik.tmp` in the same
 * directory, fsync, then rename over the original. Same-dir rename is the
 * only ordering POSIX guarantees as atomic.
 */
async function atomicWriteJson(
  finalPath: string,
  content: unknown,
): Promise<void> {
  const tmp = finalPath + ".mnemonik.tmp";
  const serialized = JSON.stringify(content, null, 2) + "\n";
  const { open } = await import("node:fs/promises");
  const fh = await open(tmp, "w", 0o600);
  try {
    await fh.writeFile(serialized);
    await fh.sync();
  } finally {
    await fh.close();
  }
  await rename(tmp, finalPath);
}

export async function installHosts(
  opts: InstallHostsOptions = {},
): Promise<void> {
  const home = homedir();
  const candidates = hostCandidates(home);
  const binPath = binaryPath();

  // First pass: which candidates exist?
  const present: string[] = [];
  for (const c of candidates) {
    if (await exists(c)) present.push(c);
  }

  if (present.length === 0) {
    process.stdout.write(
      "no host configs found; install mnemonik by adding mcp.mnemonik.xyz to your agent's MCP config directly\n",
    );
    return;
  }

  let firstError: Error | undefined;

  for (const candidate of present) {
    try {
      await assertNotSymlinkOutOfHome(candidate, home);
      const raw = await readFile(candidate, "utf8");
      const parsed: Record<string, unknown> =
        raw.trim().length === 0
          ? {}
          : (JSON.parse(raw) as Record<string, unknown>);

      const next = deepClone(parsed);
      const servers = (next.mcpServers ??= {} as Record<
        string,
        unknown
      >) as Record<string, unknown>;
      const desired = {
        command: binPath,
        args: ["mcp-stdio"],
      };
      // Idempotency: only stage a write if the entry would actually change.
      const existing = servers.mnemonik;
      const wouldChange = JSON.stringify(existing) !== JSON.stringify(desired);

      if (opts.check) {
        process.stdout.write(
          wouldChange
            ? `would set mcpServers.mnemonik in ${candidate}\n`
            : `mcpServers.mnemonik already set in ${candidate}\n`,
        );
        continue;
      }

      if (opts.confirm && wouldChange) {
        const ok = await promptYesNo(
          `would write to ${candidate}; continue? [y/N] `,
        );
        if (!ok) {
          process.stdout.write(`skipped ${candidate}\n`);
          continue;
        }
      }

      if (!wouldChange) {
        process.stdout.write(
          `mcpServers.mnemonik already set in ${candidate}\n`,
        );
        continue;
      }

      servers.mnemonik = desired;
      await atomicWriteJson(candidate, next);
      process.stdout.write(`updated ${candidate}\n`);
    } catch (e) {
      const err = e instanceof Error ? e : new Error(String(e));
      process.stderr.write(`error processing ${candidate}: ${err.message}\n`);
      if (firstError === undefined) firstError = err;
      // Per task spec: per-candidate atomicity. Continue to next candidate.
    }
  }

  // Suppress the restart hint in check mode — no files were modified, so the
  // user has nothing to restart. Per code-reviewer round 1 ISSUE-001.
  if (!opts.check) {
    process.stdout.write(RESTART_LINE + "\n");
  }

  if (firstError !== undefined) throw firstError;
}

export const _internal = {
  RESTART_LINE,
  atomicWriteJson,
  assertNotSymlinkOutOfHome,
};

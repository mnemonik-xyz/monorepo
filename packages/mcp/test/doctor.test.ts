import { createHash } from "node:crypto";
import { chmod, mkdir, rm, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { doctor } from "../src/doctor.js";
import { hostCandidates } from "../src/install-hosts.js";
import { binaryPath, cacheDir, manifestPath } from "../src/paths.js";

import { cleanupTemp, freshTempDir } from "./helpers.js";

interface Ctx {
  home: string;
  cache: string;
  config: string;
  origHome: string | undefined;
  origCache: string | undefined;
  origConfig: string | undefined;
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
  stream.write = ((chunk: unknown) => {
    sink.value += String(chunk);
    return true;
  }) as unknown as typeof stream.write;
  return () => {
    stream.write = original;
  };
}

beforeEach(async () => {
  const home = await freshTempDir("mnemonik-doctor-home-");
  const cache = await freshTempDir("mnemonik-doctor-cache-");
  const config = await freshTempDir("mnemonik-doctor-cfg-");
  ctx = {
    home,
    cache,
    config,
    origHome: process.env.HOME,
    origCache: process.env.MNEMONIK_MCP_CACHE_DIR,
    origConfig: process.env.MNEMONIC_CONFIG_DIR,
    capturedStdout: "",
    capturedStderr: "",
    restoreStdout: () => {},
    restoreStderr: () => {},
  };
  process.env.HOME = home;
  process.env.USERPROFILE = home;
  process.env.MNEMONIK_MCP_CACHE_DIR = cache;
  process.env.MNEMONIC_CONFIG_DIR = config;
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
  vi.unstubAllGlobals();
  if (ctx.origHome === undefined) delete process.env.HOME;
  else process.env.HOME = ctx.origHome;
  if (ctx.origCache === undefined) delete process.env.MNEMONIK_MCP_CACHE_DIR;
  else process.env.MNEMONIK_MCP_CACHE_DIR = ctx.origCache;
  if (ctx.origConfig === undefined) delete process.env.MNEMONIC_CONFIG_DIR;
  else process.env.MNEMONIC_CONFIG_DIR = ctx.origConfig;
  await cleanupTemp(ctx.home);
  await cleanupTemp(ctx.cache);
  await cleanupTemp(ctx.config);
});

async function seedHappyFixture(): Promise<{ sha: string }> {
  // Host config with mnemonik entry.
  const claudeJson = hostCandidates(ctx.home)[0]!;
  const { dirname } = await import("node:path");
  await mkdir(dirname(claudeJson), { recursive: true });
  await writeFile(
    claudeJson,
    JSON.stringify(
      { mcpServers: { mnemonik: { command: "/x", args: ["mcp-stdio"] } } },
      null,
      2,
    ),
  );
  // Cached binary + manifest.
  await mkdir(join(cacheDir(), "bin"), { recursive: true, mode: 0o700 });
  const body = "#!/bin/sh\nexit 0\n";
  await writeFile(binaryPath(), body, { mode: 0o755 });
  const h = createHash("sha256");
  h.update(body);
  const sha = h.digest("hex");
  await writeFile(
    manifestPath(),
    JSON.stringify({
      sha256: sha,
      sha256sums_url: "https://example/SHA256SUMS",
      artifact: "mnemonic-mcp.tar.gz",
      tag: "v0.2.0",
      attestation_subject: "stub",
      publisher: {
        owner: "mnemonik-xyz",
        repo: "mnemonik-xyz/monorepo",
        workflow: ".github/workflows/release.yml",
      },
    }),
  );
  // Identity file.
  await mkdir(ctx.config, { recursive: true, mode: 0o700 });
  await writeFile(
    join(ctx.config, "id.json"),
    JSON.stringify({ publicKey: "abc123" }),
  );
  // Token file: valid, far-future expiry.
  const farFuture = Math.floor(Date.now() / 1000) + 3600;
  await writeFile(
    join(ctx.config, "token.json"),
    JSON.stringify({
      jwt: "eyJfake.fake.fake",
      expires_at: farFuture,
      sub: "abc123",
    }),
  );
  return { sha };
}

function stubHealthy(): void {
  vi.stubGlobal(
    "fetch",
    (async () =>
      new Response(JSON.stringify({ status: "ok" }), {
        status: 200,
      })) as unknown as typeof fetch,
  );
}

function stub503(): void {
  vi.stubGlobal(
    "fetch",
    (async () =>
      new Response("err", { status: 503 })) as unknown as typeof fetch,
  );
}

describe("doctor", () => {
  it("happy_path: exit 0, all 6 checks pass", async () => {
    await seedHappyFixture();
    stubHealthy();

    const r = await doctor();
    expect(r.exitCode).toBe(0);
    expect(r.checks).toHaveLength(6);
    for (const c of r.checks) {
      expect(c.pass).toBe(true);
    }
    ctx.restoreStdout();
    expect(ctx.capturedStdout).toMatch(/host-config-entry-presence: pass/);
    expect(ctx.capturedStdout).toMatch(/mcp-mnemonik-xyz-health: pass/);
    expect(ctx.capturedStdout).toMatch(/binary-integrity: pass/);
    expect(ctx.capturedStdout).toMatch(/local-sqlite-rw: pass/);
    expect(ctx.capturedStdout).toMatch(/identity-accessible: pass/);
    expect(ctx.capturedStdout).toMatch(/token-file-integrity: pass/);
  });

  it("parametrized_failures: each failure mode flips its check to fail with repair hint", async () => {
    interface FailureCase {
      name: string;
      check: string;
      setup: () => Promise<void>;
    }
    const cases: FailureCase[] = [
      {
        name: "missing-host-config-entry",
        check: "host-config-entry-presence",
        setup: async () => {
          // Host file exists but lacks mnemonik entry.
          const claudeJson = hostCandidates(ctx.home)[0]!;
          const { dirname } = await import("node:path");
          await mkdir(dirname(claudeJson), { recursive: true });
          await writeFile(
            claudeJson,
            JSON.stringify({ mcpServers: { foo: {} } }),
          );
        },
      },
      {
        name: "health-503",
        check: "mcp-mnemonik-xyz-health",
        setup: async () => {
          stub503();
        },
      },
      {
        name: "corrupted-cached-binary",
        check: "binary-integrity",
        setup: async () => {
          await seedHappyFixture();
          await chmod(binaryPath(), 0o600);
          await writeFile(binaryPath(), "tampered\n", { mode: 0o755 });
        },
      },
      {
        name: "config-dir-readonly",
        check: "local-sqlite-rw",
        setup: async () => {
          // Point MNEMONIC_CONFIG_DIR at a non-existent path inside a
          // non-writable parent — mkdir + writeFile both fail.
          process.env.MNEMONIC_CONFIG_DIR = "/dev/null/cannot-write";
        },
      },
      {
        name: "missing-identity",
        check: "identity-accessible",
        setup: async () => {
          // Nothing in config dir.
          await rm(join(ctx.config, "id.json"), { force: true });
          await rm(join(ctx.config, "identity.json"), { force: true });
        },
      },
      {
        name: "malformed-token",
        check: "token-file-integrity",
        setup: async () => {
          await mkdir(ctx.config, { recursive: true, mode: 0o700 });
          await writeFile(join(ctx.config, "token.json"), "{not json");
        },
      },
    ];

    for (const c of cases) {
      // Reset per case to avoid cross-contamination.
      // Restore MNEMONIC_CONFIG_DIR — the config-dir-readonly case mutates
      // it to point at /dev/null; without restoring, later cases' seeds
      // would silently write to the unwritable path.
      process.env.MNEMONIC_CONFIG_DIR = ctx.config;
      await rm(ctx.home, { recursive: true, force: true });
      await rm(ctx.cache, { recursive: true, force: true });
      await rm(ctx.config, { recursive: true, force: true });
      await mkdir(ctx.home, { recursive: true });
      await mkdir(ctx.cache, { recursive: true, mode: 0o700 });
      await mkdir(ctx.config, { recursive: true, mode: 0o700 });
      // Seed baseline so only the targeted check fails — except cases that
      // can't run after the seed (the seed itself supplies a valid one).
      if (c.name !== "corrupted-cached-binary") {
        await seedHappyFixture();
      }
      if (c.name !== "health-503") stubHealthy();
      else stub503();
      // The MNEMONIC_CONFIG_DIR override case must run AFTER seed; seed
      // wrote to the real config dir but we override the env mid-test so
      // the doctor's probe targets the unwritable path.
      await c.setup();

      const r = await doctor();
      expect(r.exitCode, `${c.name} should fail`).toBe(1);
      const failed = r.checks.find((x) => x.name === c.check);
      expect(failed, `${c.name} expected check ${c.check}`).toBeDefined();
      expect(failed!.pass, `${c.name}: ${c.check} should be fail`).toBe(false);
      expect(
        failed!.detail.length,
        `${c.name}: repair hint nonempty`,
      ).toBeGreaterThan(0);
      // For non-health failures, repair-hint should mention "repair" or "delete"/"run".
      if (c.name !== "health-503") {
        expect(failed!.detail).toMatch(/repair|delete|run/i);
      }
    }
  });
});

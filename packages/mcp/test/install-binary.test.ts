import { chmod, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  _internal as installInternal,
  ensureBinaryCached,
  parseSha256Sums,
} from "../src/install-binary.js";
import {
  binaryPath,
  cacheDir,
  manifestPath,
  releaseBaseUrl,
} from "../src/paths.js";

import {
  cleanupTemp,
  freshTempDir,
  makeBinaryTarball,
  makeFetchStub,
  makeMaliciousTarball,
  makeMockGh,
  makeSymlinkTarball,
  sha256SumsBody,
} from "./helpers.js";

interface TestCtx {
  cache: string;
  fixtures: string;
  ghRecord: string;
  origPath: string | undefined;
}

let ctx: TestCtx;
const TAG = "v0.2.4";
// Match the dist/binary-version.json darwin-arm64 mapping; tests force the
// platform via stubbing process.platform.
const ARTIFACT = `mnemonic-mcp-${TAG}-aarch64-apple-darwin.tar.gz`;
const BASE_URL = "https://release.example/dl";
const ARTIFACT_URL = `${BASE_URL}/${TAG}/${ARTIFACT}`;
const SUMS_URL = `${BASE_URL}/${TAG}/SHA256SUMS`;

async function setupCtx(): Promise<TestCtx> {
  const cache = await freshTempDir("mnemonik-mcp-cache-");
  const fixtures = await freshTempDir("mnemonik-mcp-fix-");
  const ghRecord = join(fixtures, "gh-argv.log");
  process.env.MNEMONIK_MCP_CACHE_DIR = cache;
  process.env.MNEMONIK_MCP_RELEASE_BASE_URL = BASE_URL;
  // Force darwin-arm64 so detectPlatform picks up the artifact map.
  Object.defineProperty(process, "platform", {
    value: "darwin",
    configurable: true,
  });
  Object.defineProperty(process, "arch", {
    value: "arm64",
    configurable: true,
  });
  const orig = process.env.PATH;
  return { cache, fixtures, ghRecord, origPath: orig };
}

async function teardownCtx(c: TestCtx): Promise<void> {
  delete process.env.MNEMONIK_MCP_CACHE_DIR;
  delete process.env.MNEMONIK_MCP_RELEASE_BASE_URL;
  if (c.origPath !== undefined) process.env.PATH = c.origPath;
  await cleanupTemp(c.cache);
  await cleanupTemp(c.fixtures);
}

beforeEach(async () => {
  ctx = await setupCtx();
});

afterEach(async () => {
  vi.unstubAllGlobals();
  await teardownCtx(ctx);
});

describe("parseSha256Sums", () => {
  it("parses well-formed lines and tolerates the binary-mode `*` prefix", () => {
    const body = [
      "deadbeef".repeat(8) + "  artifact-one.tar.gz",
      "00".repeat(32) + " *artifact-two.tar.gz",
      "",
      "# comment-line (no hex)",
    ].join("\n");
    const map = parseSha256Sums(body);
    expect(map.size).toBe(2);
    expect(map.get("artifact-one.tar.gz")).toBe("deadbeef".repeat(8));
    expect(map.get("artifact-two.tar.gz")).toBe("00".repeat(32));
  });
});

describe("releaseBaseUrl validation (SAR7-M2)", () => {
  // Each case reads and writes the same env vars releaseBaseUrl() consults;
  // beforeEach/afterEach (outer setupCtx/teardownCtx) reset them between
  // tests. We snapshot and restore the M2-specific vars here too because
  // some cases mutate MNEMONIK_MCP_ALLOW_HTTP which the outer harness does
  // not track.
  let origAllowHttp: string | undefined;
  let origBaseUrl: string | undefined;

  beforeEach(() => {
    origAllowHttp = process.env.MNEMONIK_MCP_ALLOW_HTTP;
    origBaseUrl = process.env.MNEMONIK_MCP_RELEASE_BASE_URL;
  });

  afterEach(() => {
    if (origAllowHttp === undefined) delete process.env.MNEMONIK_MCP_ALLOW_HTTP;
    else process.env.MNEMONIK_MCP_ALLOW_HTTP = origAllowHttp;
    if (origBaseUrl === undefined)
      delete process.env.MNEMONIK_MCP_RELEASE_BASE_URL;
    else process.env.MNEMONIK_MCP_RELEASE_BASE_URL = origBaseUrl;
  });

  it("returns compiled-in default when no override set", () => {
    delete process.env.MNEMONIK_MCP_RELEASE_BASE_URL;
    delete process.env.MNEMONIK_MCP_ALLOW_HTTP;
    expect(releaseBaseUrl()).toMatch(/^https:\/\/github\.com\//);
  });

  it("accepts https override unconditionally", () => {
    process.env.MNEMONIK_MCP_RELEASE_BASE_URL = "https://mirror.example/dl";
    delete process.env.MNEMONIK_MCP_ALLOW_HTTP;
    expect(releaseBaseUrl()).toBe("https://mirror.example/dl");
  });

  it("rejects http override without MNEMONIK_MCP_ALLOW_HTTP — closes MITM redirect vector", () => {
    process.env.MNEMONIK_MCP_RELEASE_BASE_URL = "http://attacker.example";
    delete process.env.MNEMONIK_MCP_ALLOW_HTTP;
    expect(() => releaseBaseUrl()).toThrow(/must use https:\/\//);
  });

  it("accepts http override only when MNEMONIK_MCP_ALLOW_HTTP=1 escape hatch is set", () => {
    process.env.MNEMONIK_MCP_RELEASE_BASE_URL = "http://localhost:9999/dl";
    process.env.MNEMONIK_MCP_ALLOW_HTTP = "1";
    expect(releaseBaseUrl()).toBe("http://localhost:9999/dl");
  });

  it("rejects malformed URLs", () => {
    process.env.MNEMONIK_MCP_RELEASE_BASE_URL = "not a url";
    delete process.env.MNEMONIK_MCP_ALLOW_HTTP;
    expect(() => releaseBaseUrl()).toThrow(/is not a valid URL/);
  });

  it("rejects file:// and other non-http(s) schemes even with escape hatch", () => {
    process.env.MNEMONIK_MCP_RELEASE_BASE_URL = "file:///etc/passwd";
    process.env.MNEMONIK_MCP_ALLOW_HTTP = "1";
    expect(() => releaseBaseUrl()).toThrow(/must use https:\/\//);
  });
});

describe("ensureBinaryCached", () => {
  it("sha256_match_succeeds: downloads, verifies SHA256, runs gh attest, extracts, writes manifest", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeBinaryTarball(tarFile);
    const tarBytes = await readFile(tarFile);

    const { fetch: stub, calls } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(sha256, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub);

    const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;

    const path = await ensureBinaryCached();
    expect(path).toBe(binaryPath());
    const onDisk = await stat(path);
    expect(onDisk.isFile()).toBe(true);
    // 0o755 bit set
    expect(onDisk.mode & 0o777).toBe(0o755);

    const manifestRaw = await readFile(manifestPath(), "utf8");
    const manifest = JSON.parse(manifestRaw) as {
      sha256: string;
      tarball_sha256: string;
      tag: string;
      publisher: { owner: string; repo: string; workflow: string };
    };
    // manifest.sha256 is the binary's on-disk SHA, used for cache re-verify.
    expect(manifest.sha256).toMatch(/^[0-9a-f]{64}$/);
    // manifest.tarball_sha256 records the SHA256SUMS entry for audit.
    expect(manifest.tarball_sha256).toBe(sha256);
    expect(manifest.tag).toBe(TAG);
    expect(manifest.publisher.owner).toBe("mnemonik-xyz");
    expect(manifest.publisher.repo).toBe("mnemonik-xyz/monorepo");
    expect(manifest.publisher.workflow).toBe(
      "mnemonik-xyz/monorepo/.github/workflows/release.yml",
    );
    expect(calls).toContain(ARTIFACT_URL);
    expect(calls).toContain(SUMS_URL);
  });

  it("sha256_mismatch_rejected: throws when SHA256SUMS line does not match the downloaded bytes", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    await makeBinaryTarball(tarFile);
    const tarBytes = await readFile(tarFile);
    const wrong = "ff".repeat(32);

    const { fetch: stub } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(wrong, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub);

    const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;

    await expect(ensureBinaryCached()).rejects.toThrow(/SHA256 mismatch/);
    // No manifest written on failure.
    await expect(stat(manifestPath())).rejects.toThrow();
  });

  it("gh_attestation_verify_invocation: argv contains --repo --signer-workflow with pinned values and no --owner", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeBinaryTarball(tarFile);
    const tarBytes = await readFile(tarFile);
    const { fetch: stub } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(sha256, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub);

    const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;

    await ensureBinaryCached();
    // Per test-reviewer round 1 FINDING-1: substring `toContain` checks would
    // pass even if the flag and its value were swapped or reordered, letting
    // `gh attestation verify` receive malformed argv. Assert each flag's
    // exact follower in the argv list.
    const args = (await readFile(ctx.ghRecord, "utf8"))
      .split("\n")
      .filter((line) => line.length > 0);
    const repoIdx = args.indexOf("--repo");
    const wfIdx = args.indexOf("--signer-workflow");
    expect(repoIdx).toBeGreaterThan(-1);
    expect(wfIdx).toBeGreaterThan(-1);
    expect(args[repoIdx + 1]).toBe("mnemonik-xyz/monorepo");
    // `gh attestation verify --signer-workflow` expects the fully-qualified
    // [host/]<owner>/<repo>/<path> form (per `gh attestation verify --help`).
    // A bare workflow path was silently failing with "verifying with issuer
    // sigstore.dev" — the cert's SubjectAlternativeName encodes the FULL
    // GitHub URL form, not the bare path.
    expect(args[wfIdx + 1]).toBe(
      "mnemonik-xyz/monorepo/.github/workflows/release.yml",
    );
    // --owner is intentionally absent — current `gh` CLI treats --owner and
    // --repo as mutually exclusive; --repo owner/name already pins the owner.
    expect(args.indexOf("--owner")).toBe(-1);
    // Also assert the leading subcommand sequence.
    expect(args[0]).toBe("attestation");
    expect(args[1]).toBe("verify");
  });

  it("gh_attestation_verify_rejection: throws when gh exits non-zero", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeBinaryTarball(tarFile);
    const tarBytes = await readFile(tarFile);
    const { fetch: stub } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(sha256, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub);

    const ghDir = await makeMockGh({ exit: 1, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;

    await expect(ensureBinaryCached()).rejects.toThrow(
      /gh attestation verify rejected/,
    );
    await expect(stat(manifestPath())).rejects.toThrow();
  });

  it("zip_slip_hardened_extraction: traversal entries are filtered out, no file escapes cache dir", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeMaliciousTarball(tarFile);
    const tarBytes = await readFile(tarFile);
    const { fetch: stub } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(sha256, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub);

    const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;

    // Per test-reviewer round 1 FINDING-2: the fixture now packs BOTH a
    // legitimate `mnemonic-mcp` entry AND a traversal entry named
    // `../escape.txt`. The install succeeds because the legitimate binary
    // entry extracts normally; the traversal entry must be skipped by the
    // filter (or by tar v7's built-in path-traversal protection) without
    // landing at `cacheDir/escape.txt`. If the guard were absent, install
    // would still succeed AND the escape probe would find the planted file.
    //
    // Per code-reviewer round 1 ISSUE-002: the probe path joins to the
    // binDir parent (`cacheDir/escape.txt`) which is the exact landing
    // site for an unfiltered `../escape.txt` entry when CWD=cacheDir/bin.
    await ensureBinaryCached();
    const escapeProbe = join(cacheDir(), "escape.txt");
    await expect(stat(escapeProbe)).rejects.toThrow();
    // Also assert the binary was actually extracted — proves the legitimate
    // entry got through and the filter only rejected the malicious one.
    const binarySt = await stat(join(cacheDir(), "bin", "mnemonik-mcp"));
    expect(binarySt.isFile()).toBe(true);
  });

  it("makeExtractFilter rejects SymbolicLink + Link entries (zip-slip hardening unit)", () => {
    const filter = installInternal.makeExtractFilter("/tmp/cache");
    expect(filter("bin", { type: "File" })).toBe(true);
    expect(filter("link", { type: "SymbolicLink" })).toBe(false);
    expect(filter("hardlink", { type: "Link" })).toBe(false);
    expect(filter("../escape", { type: "File" })).toBe(false);
    expect(filter("/abs/path", { type: "File" })).toBe(false);
  });

  it("symlink_entries_skipped: symlink entry in tarball is not extracted to disk", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeSymlinkTarball(tarFile);
    const tarBytes = await readFile(tarFile);
    const { fetch: stub } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(sha256, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub);

    const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;

    // The tarball's `mnemonic-mcp` entry is a symlink → filter skips it →
    // ensureBinaryCached throws because the expected binary never appears.
    await expect(ensureBinaryCached()).rejects.toThrow(
      /did not produce a binary/,
    );
    // `real-binary` is a regular file and DOES land — proves the filter is
    // selective (skips only symlinks).
    const realFile = join(cacheDir(), "bin", "real-binary");
    const s = await stat(realFile);
    expect(s.isFile()).toBe(true);
    // The symlink target was NOT created.
    const fakeSymlink = join(cacheDir(), "bin", "mnemonic-mcp");
    await expect(stat(fakeSymlink)).rejects.toThrow();
  });

  it("cached_binary_skips_re-download: pre-populated cache + manifest skips fetch entirely", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeBinaryTarball(tarFile);
    const tarBytes = await readFile(tarFile);

    // First pass: real install primes the cache.
    {
      const { fetch: stub } = makeFetchStub(
        ARTIFACT_URL,
        tarBytes,
        SUMS_URL,
        sha256SumsBody(sha256, ARTIFACT),
      );
      vi.stubGlobal("fetch", stub);
      const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
      process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;
      await ensureBinaryCached();
    }
    vi.unstubAllGlobals();
    // Wipe the gh recording so we can prove gh is NOT invoked on the second
    // call either.
    await rm(ctx.ghRecord, { force: true });

    // Second pass: fetch stub throws if called — proves zero network.
    const callCount = { n: 0 };
    const throwingFetch: typeof fetch = async () => {
      callCount.n += 1;
      throw new Error("fetch should not have been called");
    };
    vi.stubGlobal("fetch", throwingFetch);
    const path = await ensureBinaryCached();
    expect(path).toBe(binaryPath());
    expect(callCount.n).toBe(0);
    // gh should also not be invoked.
    await expect(stat(ctx.ghRecord)).rejects.toThrow();
  });

  it("cached_binary_with_mismatched_sha redownloads (manifest invalidation)", async () => {
    const tarFile = join(ctx.fixtures, ARTIFACT);
    const { sha256 } = await makeBinaryTarball(tarFile);
    const tarBytes = await readFile(tarFile);
    {
      const { fetch: stub } = makeFetchStub(
        ARTIFACT_URL,
        tarBytes,
        SUMS_URL,
        sha256SumsBody(sha256, ARTIFACT),
      );
      vi.stubGlobal("fetch", stub);
      const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
      process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;
      await ensureBinaryCached();
    }
    vi.unstubAllGlobals();
    // Corrupt the cached binary in-place (sha changes → manifest mismatch).
    await chmod(binaryPath(), 0o600);
    await writeFile(binaryPath(), "tampered\n", { mode: 0o755 });

    const { fetch: stub2 } = makeFetchStub(
      ARTIFACT_URL,
      tarBytes,
      SUMS_URL,
      sha256SumsBody(sha256, ARTIFACT),
    );
    vi.stubGlobal("fetch", stub2);
    const ghDir = await makeMockGh({ exit: 0, recordPath: ctx.ghRecord });
    process.env.PATH = `${ghDir}:${ctx.origPath ?? ""}`;
    await ensureBinaryCached();
    // After re-install, the binary's sha matches the manifest again.
    const cached = await readFile(binaryPath());
    expect(cached.length).toBeGreaterThan(0);
  });
});

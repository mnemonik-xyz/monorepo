import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import * as tar from "tar";

import {
  REPO_OWNER,
  REPO_NAME,
  SIGNER_WORKFLOW,
  binaryPath,
  cacheDir,
  detectPlatform,
  manifestPath,
  releaseBaseUrl,
} from "./paths.js";

interface BinaryVersion {
  tag: string;
  artifacts: Record<string, string>;
}

export interface Manifest {
  /** SHA256 of the cached on-disk binary; used for re-verification. */
  sha256: string;
  /** SHA256 of the downloaded tarball, as it appeared in SHA256SUMS. */
  tarball_sha256: string;
  sha256sums_url: string;
  artifact: string;
  tag: string;
  attestation_subject: string;
  publisher: { owner: string; repo: string; workflow: string };
}

const here = dirname(fileURLToPath(import.meta.url));

/**
 * Read dist/binary-version.json. Walks up from this module so it works both
 * compiled (dist/install-binary.js → ../dist/binary-version.json one level up)
 * and from source (src/install-binary.ts → ../dist/binary-version.json).
 */
async function readBinaryVersion(): Promise<BinaryVersion> {
  const candidates = [
    join(here, "..", "binary-version.json"),
    join(here, "..", "dist", "binary-version.json"),
    join(here, "..", "..", "dist", "binary-version.json"),
  ];
  for (const c of candidates) {
    try {
      const buf = await readFile(c, "utf8");
      return JSON.parse(buf) as BinaryVersion;
    } catch {
      // continue
    }
  }
  throw new Error(
    "binary-version.json not found; @mnemonik-xyz/mcp package layout broken",
  );
}

async function fileExists(p: string): Promise<boolean> {
  try {
    await stat(p);
    return true;
  } catch {
    return false;
  }
}

async function sha256Hex(buf: Buffer): Promise<string> {
  const h = createHash("sha256");
  h.update(buf);
  return h.digest("hex");
}

async function sha256OfFile(path: string): Promise<string> {
  const buf = await readFile(path);
  return sha256Hex(buf);
}

/**
 * Parse a `sha256sum -b` style manifest. Lines look like:
 *   `<64-hex>  <filename>`
 * (two spaces; `*` prefix on filename for binary mode is also tolerated).
 */
export function parseSha256Sums(body: string): Map<string, string> {
  const map = new Map<string, string>();
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.length === 0) continue;
    // Strict: 64 hex digits, whitespace, optional `*`, filename
    const m = /^([0-9a-fA-F]{64})\s+\*?(\S+)$/.exec(line);
    if (!m) continue;
    map.set(m[2]!, m[1]!.toLowerCase());
  }
  return map;
}

async function downloadToBuffer(url: string): Promise<Buffer> {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) {
    throw new Error(`download failed for ${url}: HTTP ${res.status}`);
  }
  const ab = await res.arrayBuffer();
  return Buffer.from(ab);
}

/**
 * Run `gh attestation verify` against the downloaded artifact, pinning
 * --owner, --repo, and --signer-workflow so a generic GitHub-published
 * attestation over matching bytes cannot pass (security audit round 2).
 *
 * Returns the bundle path on success; throws otherwise. The `gh` CLI is
 * required — fail closed, never silently skip.
 */
async function runGhAttestationVerify(artifactPath: string): Promise<string> {
  return new Promise<string>((resolveP, rejectP) => {
    const args = [
      "attestation",
      "verify",
      artifactPath,
      "--owner",
      REPO_OWNER,
      "--repo",
      `${REPO_OWNER}/${REPO_NAME}`,
      "--signer-workflow",
      SIGNER_WORKFLOW,
    ];
    const child = spawn("gh", args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => (stdout += String(chunk)));
    child.stderr.on("data", (chunk) => (stderr += String(chunk)));
    child.on("error", (err) => {
      const code = (err as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        rejectP(
          new Error(
            "gh CLI is required for signature verification — install from https://cli.github.com or pin a verified binary via " +
              binaryPath(),
          ),
        );
        return;
      }
      rejectP(err);
    });
    child.on("close", (exit) => {
      if (exit === 0) {
        resolveP(`${stdout}\n${stderr}`.trim());
        return;
      }
      rejectP(
        new Error(
          `gh attestation verify rejected ${artifactPath} (exit ${exit}): ${stderr.trim()}`,
        ),
      );
    });
  });
}

/**
 * tar.extract filter: reject any entry whose resolved absolute path escapes
 * `extractRoot`, and skip symlinks + hard links entirely (these are escape
 * vectors that defeat path-prefix checks: a symlink can be followed on
 * subsequent extractions, a hard link can re-point to an existing file
 * outside the cache dir).
 *
 * `tar` API note: the filter is called with the entry's *original* path
 * (relative to the tar root), so resolve against extractRoot to get the
 * realised on-disk path before checking containment.
 */
function makeExtractFilter(
  extractRoot: string,
): (entryPath: string, entry: unknown) => boolean {
  const root = resolve(extractRoot);
  return (entryPath, entry) => {
    // tar v7 hands us `Stats | ReadEntry`; ReadEntry carries `type` (e.g.
    // "File", "SymbolicLink", "Link"), Stats does not. Sniff via the
    // common shape and reject the link variants explicitly.
    const t =
      typeof entry === "object" && entry !== null && "type" in entry
        ? (entry as { type?: unknown }).type
        : undefined;
    if (t === "SymbolicLink" || t === "Link") return false;
    const resolved = resolve(root, entryPath);
    const rel = relative(root, resolved);
    if (rel === "" || rel === ".") return false;
    if (rel.startsWith("..") || isAbsolute(rel)) return false;
    return true;
  };
}

async function writeManifestSidecar(manifest: Manifest): Promise<void> {
  const tmp = manifestPath() + ".tmp";
  await writeFile(tmp, JSON.stringify(manifest, null, 2), {
    mode: 0o600,
  });
  const { rename } = await import("node:fs/promises");
  await rename(tmp, manifestPath());
}

/**
 * Verify the cached binary against its manifest sidecar. Returns true only
 * when both files exist AND the on-disk SHA256 matches the manifest's record.
 */
async function cachedBinaryValid(): Promise<boolean> {
  const bp = binaryPath();
  const mp = manifestPath();
  if (!(await fileExists(bp)) || !(await fileExists(mp))) return false;
  try {
    const m = JSON.parse(await readFile(mp, "utf8")) as Manifest;
    const onDisk = await sha256OfFile(bp);
    return onDisk === m.sha256;
  } catch {
    return false;
  }
}

/**
 * Lazy install. On first call: download tarball + SHA256SUMS, verify SHA,
 * verify `gh attestation`, extract zip-slip-hardened, chmod, write manifest.
 * Subsequent calls re-verify the cache against its manifest and return fast.
 */
export async function ensureBinaryCached(): Promise<string> {
  if (await cachedBinaryValid()) return binaryPath();

  const version = await readBinaryVersion();
  const plat = detectPlatform();
  const artifact = version.artifacts[plat];
  if (!artifact) {
    throw new Error(
      `no artifact mapped for platform ${plat} in binary-version.json`,
    );
  }

  const dir = cacheDir();
  await mkdir(dir, { recursive: true, mode: 0o700 });
  // Re-chmod in case the directory already existed with looser bits.
  await chmod(dir, 0o700);
  const binDir = join(dir, "bin");
  await mkdir(binDir, { recursive: true, mode: 0o700 });
  await chmod(binDir, 0o700);

  const base = releaseBaseUrl();
  const artifactUrl = `${base}/${version.tag}/${artifact}`;
  const sumsUrl = `${base}/${version.tag}/SHA256SUMS`;

  const tarball = await downloadToBuffer(artifactUrl);
  const sumsBody = (await downloadToBuffer(sumsUrl)).toString("utf8");

  const sums = parseSha256Sums(sumsBody);
  const expected = sums.get(artifact);
  if (!expected) {
    throw new Error(
      `SHA256SUMS does not include an entry for ${artifact} (URL: ${sumsUrl})`,
    );
  }
  const actual = await sha256Hex(tarball);
  if (actual !== expected) {
    throw new Error(
      `SHA256 mismatch for ${artifact}: expected ${expected}, got ${actual}`,
    );
  }

  // Stage the tarball on disk for `gh attestation verify` + tar.extract.
  const stagedTar = join(dir, artifact);
  await writeFile(stagedTar, tarball, { mode: 0o600 });

  try {
    const attestationOutput = await runGhAttestationVerify(stagedTar);

    await tar.extract({
      file: stagedTar,
      cwd: binDir,
      filter: makeExtractFilter(binDir),
    });

    // Locate the extracted binary. The release tarball ships
    // `mnemonic-mcp` at the root; we keep the on-disk name `mnemonik-mcp`
    // (note the `k`) so it matches the bin field.
    const extractedSource = join(binDir, "mnemonic-mcp");
    const target = binaryPath();
    if ((await fileExists(extractedSource)) && extractedSource !== target) {
      const { rename } = await import("node:fs/promises");
      await rename(extractedSource, target);
    }
    if (!(await fileExists(target))) {
      throw new Error(
        `extracted tarball did not produce a binary at ${target}; archive layout may have changed`,
      );
    }
    await chmod(target, 0o755);

    const binarySha = await sha256OfFile(target);
    const manifest: Manifest = {
      sha256: binarySha,
      tarball_sha256: actual,
      sha256sums_url: sumsUrl,
      artifact,
      tag: version.tag,
      attestation_subject: attestationOutput.slice(0, 4096),
      publisher: {
        owner: REPO_OWNER,
        repo: `${REPO_OWNER}/${REPO_NAME}`,
        workflow: SIGNER_WORKFLOW,
      },
    };
    await writeManifestSidecar(manifest);
    return target;
  } finally {
    await rm(stagedTar, { force: true });
  }
}

// Re-export internals for testing.
export const _internal = {
  makeExtractFilter,
  cachedBinaryValid,
  readBinaryVersion,
};

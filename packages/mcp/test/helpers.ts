import { createHash } from "node:crypto";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import * as tar from "tar";

/**
 * Allocate a per-test tempdir. Caller must clean it up via `cleanupTemp`.
 */
export async function freshTempDir(
  prefix = "mnemonik-mcp-test-",
): Promise<string> {
  return mkdtemp(join(tmpdir(), prefix));
}

export async function cleanupTemp(dir: string): Promise<void> {
  await rm(dir, { recursive: true, force: true });
}

/**
 * Build a tar.gz on disk containing a single file `mnemonic-mcp` with the
 * supplied body. Used as the fixture artifact for install-binary tests.
 */
export async function makeBinaryTarball(
  destTar: string,
  binaryName = "mnemonic-mcp",
  body = "#!/bin/sh\necho 'stub mnemonik-mcp'\n",
): Promise<{ tarPath: string; sha256: string; archiveName: string }> {
  const stagingDir = await freshTempDir("mnemonik-mcp-tar-");
  try {
    const filePath = join(stagingDir, binaryName);
    await writeFile(filePath, body, { mode: 0o755 });
    await chmod(filePath, 0o755);
    await mkdir(dirname(destTar), { recursive: true });
    await tar.create(
      {
        gzip: true,
        file: destTar,
        cwd: stagingDir,
      },
      [binaryName],
    );
    const { readFile } = await import("node:fs/promises");
    const buf = await readFile(destTar);
    const h = createHash("sha256");
    h.update(buf);
    return {
      tarPath: destTar,
      sha256: h.digest("hex"),
      archiveName: binaryName,
    };
  } finally {
    await rm(stagingDir, { recursive: true, force: true });
  }
}

/**
 * Build a malicious tarball with a path-traversal entry. The actual file
 * `mnemonic-mcp` is included so a normal extraction would also produce the
 * binary, but the additional `../escape.txt` entry tests the zip-slip guard.
 */
export async function makeMaliciousTarball(
  destTar: string,
): Promise<{ tarPath: string; sha256: string }> {
  const stagingDir = await freshTempDir("mnemonik-mcp-evil-");
  try {
    const inner = join(stagingDir, "inner");
    await mkdir(inner, { recursive: true });
    // The benign root entry — a normal extraction would succeed without it.
    await writeFile(join(stagingDir, "mnemonic-mcp"), "#!/bin/sh\nexit 0\n", {
      mode: 0o755,
    });
    // The traversal entry: pack with portable=true so `..` survives.
    await writeFile(join(inner, "escape.txt"), "pwned\n");
    await mkdir(dirname(destTar), { recursive: true });
    // tar accepts `..` entries when we pass them explicitly via `prefix` or
    // by listing a path starting with `..`. We construct the tar by hand
    // via tar.create + a transform that prepends `../` to the inner entry
    // name during pack.
    await tar.create(
      {
        gzip: true,
        file: destTar,
        cwd: stagingDir,
        portable: true,
        prefix: "../escape",
      },
      ["inner/escape.txt"],
    );
    const { readFile } = await import("node:fs/promises");
    const buf = await readFile(destTar);
    const h = createHash("sha256");
    h.update(buf);
    return { tarPath: destTar, sha256: h.digest("hex") };
  } finally {
    await rm(stagingDir, { recursive: true, force: true });
  }
}

/**
 * Build a tarball whose only entry is a symlink. Used to test the
 * SymbolicLink skip filter.
 */
export async function makeSymlinkTarball(
  destTar: string,
): Promise<{ tarPath: string; sha256: string }> {
  const stagingDir = await freshTempDir("mnemonik-mcp-symlink-");
  try {
    const { symlink } = await import("node:fs/promises");
    // The symlink target doesn't need to exist for tar.create — it captures
    // the link metadata, not the target file.
    await symlink("/etc/passwd", join(stagingDir, "mnemonic-mcp"));
    // Include a real file too so extraction has *something* legitimate to
    // process and we can prove the symlink was skipped specifically.
    await writeFile(join(stagingDir, "real-binary"), "ok\n");
    await mkdir(dirname(destTar), { recursive: true });
    await tar.create(
      {
        gzip: true,
        file: destTar,
        cwd: stagingDir,
        portable: true,
      },
      ["mnemonic-mcp", "real-binary"],
    );
    const { readFile } = await import("node:fs/promises");
    const buf = await readFile(destTar);
    const h = createHash("sha256");
    h.update(buf);
    return { tarPath: destTar, sha256: h.digest("hex") };
  } finally {
    await rm(stagingDir, { recursive: true, force: true });
  }
}

/**
 * Write a mock `gh` shim that records its argv into a file and exits 0 or 1.
 * Returns the directory to prepend to PATH.
 */
export async function makeMockGh(opts: {
  exit: 0 | 1;
  recordPath: string;
}): Promise<string> {
  const dir = await freshTempDir("mnemonik-mock-gh-");
  const sh = `#!/bin/sh
# Mock gh CLI for install-binary tests. Records argv into the supplied file
# (newline-separated) and exits with the requested code.
for arg in "$@"; do
  printf "%s\\n" "$arg" >> "${opts.recordPath}"
done
exit ${opts.exit}
`;
  const ghPath = join(dir, "gh");
  await writeFile(ghPath, sh, { mode: 0o755 });
  await chmod(ghPath, 0o755);
  return dir;
}

/**
 * Build the SHA256SUMS body matching a single artifact.
 */
export function sha256SumsBody(
  sha: string,
  filename: string,
  extra: Array<[string, string]> = [],
): string {
  const lines = [`${sha}  ${filename}`];
  for (const [s, f] of extra) lines.push(`${s}  ${f}`);
  return lines.join("\n") + "\n";
}

/**
 * Build a minimal HTTP fetch double that serves a tarball + SHA256SUMS.
 * Returns a stub usable with `vi.stubGlobal("fetch", ...)`.
 */
export function makeFetchStub(
  artifactUrl: string,
  artifactBytes: Buffer,
  sumsUrl: string,
  sumsBody: string,
): { fetch: typeof fetch; calls: string[] } {
  const calls: string[] = [];
  const stub: typeof fetch = async (input) => {
    const url = typeof input === "string" ? input : input.toString();
    calls.push(url);
    if (url === artifactUrl) {
      return new Response(artifactBytes, { status: 200 });
    }
    if (url === sumsUrl) {
      return new Response(sumsBody, { status: 200 });
    }
    return new Response("not found", { status: 404 });
  };
  return { fetch: stub, calls };
}

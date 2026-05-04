// CLI integration: spawn the actual `mnemonic` binary against the SDK's
// in-process mock server. Validates argv parsing, exit codes, env-var
// propagation (MNEMONIC_BASE_URL, MNEMONIC_CONFIG_DIR), and end-to-end
// init → login --token → sign → recall → verify.
//
// IMPORTANT: Bun's test runner (used in the CI matrix per Decision 11)
// runs `it()` blocks within a file *concurrently*. Each test here owns
// its own `{cfgDir, server}` pair to remain hermetic.
//
// Even with hermetic state, spawning many bun subprocesses concurrently
// produces flaky results on macOS (filesystem races on temp-dir creation
// and WASM init contention). To stabilize, the entire scenario set runs
// inside ONE `it()` block as a sequence of awaited steps.

import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { beforeAll, describe, expect, it } from "vitest";

import {
  startMockServer,
  type MockServer,
} from "../../../sdk/test/mock-server.js";

// NOTE: execa v9 was added to devDependencies per the task brief, but its
// stdio capture is unreliable under Bun's runtime (child_process.spawn
// works correctly in Bun while execa's high-level wrapper returns empty
// strings — Bun-vs-Node parity gap as of bun 1.3.13). For determinism we
// use `node:child_process.spawn` directly.

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const CLI_BIN = join(__dirname, "..", "..", "dist", "bin", "mnemonic.js");

beforeAll(() => {
  if (!existsSync(CLI_BIN)) {
    throw new Error(
      `CLI binary not found at ${CLI_BIN} — run \`tsc -b\` in packages/cli first`
    );
  }
});

interface CliResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

/**
 * Spawn the CLI binary. Uses `process.execPath` so the subprocess
 * inherits the test runner's runtime (bun under `bun test`). The
 * `--target web` WASM artefact in `core/pkg-web/` initializes via
 * `fetch(file://...)` which is supported in Bun + Deno + browsers
 * but NOT in Node's undici. CI runs all matrix entries via `bun test`,
 * so `process.execPath` is bun in every cell.
 */
function spawnCli(
  cfgDir: string,
  baseUrl: string,
  args: string[]
): Promise<CliResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [CLI_BIN, ...args], {
      env: {
        ...process.env,
        MNEMONIC_CONFIG_DIR: cfgDir,
        MNEMONIC_BASE_URL: baseUrl,
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let exited = false;
    let stdoutEnded = false;
    let stderrEnded = false;
    let exitCode = 0;
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (c) => (stdout += c));
    child.stderr.on("data", (c) => (stderr += c));
    child.stdout.on("end", () => {
      stdoutEnded = true;
      maybeResolve();
    });
    child.stderr.on("end", () => {
      stderrEnded = true;
      maybeResolve();
    });
    function maybeResolve(): void {
      if (exited && stdoutEnded && stderrEnded) {
        clearTimeout(timer);
        resolve({ exitCode, stdout, stderr });
      }
    }
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error(`runCli: timed out (${args.join(" ")})`));
    }, 15_000);
    child.once("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
    child.once("close", (code) => {
      exitCode = code ?? 0;
      exited = true;
      maybeResolve();
    });
  });
}

/** Build a JWT-shaped (HS256) test token whose payload has a future exp. */
function makeToken(sub = "test-user-pubkey"): string {
  const b64 = (o: unknown) =>
    Buffer.from(JSON.stringify(o), "utf8")
      .toString("base64")
      .replace(/=+$/g, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
  const header = { alg: "HS256", typ: "JWT" };
  const now = Math.floor(Date.now() / 1000);
  const payload = { sub, iat: now, exp: now + 3600 };
  return `${b64(header)}.${b64(payload)}.test-signature`;
}

// The compiled `mnemonic` binary loads the SDK's `--target web` WASM
// artifact, whose `init()` resolves the .wasm via `fetch(file://...)`.
// That fetch path is supported under Bun, Deno, and browsers, but NOT
// under Node 20's undici. Decision 11 / Task 8 pin the CI matrix to
// `bun test` precisely because of this gap. When this file is executed
// under Node (e.g. `npm test` invoked from a `prepublishOnly` script,
// or by a contributor without bun installed), skip the scenario rather
// than fail the whole suite. CI still asserts these flows under bun.
const isBun = typeof (globalThis as { Bun?: unknown }).Bun !== "undefined";
const itOrSkip = isBun ? it : it.skip;

describe("CLI scenarios", () => {
  // Single it() — see file header for why we serialize the scenarios.
  itOrSkip(
    "init → login → recall → verify(verified|tampered|not_found) → sign",
    async () => {
      const cfgDir = mkdtempSync(join(tmpdir(), "mnemonic-cli-int-"));
      const server: MockServer = await startMockServer();
      const run = (args: string[]): Promise<CliResult> =>
        spawnCli(cfgDir, server.url, args);

      try {
        // ── init (no flags) — 0.1.6 BREAKING: must error with "pick a mode"
        const r0 = await run(["init"]);
        expect(r0.exitCode).toBe(1);
        expect(r0.stderr).toMatch(/pick a mode/i);

        // ── init --standalone ───────────────────────────────────────────────
        const r1 = await run(["--json", "init", "--standalone"]);
        expect(r1.exitCode).toBe(0);
        expect(existsSync(join(cfgDir, "identity.json"))).toBe(true);
        const initData = JSON.parse(r1.stdout);
        expect(typeof initData.pubkey).toBe("string");
        expect(initData.pubkey.length).toBeGreaterThan(0);

        // ── init --standalone refuse-overwrite ──────────────────────────────
        const r2 = await run(["init", "--standalone"]);
        expect(r2.exitCode).toBe(1); // UserError
        expect(r2.stderr).toMatch(/already exists/);

        // ── init --standalone --force overwrites ────────────────────────────
        const before = readFileSync(join(cfgDir, "identity.json"), "utf8");
        const r3 = await run(["init", "--standalone", "--force"]);
        expect(r3.exitCode).toBe(0);
        const after = readFileSync(join(cfgDir, "identity.json"), "utf8");
        expect(after).not.toBe(before);

        // ── login --token (headless, valid) ─────────────────────────────────
        // The 0.1.5 pre-flight check rejects sign/recall/verify when the
        // identity's pubkey ≠ the JWT's `sub`. So mint the token with the
        // identity's actual pubkey, NOT the placeholder "test-user-pubkey".
        const idAfterForce = JSON.parse(
          readFileSync(join(cfgDir, "identity.json"), "utf8")
        ) as { pubkey_base58: string };
        const r4 = await run([
          "login",
          "--token",
          makeToken(idAfterForce.pubkey_base58),
        ]);
        expect(r4.exitCode).toBe(0);
        expect(existsSync(join(cfgDir, "token.json"))).toBe(true);
        const tok = JSON.parse(
          readFileSync(join(cfgDir, "token.json"), "utf8")
        );
        expect(tok.sub).toBe(idAfterForce.pubkey_base58);
        expect(typeof tok.jwt).toBe("string");

        // ── login --token (malformed → exit 4) ──────────────────────────────
        const r5 = await run(["login", "--token", "not.a.jwt"]);
        expect(r5.exitCode).toBe(4);

        // ── recall ──────────────────────────────────────────────────────────
        // Re-login with a valid token (the malformed-token attempt above
        // still leaves the prior good token persisted, but we re-login to
        // exercise the saveToken path explicitly). Sub must still match the
        // current identity pubkey for the recall pre-flight to pass.
        await run(["login", "--token", makeToken(idAfterForce.pubkey_base58)]);
        const r6 = await run(["--json", "recall", "test query"]);
        expect(r6.exitCode).toBe(0);
        const recallData = JSON.parse(r6.stdout);
        expect(recallData.total).toBeGreaterThanOrEqual(1);
        expect(recallData.hits[0].attestationId).toBe("att-mock-1");

        // ── verify(verified) ────────────────────────────────────────────────
        const r7 = await run(["verify", "att-good-1"]);
        expect(r7.exitCode).toBe(0);
        expect(r7.stdout).toMatch(/verified/);

        // ── verify(tampered) → exit 3 ───────────────────────────────────────
        const r8 = await run(["verify", "tampered-attestation"]);
        expect(r8.exitCode).toBe(3);

        // ── verify(not_found) → exit 1 ──────────────────────────────────────
        const r9 = await run(["verify", "missing-attestation"]);
        expect(r9.exitCode).toBe(1);

        // ── sign (full pending-bundle round-trip) ──────────────────────────
        const r10 = await run(["--json", "sign", "hello world"]);
        expect(r10.exitCode).toBe(0);
        const signData = JSON.parse(r10.stdout);
        expect(signData.attestationId).toMatch(/^att-/);
        const paths = server.calls.map((c) => `${c.method} ${c.path}`);
        expect(paths.some((p) => p === "POST /mcp")).toBe(true);
        expect(paths.some((p) => p.startsWith("GET /api/pending/"))).toBe(true);
        expect(paths.some((p) => p === "POST /api/sign-callback")).toBe(true);
      } finally {
        try {
          server.closeAllConnections();
        } catch {
          /* ignore */
        }
        try {
          await server.close();
        } catch {
          /* ignore */
        }
        try {
          rmSync(cfgDir, { recursive: true, force: true });
        } catch {
          /* ignore */
        }
      }
    },
    60_000
  );
});

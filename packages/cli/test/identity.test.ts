// `mnemonic identity import|export` — TDD anchor: first redeem succeeds,
// second returns 410 Gone (atomic, single-use), CLI surfaces exit 1.

import { existsSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  runIdentityExport,
  runIdentityImport,
} from "../src/commands/identity.js";
import { loadIdentityJson, saveIdentityJson } from "../src/config.js";
import { UserError } from "../src/errors.js";
import { clearWasmMock, installWasmMock, withTmpConfigDir } from "./helpers.js";

describe("runIdentityImport / runIdentityExport", () => {
  let cleanup = (): void => {};
  let dir = "";
  let realFetch: typeof globalThis.fetch | undefined;
  let mock: ReturnType<typeof installWasmMock>;

  beforeEach(() => {
    const r = withTmpConfigDir();
    dir = r.dir;
    cleanup = r.cleanup;
    mock = installWasmMock();
    realFetch = globalThis.fetch;
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    if (realFetch) globalThis.fetch = realFetch;
    cleanup();
  });

  it("TDD anchor: first --ticket succeeds, second 410 → exit 1", async () => {
    const kp = mock.generate_keypair();
    let calls = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString();
      expect(url).toContain("/api/cli-bootstrap/redeem/abc-uuid");
      calls++;
      if (calls === 1) {
        return new Response(JSON.stringify(kp), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify({ error: "already redeemed" }), {
        status: 410,
        headers: { "content-type": "application/json" },
      });
    });
    globalThis.fetch = fetchMock as never;

    // First call.
    await runIdentityImport({
      ticket: "abc-uuid",
      baseUrl: "http://srv",
    });
    expect(existsSync(join(dir, "identity.json"))).toBe(true);
    expect(loadIdentityJson().pubkey_base58).toBe(kp.pubkey_base58);

    // Second call — server returns 410 → CLI surfaces UserError (exit 1).
    await expect(
      runIdentityImport({
        ticket: "abc-uuid",
        baseUrl: "http://srv",
        force: true, // bypass overwrite refusal so we hit the 410 path
      })
    ).rejects.toMatchObject({ exitCode: 1 });

    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("--ticket refuses to overwrite without --force", async () => {
    saveIdentityJson({
      secret: Array.from({ length: 64 }, () => 7),
      pubkey_base58: "Existing",
    });
    const fetchMock = vi.fn(async () => new Response("{}", { status: 200 }));
    globalThis.fetch = fetchMock as never;
    await expect(
      runIdentityImport({ ticket: "x", baseUrl: "http://srv" })
    ).rejects.toBeInstanceOf(UserError);
    expect(fetchMock).toHaveBeenCalledTimes(0);
  });

  it("--file <path> reads and validates JSON", async () => {
    const kp = mock.generate_keypair();
    const path = join(dir, "imported.json");
    writeFileSync(path, JSON.stringify(kp));
    await runIdentityImport({ file: path });
    expect(loadIdentityJson().pubkey_base58).toBe(kp.pubkey_base58);
  });

  it("--file rejects malformed JSON", async () => {
    const path = join(dir, "bad.json");
    writeFileSync(path, "{not json");
    await expect(runIdentityImport({ file: path })).rejects.toBeInstanceOf(
      UserError
    );
  });

  it("--file rejects bad shape", async () => {
    const path = join(dir, "wrong-shape.json");
    writeFileSync(path, JSON.stringify({ secret: [1, 2], pubkey_base58: "x" }));
    await expect(runIdentityImport({ file: path })).rejects.toBeInstanceOf(
      UserError
    );
  });

  it("requires either --ticket or --file", async () => {
    await expect(runIdentityImport({})).rejects.toBeInstanceOf(UserError);
  });

  it("rejects --ticket and --file together", async () => {
    await expect(
      runIdentityImport({ ticket: "x", file: "y" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("export --file writes mode 0600", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    const out = join(dir, "exported.json");
    await runIdentityExport({ file: out });
    expect(existsSync(out)).toBe(true);
    if (process.platform !== "win32") {
      const st = statSync(out);
      expect((st.mode & 0o777).toString(8)).toBe("600");
    }
  });

  it("export refuses without --file", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    await expect(runIdentityExport({})).rejects.toBeInstanceOf(UserError);
  });

  it("export refuses if no identity to export", async () => {
    const out = join(dir, "exported.json");
    await expect(runIdentityExport({ file: out })).rejects.toBeInstanceOf(
      UserError
    );
  });
});

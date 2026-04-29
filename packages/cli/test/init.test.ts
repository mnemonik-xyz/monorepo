// `mnemonic init` — creates ~/.mnemonic/identity.json mode 0600 and refuses
// to overwrite without --force.

import { existsSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runInit } from "../src/commands/init.js";
import { UserError } from "../src/errors.js";
import { clearWasmMock, installWasmMock, withTmpConfigDir } from "./helpers.js";

describe("runInit", () => {
  let cleanup = (): void => {};
  let dir = "";
  let stdoutSpy: ReturnType<typeof vi.spyOn>;
  let stderrSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    const r = withTmpConfigDir();
    dir = r.dir;
    cleanup = r.cleanup;
    installWasmMock();
    stdoutSpy = vi
      .spyOn(process.stdout, "write")
      .mockImplementation(() => true);
    stderrSpy = vi
      .spyOn(process.stderr, "write")
      .mockImplementation(() => true);
  });

  afterEach(() => {
    stdoutSpy.mockRestore();
    stderrSpy.mockRestore();
    clearWasmMock();
    cleanup();
  });

  it("creates identity.json with mode 0600", async () => {
    await runInit({});
    const path = join(dir, "identity.json");
    expect(existsSync(path)).toBe(true);
    if (process.platform !== "win32") {
      const st = statSync(path);
      expect((st.mode & 0o777).toString(8)).toBe("600");
    }
  });

  it("refuses to overwrite without --force", async () => {
    writeFileSync(
      join(dir, "identity.json"),
      JSON.stringify({
        secret: Array.from({ length: 64 }, () => 1),
        pubkey_base58: "Existing",
      })
    );
    await expect(runInit({})).rejects.toBeInstanceOf(UserError);
  });

  it("overwrites with --force", async () => {
    writeFileSync(
      join(dir, "identity.json"),
      JSON.stringify({
        secret: Array.from({ length: 64 }, () => 1),
        pubkey_base58: "OldKey",
      })
    );
    await runInit({ force: true });
    // The mock generates a fresh keypair — pubkey will not be "OldKey".
    const { loadIdentityJson } = await import("../src/config.js");
    const id = loadIdentityJson();
    expect(id.pubkey_base58).not.toBe("OldKey");
  });
});

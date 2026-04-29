// TDD anchor: `--json --quiet whoami` emits one JSON object on stdout
// and nothing on stderr.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runWhoami } from "../src/commands/whoami.js";
import { format, hint, warn } from "../src/output.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";
import { saveIdentityJson, saveToken } from "../src/config.js";

describe("output.format", () => {
  let stdout = "";
  let stderr = "";
  let stdoutSpy: ReturnType<typeof vi.spyOn>;
  let stderrSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    stdout = "";
    stderr = "";
    stdoutSpy = vi
      .spyOn(process.stdout, "write")
      .mockImplementation((chunk: unknown) => {
        stdout += String(chunk);
        return true;
      });
    stderrSpy = vi
      .spyOn(process.stderr, "write")
      .mockImplementation((chunk: unknown) => {
        stderr += String(chunk);
        return true;
      });
  });

  afterEach(() => {
    stdoutSpy.mockRestore();
    stderrSpy.mockRestore();
  });

  it("--json: emits a single JSON object to stdout", () => {
    format({ a: 1 }, { json: true });
    expect(stdout.trim()).toBe(JSON.stringify({ a: 1 }, null, 2));
    expect(stderr).toBe("");
  });

  it("--quiet alone: suppresses stdout entirely", () => {
    format({ a: 1 }, { quiet: true });
    expect(stdout).toBe("");
  });

  it("--json --quiet: only the JSON payload is emitted on stdout", () => {
    format({ ok: true }, { json: true, quiet: true });
    expect(stdout.trim()).toBe(JSON.stringify({ ok: true }, null, 2));
    expect(stderr).toBe("");
  });

  it("hint() goes to stderr; --quiet suppresses it", () => {
    hint("hello", {});
    expect(stderr).toContain("hello");
    stderr = "";
    hint("hidden", { quiet: true });
    expect(stderr).toBe("");
  });

  it("warn() always writes to stderr (even under --quiet)", () => {
    warn("watch out", { quiet: true });
    expect(stderr).toContain("watch out");
  });
});

// TDD anchor: `mnemonic --json --quiet whoami` stdout is one JSON object,
// stderr empty. Exercises the full whoami command, not just `format()`.
describe("--json --quiet whoami (TDD anchor)", () => {
  let cleanup = (): void => {};
  let stdout = "";
  let stderr = "";
  let stdoutSpy: ReturnType<typeof vi.spyOn>;
  let stderrSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    cleanup = withTmpConfigDir().cleanup;
    installWasmMock();
    // Seed a fake identity + token so whoami has both halves to print.
    saveIdentityJson({
      secret: Array.from({ length: 64 }, (_, i) => i),
      pubkey_base58: "FakePubkey1111111111111111111111111111111111",
    });
    const jwt = makeJwt("FakePubkey1111111111111111111111111111111111");
    saveToken({
      jwt,
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub: "FakePubkey1111111111111111111111111111111111",
    });
    stdout = "";
    stderr = "";
    stdoutSpy = vi
      .spyOn(process.stdout, "write")
      .mockImplementation((chunk: unknown) => {
        stdout += String(chunk);
        return true;
      });
    stderrSpy = vi
      .spyOn(process.stderr, "write")
      .mockImplementation((chunk: unknown) => {
        stderr += String(chunk);
        return true;
      });
  });

  afterEach(() => {
    stdoutSpy.mockRestore();
    stderrSpy.mockRestore();
    clearWasmMock();
    cleanup();
  });

  it("stdout is one JSON object, stderr empty", async () => {
    await runWhoami({ json: true, quiet: true });
    // Exactly one JSON.parse-able blob.
    const parsed = JSON.parse(stdout);
    expect(parsed).toBeTypeOf("object");
    expect(parsed.pubkey).toBe("FakePubkey1111111111111111111111111111111111");
    expect(stderr).toBe("");
  });
});

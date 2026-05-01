// `restrictFileMode` — verifies the Windows icacls invocation uses
// `execFileSync` with an argv array (no shell), defeating CWE-78 quote-
// escape attacks via user-supplied --file paths.
//
// `loadIdentityJson` — covers the canonical `identity.json` path AND the
// 0.1.4 fallback to legacy `id.json` (Solana keypair file format) used by
// existing self-host server installs (`MNEMONIC_KEYPAIR_PATH`).

import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock node:child_process before importing config.ts so the import binds the
// mock'd execFileSync. vitest hoists vi.mock() to the top of the module. The
// mock factory closes over a Mock function reachable via the import below.
const execFileSyncMock = vi.fn();
vi.mock("node:child_process", () => ({
  execFileSync: (...args: unknown[]) => execFileSyncMock(...args),
}));

import { loadIdentityJson, restrictFileMode } from "../src/config.js";
import { UserError } from "../src/errors.js";
import { withTmpConfigDir } from "./helpers.js";

describe("restrictFileMode (Windows ACL hardening)", () => {
  let origPlatform: PropertyDescriptor | undefined;
  let origUsername: string | undefined;

  beforeEach(() => {
    origPlatform = Object.getOwnPropertyDescriptor(process, "platform");
    origUsername = process.env.USERNAME;
    execFileSyncMock.mockClear();
  });

  afterEach(() => {
    if (origPlatform) {
      Object.defineProperty(process, "platform", origPlatform);
    }
    if (origUsername === undefined) delete process.env.USERNAME;
    else process.env.USERNAME = origUsername;
  });

  it("on Windows: invokes execFileSync('icacls', [path, ...flags, 'user:F'])", () => {
    Object.defineProperty(process, "platform", {
      value: "win32",
      configurable: true,
    });
    process.env.USERNAME = "alice";

    const evilPath = 'C:\\Users\\me\\foo".bat';
    restrictFileMode(evilPath);

    expect(execFileSyncMock).toHaveBeenCalledTimes(1);
    const call = execFileSyncMock.mock.calls[0];
    expect(call?.[0]).toBe("icacls");
    // argv array: [path, '/inheritance:r', '/grant:r', 'alice:F']. The
    // embedded `"` in the path is preserved literally — no shell parsing.
    expect(call?.[1]).toEqual([
      evilPath,
      "/inheritance:r",
      "/grant:r",
      "alice:F",
    ]);
    // shell is NOT enabled (default false) — verify by absence of shell:true
    // in the options. Options is the 3rd arg.
    const opts = call?.[2] as { stdio?: string; shell?: boolean } | undefined;
    expect(opts?.shell).toBeUndefined();
    expect(opts?.stdio).toBe("ignore");
  });

  it("on Windows with no USERNAME/USER: skips the icacls call (best-effort)", () => {
    Object.defineProperty(process, "platform", {
      value: "win32",
      configurable: true,
    });
    delete process.env.USERNAME;
    const prevUser = process.env.USER;
    delete process.env.USER;

    try {
      restrictFileMode("C:\\some\\file");
      expect(execFileSyncMock).not.toHaveBeenCalled();
    } finally {
      if (prevUser !== undefined) process.env.USER = prevUser;
    }
  });

  it("on Windows: swallows execFileSync errors (non-fatal)", () => {
    Object.defineProperty(process, "platform", {
      value: "win32",
      configurable: true,
    });
    process.env.USERNAME = "bob";
    execFileSyncMock.mockImplementationOnce(() => {
      throw new Error("icacls not found");
    });
    expect(() => restrictFileMode("C:\\file")).not.toThrow();
  });
});

describe("loadIdentityJson (identity.json + legacy id.json fallback)", () => {
  let cleanup = (): void => {};
  let dir = "";
  let stderrSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    const r = withTmpConfigDir();
    dir = r.dir;
    cleanup = r.cleanup;
    stderrSpy = vi
      .spyOn(process.stderr, "write")
      .mockImplementation(() => true);
  });

  afterEach(() => {
    stderrSpy.mockRestore();
    cleanup();
  });

  it("returns identity.json when present (canonical path)", () => {
    const expected = {
      secret: Array.from({ length: 64 }, (_, i) => i),
      pubkey_base58: "TestPubkey123",
    };
    writeFileSync(join(dir, "identity.json"), JSON.stringify(expected));
    const got = loadIdentityJson();
    expect(got).toEqual(expected);
    // No "legacy" stderr note when canonical file is present.
    expect(stderrSpy).not.toHaveBeenCalled();
  });

  it("falls back to id.json (Solana keypair file format)", () => {
    // Solana keypair file = bare JSON array of 64 numbers, [seed(32) || pub(32)].
    // Use a fixed pattern so the derived pubkey_base58 is deterministic for
    // this test. Bytes 32..64 are all 0x01 → base58 of 32×0x01.
    const secret = Array.from({ length: 64 }, (_, i) => (i < 32 ? i + 1 : 1));
    writeFileSync(join(dir, "id.json"), JSON.stringify(secret));

    const got = loadIdentityJson();
    expect(got.secret).toEqual(secret);
    expect(typeof got.pubkey_base58).toBe("string");
    expect(got.pubkey_base58.length).toBeGreaterThan(0);
    // Round-trip: re-deriving pubkey_base58 from secret[32..64] must match.
    // This catches any silent slice/encode regression.
    const ALPH = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    const tail = secret.slice(32, 64);
    let zeros = 0;
    while (zeros < tail.length && tail[zeros] === 0) zeros++;
    const digits: number[] = [];
    for (let i = zeros; i < tail.length; i++) {
      let carry = tail[i]!;
      for (let j = 0; j < digits.length; j++) {
        carry += digits[j]! * 256;
        digits[j] = carry % 58;
        carry = (carry / 58) | 0;
      }
      while (carry > 0) {
        digits.push(carry % 58);
        carry = (carry / 58) | 0;
      }
    }
    let expected = "";
    for (let i = 0; i < zeros; i++) expected += ALPH[0];
    for (let i = digits.length - 1; i >= 0; i--) expected += ALPH[digits[i]!];
    expect(got.pubkey_base58).toBe(expected);

    // Stderr note must fire so users see what's happening.
    const calls = stderrSpy.mock.calls.flat().join("");
    expect(calls).toContain("legacy id.json");
  });

  it("throws helpful UserError when both files are absent", () => {
    let err: unknown;
    try {
      loadIdentityJson();
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(UserError);
    const msg = (err as UserError).message;
    expect(msg).toContain("mnemonic init");
    expect(msg).toContain("mnemonic identity import --ticket");
    expect(msg).toContain("mnemonic identity import --file");
  });

  it("throws when id.json is malformed (not a 64-number array)", () => {
    writeFileSync(join(dir, "id.json"), JSON.stringify([1, 2, 3]));
    expect(() => loadIdentityJson()).toThrow(UserError);
  });
});

// `restrictFileMode` — verifies the Windows icacls invocation uses
// `execFileSync` with an argv array (no shell), defeating CWE-78 quote-
// escape attacks via user-supplied --file paths.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock node:child_process before importing config.ts so the import binds the
// mock'd execFileSync. vitest hoists vi.mock() to the top of the module. The
// mock factory closes over a Mock function reachable via the import below.
const execFileSyncMock = vi.fn();
vi.mock("node:child_process", () => ({
  execFileSync: (...args: unknown[]) => execFileSyncMock(...args),
}));

import { restrictFileMode } from "../src/config.js";

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

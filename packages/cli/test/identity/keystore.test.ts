/**
 * Unit tests for KeyStore interface implementations.
 *
 * OsKeyStore tests are gated behind MNEMONIC_TEST_KEYCHAIN=1 (Wave 3 Task 9).
 * All other tests run in CI unconditionally.
 */

import os from "node:os";
import path from "node:path";
import fs from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { describe, it, expect, afterEach } from "vitest";

import {
  canonicalEntryJson,
  KeystoreError,
} from "../../src/identity/keystore.js";
import { MemoryKeyStore } from "../../src/identity/keystore-memory.js";
import { FileKeyStore } from "../../src/identity/keystore-file.js";
import { OsKeyStore } from "../../src/identity/keystore-os.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function tmpFile(): string {
  return path.join(
    os.tmpdir(),
    "mnemonik-ks-test-" + randomBytes(4).toString("hex") + ".json",
  );
}

function makeEntry(byte = 0): { secret: number[]; pubkey_base58: string } {
  return {
    secret: Array.from({ length: 64 }, () => byte),
    pubkey_base58: "testpubkey",
  };
}

// ---------------------------------------------------------------------------
// Canonical JSON — golden byte-equality test (Decision 13)
// ---------------------------------------------------------------------------

describe("canonicalEntryJson", () => {
  it("canonical JSON bytes are stable (golden)", () => {
    const entry = {
      secret: Array.from({ length: 64 }, () => 0),
      pubkey_base58: "test",
    };
    // Expected string is pre-computed manually and must match the Rust golden
    // test output byte-for-byte (serde_json compact, secret first).
    const expected =
      '{"secret":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"pubkey_base58":"test"}';
    expect(canonicalEntryJson(entry)).toBe(expected);
  });

  it("field order is always secret then pubkey_base58", () => {
    const entry = { secret: [1, 2, 3], pubkey_base58: "abc" };
    const json = canonicalEntryJson(entry);
    const secretIdx = json.indexOf('"secret"');
    const pubkeyIdx = json.indexOf('"pubkey_base58"');
    expect(secretIdx).toBeLessThan(pubkeyIdx);
  });
});

// ---------------------------------------------------------------------------
// MemoryKeyStore
// ---------------------------------------------------------------------------

describe("MemoryKeyStore", () => {
  it("round-trips set then get", async () => {
    const store = new MemoryKeyStore();
    const entry = makeEntry(42);
    await store.set(entry);
    const got = await store.get();
    expect(got).toEqual(entry);
  });

  it("get returns null before any set", async () => {
    const store = new MemoryKeyStore();
    expect(await store.get()).toBeNull();
  });

  it("remove clears the stored entry", async () => {
    const store = new MemoryKeyStore();
    await store.set(makeEntry(7));
    await store.remove();
    expect(await store.get()).toBeNull();
  });

  it("double remove is idempotent", async () => {
    const store = new MemoryKeyStore();
    await store.set(makeEntry(1));
    await store.remove();
    await expect(store.remove()).resolves.toBeUndefined();
    expect(await store.get()).toBeNull();
  });

  it("available returns true by default", async () => {
    const store = new MemoryKeyStore();
    expect(await store.available()).toBe(true);
  });

  it("available returns false when rigged via setAvailable", async () => {
    const store = new MemoryKeyStore();
    store.setAvailable(false);
    expect(await store.available()).toBe(false);
  });

  it("name is 'memory'", () => {
    expect(new MemoryKeyStore().name).toBe("memory");
  });
});

// ---------------------------------------------------------------------------
// FileKeyStore
// ---------------------------------------------------------------------------

describe("FileKeyStore", () => {
  const tmpFiles: string[] = [];

  function track(p: string): string {
    tmpFiles.push(p);
    return p;
  }

  afterEach(async () => {
    // Best-effort cleanup; ignore errors if files were already removed.
    for (const f of tmpFiles.splice(0)) {
      await fs.unlink(f).catch(() => undefined);
      await fs.unlink(f + ".tmp").catch(() => undefined);
    }
  });

  it("set then get round-trips the full entry", async () => {
    const p = track(tmpFile());
    const store = new FileKeyStore(p);
    const entry = makeEntry(99);
    await store.set(entry);
    const got = await store.get();
    expect(got).toEqual(entry);
  });

  it("get returns null when file is absent (ENOENT)", async () => {
    const p = track(tmpFile());
    const store = new FileKeyStore(p);
    expect(await store.get()).toBeNull();
  });

  it("mode is 0o600 on Unix after set", async () => {
    if (process.platform === "win32") return;

    const p = track(tmpFile());
    const store = new FileKeyStore(p);
    await store.set(makeEntry(55));
    const stat = await fs.stat(p);
    // eslint-disable-next-line no-bitwise
    expect(stat.mode & 0o777).toBe(0o600);
  });

  it("get returns null for stub shape (no secret key)", async () => {
    const p = track(tmpFile());
    // Write a stub: has pubkey_base58 + keychain_ref but no secret.
    await fs.writeFile(
      p,
      JSON.stringify({ pubkey_base58: "x", keychain_ref: "y" }),
      "utf8",
    );
    const store = new FileKeyStore(p);
    expect(await store.get()).toBeNull();
  });

  it("get throws KeystoreError('serde') on garbage JSON", async () => {
    const p = track(tmpFile());
    await fs.writeFile(p, "not-valid-json{{{", "utf8");
    const store = new FileKeyStore(p);
    await expect(store.get()).rejects.toSatisfy(
      (e: unknown) => e instanceof KeystoreError && e.kind === "serde",
    );
  });

  it("set is atomic — no .tmp file left after success", async () => {
    const p = track(tmpFile());
    const store = new FileKeyStore(p);
    await store.set(makeEntry(3));

    // Enumerate any .tmp.* files beside the target.
    const dir = path.dirname(p);
    const base = path.basename(p);
    const entries = await fs.readdir(dir);
    const leftoverTmp = entries.filter((f) => f.startsWith(base + ".tmp."));
    expect(leftoverTmp).toHaveLength(0);
  });

  it("remove deletes the file", async () => {
    const p = track(tmpFile());
    const store = new FileKeyStore(p);
    await store.set(makeEntry(10));
    await store.remove();
    expect(await store.get()).toBeNull();
  });

  it("remove is idempotent when file is absent", async () => {
    const p = track(tmpFile());
    const store = new FileKeyStore(p);
    await expect(store.remove()).resolves.toBeUndefined();
  });

  it("available returns true for a writable tmp dir", async () => {
    const store = new FileKeyStore(
      path.join(os.tmpdir(), "mnemonik-avail-test.json"),
    );
    expect(await store.available()).toBe(true);
  });

  it("name is 'file'", () => {
    expect(new FileKeyStore("/any/path").name).toBe("file");
  });
});

// ---------------------------------------------------------------------------
// OsKeyStore — gated behind MNEMONIC_TEST_KEYCHAIN=1 (Wave 3 Task 9)
// ---------------------------------------------------------------------------

describe.skipIf(!process.env["MNEMONIC_TEST_KEYCHAIN"])(
  "OsKeyStore (MNEMONIC_TEST_KEYCHAIN=1 required)",
  () => {
    it("placeholder — full coverage in Wave 3 Task 9", async () => {
      const store = new OsKeyStore();
      // Smoke-test only: available() must not throw.
      const avail = await store.available();
      expect(typeof avail).toBe("boolean");
    });
  },
);

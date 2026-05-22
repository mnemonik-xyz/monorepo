/**
 * Unit tests for identity/ensure.ts — five algorithm paths + skip-list.
 *
 * All tests use MemoryKeyStore for OS and a tmp-directory FileKeyStore for
 * the file backend so they never touch the real ~/.mnemonic or the macOS
 * Keychain.  The Keypair.generate() call hits WASM — tests are therefore
 * integration-level in execution but scoped to the ensure() algorithm only.
 */

import os from "node:os";
import path from "node:path";
import fs from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { describe, it, expect } from "vitest";

import {
  ensureWithStores,
  shouldSkipEnsure,
  type KeyStores,
} from "../../src/identity/ensure.js";
import { MemoryKeyStore } from "../../src/identity/keystore-memory.js";
import { FileKeyStore } from "../../src/identity/keystore-file.js";
import type { KeystoreEntry } from "../../src/identity/keystore.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function tmpDir(): string {
  return path.join(
    os.tmpdir(),
    "mnemonik-ensure-test-" + randomBytes(4).toString("hex"),
  );
}

async function makeStores(
  dir: string,
  osStore: KeyStore | null,
): Promise<KeyStores> {
  await fs.mkdir(dir, { recursive: true });
  const identityPath = path.join(dir, "identity.json");
  const readmePath = path.join(dir, "README.txt");
  return {
    os: osStore,
    file: new FileKeyStore(identityPath),
    identityPath,
    readmePath,
  };
}

async function writeLegacyFile(
  identityPath: string,
  entry: KeystoreEntry,
): Promise<void> {
  const content = JSON.stringify({
    secret: entry.secret,
    pubkey_base58: entry.pubkey_base58,
  });
  await fs.writeFile(identityPath, content, { mode: 0o600 });
}

async function writeStubFile(
  identityPath: string,
  pubkey_base58: string,
): Promise<void> {
  const content = JSON.stringify({
    pubkey_base58,
    did_sol: `did:sol:${pubkey_base58}`,
    keychain_ref: "xyz.mnemonik.identity/default",
    created_at: new Date().toISOString(),
  });
  await fs.writeFile(identityPath, content, { mode: 0o600 });
}

// A fake KeystoreEntry for tests that don't need a real WASM-generated key.
function fakeEntry(): KeystoreEntry {
  return {
    secret: Array.from({ length: 64 }, (_, i) => i % 256),
    pubkey_base58: "FakeBase58PubkeyForTestingPurposesOnly1234567890",
  };
}

// ---------------------------------------------------------------------------
// Scenario 1 — CREATE when absent, OS available
// ---------------------------------------------------------------------------

describe("ensure creates when absent (os available)", () => {
  it("generates a new identity and stores it in OS keychain + stub file", async () => {
    const dir = tmpDir();
    const osStore = new MemoryKeyStore();
    const stores = await makeStores(dir, osStore);

    const result = await ensureWithStores(stores);

    expect(result.created).toBe(true);
    expect(result.migrated).toBe(false);
    expect(result.storage).toBe("os-keychain");
    expect(typeof result.pubkey_base58).toBe("string");
    expect(result.pubkey_base58.length).toBeGreaterThan(0);

    // OS keychain should have the entry.
    const osEntry = await osStore.get();
    expect(osEntry).not.toBeNull();
    expect(osEntry!.pubkey_base58).toBe(result.pubkey_base58);

    // Stub file should exist and have keychain_ref, not secret.
    const raw = await fs.readFile(stores.identityPath, "utf8");
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    expect("keychain_ref" in parsed).toBe(true);
    expect("secret" in parsed).toBe(false);
    expect(parsed.pubkey_base58).toBe(result.pubkey_base58);

    // README should be written.
    const readme = await fs.readFile(stores.readmePath, "utf8");
    expect(readme).toContain("Mnemonic identity");
  });
});

// ---------------------------------------------------------------------------
// Scenario 2 — STUB already correct — no side effects
// ---------------------------------------------------------------------------

describe("ensure reads existing stub (no side effects)", () => {
  it("verifies keychain entry and returns without creating or migrating", async () => {
    const dir = tmpDir();
    const osStore = new MemoryKeyStore();
    const entry = fakeEntry();
    // Pre-populate OS keychain.
    await osStore.set(entry);

    const stores = await makeStores(dir, osStore);
    // Write stub file.
    await writeStubFile(stores.identityPath, entry.pubkey_base58);

    const result = await ensureWithStores(stores);

    expect(result.created).toBe(false);
    expect(result.migrated).toBe(false);
    expect(result.storage).toBe("os-keychain");
    expect(result.pubkey_base58).toBe(entry.pubkey_base58);

    // Stub file should be unchanged (still a stub).
    const raw = await fs.readFile(stores.identityPath, "utf8");
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    expect("keychain_ref" in parsed).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Scenario 3 — LEGACY → MIGRATE to OS keychain
// ---------------------------------------------------------------------------

describe("ensure migrates legacy file to OS keychain", () => {
  it("moves secret to keychain and rewrites file as stub", async () => {
    const dir = tmpDir();
    const osStore = new MemoryKeyStore(); // starts empty
    const entry = fakeEntry();

    const stores = await makeStores(dir, osStore);
    // Write legacy file.
    await writeLegacyFile(stores.identityPath, entry);

    const result = await ensureWithStores(stores);

    expect(result.created).toBe(false);
    expect(result.migrated).toBe(true);
    expect(result.storage).toBe("os-keychain");
    expect(result.pubkey_base58).toBe(entry.pubkey_base58);

    // Keychain should now hold the entry.
    const osEntry = await osStore.get();
    expect(osEntry).not.toBeNull();

    // File should now be a stub (no secret).
    const raw = await fs.readFile(stores.identityPath, "utf8");
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    expect("keychain_ref" in parsed).toBe(true);
    expect("secret" in parsed).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Scenario 4 — OS unavailable — fall back to file
// ---------------------------------------------------------------------------

describe("ensure falls back to file when OS unavailable", () => {
  it("creates a legacy-shape file and reports storage=file", async () => {
    const dir = tmpDir();
    const osStore = new MemoryKeyStore();
    osStore.setAvailable(false);

    const stores = await makeStores(dir, osStore);

    const result = await ensureWithStores(stores);

    expect(result.created).toBe(true);
    expect(result.storage).toBe("file");

    // OS keychain should be empty.
    const osEntry = await osStore.get();
    expect(osEntry).toBeNull();

    // The file should exist and have a `secret` key (legacy shape).
    const raw = await fs.readFile(stores.identityPath, "utf8");
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    expect("secret" in parsed).toBe(true);
    expect(Array.isArray(parsed.secret)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Scenario 5 — Rollback on partial failure (file-write fails after os.set)
// ---------------------------------------------------------------------------

describe("ensure rolls back on partial failure", () => {
  it("removes keychain entry when stub-file write fails", async () => {
    const dir = tmpDir();
    const osStore = new MemoryKeyStore();
    const stores = await makeStores(dir, osStore);

    // Inject a _writeStub override that always throws to simulate a filesystem
    // failure AFTER os.set() has already succeeded.
    stores._writeStub = async (_identityPath: string, _pubkey: string) => {
      throw new Error("simulated stub-file write failure");
    };

    // ensure() must reject because the stub-file write failed.
    await expect(ensureWithStores(stores)).rejects.toThrow(
      "simulated stub-file write failure",
    );

    // After the failure the OS keychain entry must have been rolled back.
    const osEntry = await osStore.get();
    expect(osEntry).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Skip-list tests
// ---------------------------------------------------------------------------

describe("shouldSkipEnsure", () => {
  it("returns true for --help flag", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "--help"])).toBe(true);
  });

  it("returns true for -h flag", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "-h"])).toBe(true);
  });

  it("returns true for --version flag", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "--version"])).toBe(true);
  });

  it("returns true for -V flag", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "-V"])).toBe(true);
  });

  it("returns true for identity status subcommand", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "identity", "status"])).toBe(
      true,
    );
  });

  it("returns true for init --force", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "init", "--force"])).toBe(
      true,
    );
  });

  it("returns false for a regular command", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "sign", "x"])).toBe(false);
  });

  it("returns false for identity export (not status)", () => {
    expect(shouldSkipEnsure(["node", "mnemonic", "identity", "export"])).toBe(
      false,
    );
  });
});

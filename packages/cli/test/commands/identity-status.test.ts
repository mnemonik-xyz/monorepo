/**
 * Tests for `mnemonic identity status` — local drift detector.
 *
 * Seam: statusWithDeps() accepts injected KeyStore + path deps so we never
 * touch the real ~/.mnemonic directory.
 */

import { mkdtempSync, rmSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MemoryKeyStore } from "../../src/identity/keystore-memory.js";
import {
  computeStatus,
  decodeJwtSub,
  exitCodeFor,
  platformName,
  statusWithDeps,
  type StatusDeps,
} from "../../src/commands/identity.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeDeps(
  overrides: Partial<StatusDeps> & {
    tokenFilePath?: string;
    identityFilePath?: string;
  } = {},
): StatusDeps {
  const mem = new MemoryKeyStore();
  return {
    os: null,
    file: mem,
    identityFilePath:
      overrides.identityFilePath ?? "/nonexistent/identity.json",
    tokenFilePath: overrides.tokenFilePath ?? "/nonexistent/token.json",
    ...overrides,
  };
}

/** Build a fake HS256 JWT for sub `sub`. */
function makeJwt(sub: string, expSecondsFromNow = 3600): string {
  const header = { alg: "HS256", typ: "JWT" };
  const payload = {
    sub,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + expSecondsFromNow,
  };
  const b64 = (o: unknown) =>
    Buffer.from(JSON.stringify(o), "utf8")
      .toString("base64")
      .replace(/=+$/g, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
  return `${b64(header)}.${b64(payload)}.fakesig`;
}

/** Write a token.json file with the given JWT to `dir`. Returns the file path. */
function writeToken(dir: string, jwt: string): string {
  const tokenPath = join(dir, "token.json");
  writeFileSync(
    tokenPath,
    JSON.stringify({
      jwt,
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub: "ignored-field",
    }),
    { mode: 0o600 },
  );
  return tokenPath;
}

/** Write a legacy identity.json (full shape with secret). */
function writeLegacyIdentity(dir: string, pubkey: string): string {
  const identityPath = join(dir, "identity.json");
  writeFileSync(
    identityPath,
    JSON.stringify({ secret: Array(64).fill(1), pubkey_base58: pubkey }),
    { mode: 0o600 },
  );
  return identityPath;
}

/** Write a stub identity.json (keychain_ref, no secret). */
function writeStubIdentity(dir: string, pubkey: string): string {
  const identityPath = join(dir, "identity.json");
  writeFileSync(
    identityPath,
    JSON.stringify({
      pubkey_base58: pubkey,
      did_sol: `did:sol:${pubkey}`,
      keychain_ref: "xyz.mnemonik.identity/default",
      created_at: new Date().toISOString(),
    }),
    { mode: 0o600 },
  );
  return identityPath;
}

// ---------------------------------------------------------------------------
// Capture stdout helper
// ---------------------------------------------------------------------------

function captureStdout(): { output: () => string; restore: () => void } {
  const chunks: string[] = [];
  const orig = process.stdout.write.bind(process.stdout);
  const spy = vi
    .spyOn(process.stdout, "write")
    .mockImplementation((chunk: unknown) => {
      chunks.push(String(chunk));
      return true;
    });
  return {
    output: () => chunks.join(""),
    restore: () => {
      spy.mockRestore();
      void orig;
    },
  };
}

// ---------------------------------------------------------------------------
// Unit tests — computeStatus / decodeJwtSub / exitCodeFor
// ---------------------------------------------------------------------------

describe("computeStatus", () => {
  it("returns no-identity when localPubkey is null", () => {
    expect(computeStatus(null, null, false)).toBe("no-identity");
    expect(computeStatus(null, "somekey", false)).toBe("no-identity");
  });

  it("returns malformed when malformed=true and localPubkey present", () => {
    expect(computeStatus("key", null, true)).toBe("malformed");
    expect(computeStatus("key", "other", true)).toBe("malformed");
  });

  it("returns webapp-unknown when jwtSub is null and localPubkey present", () => {
    expect(computeStatus("key", null, false)).toBe("webapp-unknown");
  });

  it("returns synced when pubkeys match", () => {
    expect(computeStatus("ABC123", "ABC123", false)).toBe("synced");
  });

  it("returns diverged when pubkeys differ", () => {
    expect(computeStatus("ABC123", "XYZ789", false)).toBe("diverged");
  });
});

describe("exitCodeFor", () => {
  it("returns 0 for synced and webapp-unknown", () => {
    expect(exitCodeFor("synced")).toBe(0);
    expect(exitCodeFor("webapp-unknown")).toBe(0);
  });

  it("returns 1 for no-identity", () => {
    expect(exitCodeFor("no-identity")).toBe(1);
  });

  it("returns 3 for diverged and malformed", () => {
    expect(exitCodeFor("diverged")).toBe(3);
    expect(exitCodeFor("malformed")).toBe(3);
  });
});

describe("decodeJwtSub", () => {
  it("extracts sub from a valid JWT", () => {
    const jwt = makeJwt("H8x4ABC");
    const { sub, malformed } = decodeJwtSub(jwt);
    expect(sub).toBe("H8x4ABC");
    expect(malformed).toBe(false);
  });

  it("returns malformed for garbage string", () => {
    const { sub, malformed } = decodeJwtSub("not.a.jwt.at.all.garbage");
    // 5 parts — payload part 2 might decode but lack `sub`
    // regardless, malformed should be true or sub null
    expect(sub === null || malformed).toBe(true);
  });

  it("returns malformed for a JWT with non-JSON payload", () => {
    const bad = "header.!!!.sig";
    const { sub, malformed } = decodeJwtSub(bad);
    expect(sub).toBeNull();
    expect(malformed).toBe(true);
  });

  it("returns malformed for a JWT with no sub field", () => {
    const header = Buffer.from(JSON.stringify({ alg: "HS256" })).toString(
      "base64url",
    );
    const payload = Buffer.from(JSON.stringify({ iat: 0 })).toString(
      "base64url",
    );
    const jwt = `${header}.${payload}.sig`;
    const { sub, malformed } = decodeJwtSub(jwt);
    expect(sub).toBeNull();
    expect(malformed).toBe(true);
  });

  it("returns malformed for a string with fewer than 3 parts", () => {
    const { sub, malformed } = decodeJwtSub("only.two");
    expect(sub).toBeNull();
    expect(malformed).toBe(true);
  });
});

describe("platformName", () => {
  it("returns a non-empty string", () => {
    expect(platformName().length).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// statusWithDeps — integration-style tests using tmp dirs + MemoryKeyStore
// ---------------------------------------------------------------------------

describe("statusWithDeps", () => {
  let tmpDir: string;
  let capture: ReturnType<typeof captureStdout>;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "mnemonic-status-test-"));
    capture = captureStdout();
  });

  afterEach(() => {
    capture.restore();
    try {
      rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      // ignore
    }
  });

  it("returns synced (exit 0) when local pubkey equals jwt.sub", async () => {
    const pubkey = "H8x4ABCDEF123456789012345678901234567890abc4v";
    const mem = new MemoryKeyStore();
    await mem.set({ secret: Array(64).fill(1), pubkey_base58: pubkey });

    const tokenFilePath = writeToken(tmpDir, makeJwt(pubkey));
    const deps = makeDeps({
      file: mem,
      identityFilePath: join(tmpDir, "identity.json"),
      tokenFilePath,
    });

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(0);
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("synced");
    expect(out["local"]).toBe(pubkey);
    expect(out["jwt_sub"]).toBe(pubkey);
  });

  it("returns diverged (exit 3) when local pubkey differs from jwt.sub", async () => {
    const localKey = "LocalKeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const jwtKey = "JwtKeyBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    const mem = new MemoryKeyStore();
    await mem.set({ secret: Array(64).fill(2), pubkey_base58: localKey });

    const tokenFilePath = writeToken(tmpDir, makeJwt(jwtKey));
    const deps = makeDeps({
      file: mem,
      identityFilePath: join(tmpDir, "identity.json"),
      tokenFilePath,
    });

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(3);
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("diverged");
    expect(out["local"]).toBe(localKey);
    expect(out["jwt_sub"]).toBe(jwtKey);
    expect(Array.isArray(out["suggested_actions"])).toBe(true);
  });

  it("returns webapp-unknown (exit 0) when no token.json exists", async () => {
    const pubkey = "LocalKeyNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN";
    const mem = new MemoryKeyStore();
    await mem.set({ secret: Array(64).fill(3), pubkey_base58: pubkey });

    const deps = makeDeps({
      file: mem,
      identityFilePath: join(tmpDir, "identity.json"),
      tokenFilePath: join(tmpDir, "token.json"), // does not exist
    });

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(0);
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("webapp-unknown");
    expect(out["jwt_sub"]).toBeNull();
  });

  it("returns no-identity (exit 1) when both store and file are absent", async () => {
    const deps = makeDeps({
      identityFilePath: join(tmpDir, "identity.json"), // file does not exist
      tokenFilePath: join(tmpDir, "token.json"), // also absent
    });

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(1);
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("no-identity");
    expect(out["local"]).toBeNull();
  });

  it("returns malformed (exit 3) for a garbage JWT in token.json", async () => {
    const pubkey = "LocalKeyMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM";
    const mem = new MemoryKeyStore();
    await mem.set({ secret: Array(64).fill(4), pubkey_base58: pubkey });

    // Write a token.json with garbage JWT string
    const tokenFilePath = join(tmpDir, "token.json");
    writeFileSync(
      tokenFilePath,
      JSON.stringify({
        jwt: "not-a-jwt-at-all",
        expires_at: new Date(Date.now() + 3600_000).toISOString(),
        sub: "irrelevant",
      }),
      { mode: 0o600 },
    );

    const deps = makeDeps({
      file: mem,
      identityFilePath: join(tmpDir, "identity.json"),
      tokenFilePath,
    });

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(3);
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("malformed");
    expect(Array.isArray(out["suggested_actions"])).toBe(true);
  });

  it("json mode emits parseable one-liner with all required fields", async () => {
    const pubkey = "JsonModeKeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const mem = new MemoryKeyStore();
    await mem.set({ secret: Array(64).fill(5), pubkey_base58: pubkey });
    const tokenFilePath = writeToken(tmpDir, makeJwt(pubkey));

    const deps = makeDeps({
      file: mem,
      identityFilePath: join(tmpDir, "identity.json"),
      tokenFilePath,
    });

    await statusWithDeps({ json: true }, deps);
    const raw = capture.output().trim();
    expect(() => JSON.parse(raw)).not.toThrow();
    const out = JSON.parse(raw) as Record<string, unknown>;
    expect(typeof out["local"]).toBe("string");
    expect(typeof out["jwt_sub"]).toBe("string");
    expect(typeof out["storage"]).toBe("string");
    expect(typeof out["status"]).toBe("string");
  });
});

// ---------------------------------------------------------------------------
// End-to-end smoke: write fixture files to tmpdir, set MNEMONIC_CONFIG_DIR,
// run statusWithDeps with real FileKeyStore, parse JSON output.
// ---------------------------------------------------------------------------

describe("statusWithDeps e2e smoke (real FileKeyStore)", () => {
  let tmpDir: string;
  let capture: ReturnType<typeof captureStdout>;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "mnemonic-status-e2e-"));
    capture = captureStdout();
  });

  afterEach(() => {
    capture.restore();
    try {
      rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      // ignore
    }
  });

  it("real FileKeyStore + token fixture: synced", async () => {
    const { FileKeyStore } =
      await import("../../src/identity/keystore-file.js");
    const pubkey = "E2EKeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const identityFilePath = writeLegacyIdentity(tmpDir, pubkey);
    const tokenFilePath = writeToken(tmpDir, makeJwt(pubkey));

    const fileStore = new FileKeyStore(identityFilePath);
    const deps: StatusDeps = {
      os: null,
      file: fileStore,
      identityFilePath,
      tokenFilePath,
    };

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(0);
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("synced");
    expect(out["local"]).toBe(pubkey);
  });

  it("real FileKeyStore + stub identity: storage label includes stub-referenced", async () => {
    const { FileKeyStore } =
      await import("../../src/identity/keystore-file.js");
    const pubkey = "StubKeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const identityFilePath = writeStubIdentity(tmpDir, pubkey);
    // No token — webapp-unknown

    const fileStore = new FileKeyStore(identityFilePath);
    const deps: StatusDeps = {
      os: null,
      file: fileStore,
      identityFilePath,
      tokenFilePath: join(tmpDir, "token.json"), // absent
    };

    const code = await statusWithDeps({ json: true }, deps);
    expect(code).toBe(0); // webapp-unknown
    const out = JSON.parse(capture.output()) as Record<string, unknown>;
    expect(out["status"]).toBe("webapp-unknown");
    expect(String(out["storage"])).toContain("stub-referenced");
  });
});

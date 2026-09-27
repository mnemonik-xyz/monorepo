// Key-access rule (CLI 0.3.0) + silent session renewal (issue #33).
//
// The OS keychain is mocked: every `OsKeyStore.get()` is counted. recall,
// verify, whoami and a local `sign` MUST work from a keychain-backed stub
// identity (pubkey only) with ZERO keychain reads. Only `sign --anchor`
// may read it.

import { writeFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface KeychainMock {
  gets: number;
  entry: null | { secret: number[]; pubkey_base58: string };
}

// Shared through globalThis (not `vi.hoisted`, which `bun test` — the CI
// runner — does not support). The factory may run before this module's
// top-level code, so both sides create the object on first use.
vi.mock("../src/identity/keystore-os.js", () => {
  const g = globalThis as { __mnemonicKeychainMock?: KeychainMock };
  const state = (g.__mnemonicKeychainMock ??= { gets: 0, entry: null });
  return {
    OsKeyStore: class {
      readonly name = "os";
      async available(): Promise<boolean> {
        return true;
      }
      async get(): Promise<KeychainMock["entry"]> {
        state.gets++;
        return state.entry;
      }
      async set(): Promise<void> {
        throw new Error("unexpected keychain write");
      }
      async remove(): Promise<void> {}
    },
  };
});

const ks: KeychainMock = ((
  globalThis as { __mnemonicKeychainMock?: KeychainMock }
).__mnemonicKeychainMock ??= { gets: 0, entry: null });

import { runRecall } from "../src/commands/recall.js";
import { runSign } from "../src/commands/sign.js";
import { runVerify } from "../src/commands/verify.js";
import { runWhoami } from "../src/commands/whoami.js";
import { identityPath, readTokenFile, saveToken } from "../src/config.js";
import { UserError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

interface Call {
  url: string;
  method: string;
  body: Record<string, unknown> | null;
  auth: string | undefined;
}

const BASE = "http://srv";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function rpc(result: unknown): Response {
  return json({
    jsonrpc: "2.0",
    id: 1,
    result: { content: [{ type: "text", text: JSON.stringify(result) }] },
  });
}

/** Install a routing fetch mock; `route` returns the response per call. */
function installFetch(route: (c: Call) => Response | Promise<Response>): Call[] {
  const calls: Call[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const headers = (init?.headers ?? {}) as Record<string, string>;
    let body: Record<string, unknown> | null = null;
    if (typeof init?.body === "string") {
      try {
        body = JSON.parse(init.body) as Record<string, unknown>;
      } catch {
        body = null;
      }
    }
    const c: Call = {
      url: String(input),
      method: init?.method ?? "GET",
      body,
      auth: headers.Authorization,
    };
    calls.push(c);
    return route(c);
  }) as typeof fetch;
  return calls;
}

function toolName(c: Call): string | undefined {
  const params = c.body?.params as { name?: string } | undefined;
  return params?.name;
}

function toolArgs(c: Call): Record<string, unknown> {
  const params = c.body?.params as { arguments?: Record<string, unknown> };
  return params?.arguments ?? {};
}

let cleanup = (): void => {};
let realFetch: typeof globalThis.fetch;
let mock: ReturnType<typeof installWasmMock>;
let pubkey: string;

/** Keychain-backed stub identity.json (no secret in the file). */
function writeStubIdentity(): void {
  const kp = mock.generate_keypair();
  pubkey = kp.pubkey_base58;
  ks.entry = kp; // what the (mocked) OS keychain would return
  writeFileSync(
    identityPath(),
    JSON.stringify({
      pubkey_base58: pubkey,
      did_sol: `did:sol:${pubkey}`,
      keychain_ref: "xyz.mnemonik.identity/default",
    }),
  );
}

function saveValidToken(extra: { refresh_token?: string } = {}): string {
  const jwt = makeJwt(pubkey);
  saveToken({
    jwt,
    expires_at: new Date(Date.now() + 3600_000).toISOString(),
    sub: pubkey,
    ...extra,
  });
  return jwt;
}

function saveExpiredToken(extra: { refresh_token?: string } = {}): void {
  saveToken({
    jwt: makeJwt(pubkey, -60),
    expires_at: new Date(Date.now() - 60_000).toISOString(),
    sub: pubkey,
    ...extra,
  });
}

beforeEach(() => {
  cleanup = withTmpConfigDir().cleanup;
  mock = installWasmMock();
  realFetch = globalThis.fetch;
  ks.gets = 0;
  ks.entry = null;
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  writeStubIdentity();
});

afterEach(() => {
  vi.restoreAllMocks();
  clearWasmMock();
  globalThis.fetch = realFetch;
  cleanup();
});

describe("pubkey-only commands never read the OS keychain", () => {
  it("recall works from a keychain-backed stub identity", async () => {
    const jwt = saveValidToken();
    const calls = installFetch(() => rpc({ hits: [], total: 0 }));
    await runRecall("q", { baseUrl: BASE });
    expect(calls).toHaveLength(1);
    expect(calls[0]!.auth).toBe(`Bearer ${jwt}`);
    expect(ks.gets).toBe(0);
  });

  it("verify works from a keychain-backed stub identity", async () => {
    saveValidToken();
    installFetch(() => rpc({ status: "verified", signer: pubkey }));
    await runVerify("att-1", { baseUrl: BASE });
    expect(ks.gets).toBe(0);
  });

  it("whoami --with-count works from a keychain-backed stub identity", async () => {
    saveValidToken();
    installFetch(() => rpc({ hits: [], total: 7 }));
    await runWhoami({ baseUrl: BASE, withCount: true, json: true });
    expect(ks.gets).toBe(0);
  });
});

describe("mnemonic sign write modes", () => {
  const storedRow = {
    attestation_id: "att-local",
    content_hash: "h1",
    write_mode: "local",
  };

  it("defaults to mode=local and needs no private key", async () => {
    saveValidToken();
    const calls = installFetch(() => rpc(storedRow));
    await runSign("note", { content: "note", baseUrl: BASE, json: true });
    expect(calls).toHaveLength(1);
    expect(toolName(calls[0]!)).toBe("mnemonic_sign_memory");
    expect(toolArgs(calls[0]!).mode).toBe("local");
    expect(ks.gets).toBe(0);
  });

  it("refuses the keychain when an older server wants a signature for a local write", async () => {
    saveValidToken();
    const calls = installFetch(() =>
      rpc({ status: "awaiting_signature", correlation_id: "c1" }),
    );
    const err = await runSign("note", { content: "note", baseUrl: BASE }).catch(
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(UserError);
    expect((err as Error).message).toMatch(/--anchor/);
    expect(ks.gets).toBe(0);
    expect(calls).toHaveLength(1); // pending bundle never fetched
  });

  it("--anchor sends mode=anchored and reads the key to sign", async () => {
    saveValidToken();
    const calls = installFetch((c) => {
      if (c.url.endsWith("/mcp")) {
        return rpc({ status: "awaiting_signature", correlation_id: "c1" });
      }
      if (c.url.includes("/api/pending/")) {
        return new Response(new Uint8Array([0xa0]), { status: 200 });
      }
      return json({
        attestation_id: "att-p",
        status: "anchored",
        write_mode: "anchored",
      });
    });
    await runSign("claim", {
      content: "claim",
      baseUrl: BASE,
      anchor: true,
      json: true,
    });
    // `anchored` is the canonical wire token since 2026-09-27. A server built
    // before that rejects it, so the hosted server must be deployed before this
    // CLI version is published.
    expect(toolArgs(calls[0]!).mode).toBe("anchored");
    expect(calls.map((c) => c.url)).toEqual([
      `${BASE}/mcp`,
      `${BASE}/api/pending/c1`,
      `${BASE}/api/sign-callback`,
    ]);
    expect(calls[2]!.body?.signer_pubkey).toBe(pubkey);
    expect(ks.gets).toBe(1);
  });
});

// ── silent session renewal (issue #33) ─────────────────────────────────────

/** Server side of the browserless identity login + token endpoint. */
function oauthRoutes(c: Call, refreshOutcome: () => Response): Response | null {
  if (c.url.startsWith(`${BASE}/oauth/authorize`) && c.method === "GET") {
    return json({
      challenge_cbor: btoa("\u0001\u0002"),
      state: new URL(c.url).searchParams.get("state"),
    });
  }
  if (c.url === `${BASE}/oauth/authorize`) {
    return json({ code: "code-1", state: c.body?.state });
  }
  if (c.url === `${BASE}/oauth/token`) {
    if (c.body?.grant_type === "refresh_token") return refreshOutcome();
    return json({
      access_token: makeJwt(pubkey, 3400),
      expires_in: 3600,
      refresh_token: "rt-from-login",
    });
  }
  return null;
}

// Distinct `exp` values keep test JWTs distinct (the real server adds a
// random `jti`, so a renewed JWT never equals the old one).
const rotated = (): Response =>
  json({
    access_token: makeJwt(pubkey, 3500),
    expires_in: 3600,
    refresh_token: "rt-2",
  });

describe("silent session renewal", () => {
  it("renews an expired token with the refresh token (no keychain)", async () => {
    saveExpiredToken({ refresh_token: "rt-1" });
    const calls = installFetch(
      (c) => oauthRoutes(c, rotated) ?? rpc({ hits: [], total: 0 }),
    );
    await runRecall("q", { baseUrl: BASE });
    expect(calls[0]!.body).toMatchObject({
      grant_type: "refresh_token",
      refresh_token: "rt-1",
    });
    const saved = readTokenFile();
    expect(saved.refresh_token).toBe("rt-2");
    expect(calls[1]!.auth).toBe(`Bearer ${saved.jwt}`);
    expect(ks.gets).toBe(0);
  });

  it("prints one hint (no fetch, no keychain) when renewal needs the keychain", async () => {
    saveExpiredToken(); // no refresh token; identity is keychain-backed
    const calls = installFetch(() => rpc({ hits: [], total: 0 }));
    const err = await runRecall("q", { baseUrl: BASE }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(UserError);
    expect((err as Error).message).toMatch(/token expired .*mnemonic login/);
    expect(calls).toHaveLength(0);
    expect(ks.gets).toBe(0);
  });

  it("names a rejected refresh token in the hint", async () => {
    saveExpiredToken({ refresh_token: "rt-dead" });
    const calls = installFetch(
      (c) =>
        oauthRoutes(c, () => json({ error: "invalid_grant" }, 400)) ??
        rpc({ hits: [], total: 0 }),
    );
    const err = await runRecall("q", { baseUrl: BASE }).catch((e: unknown) => e);
    expect((err as Error).message).toMatch(/refresh token was rejected/);
    expect(calls.some((c) => c.url.endsWith("/mcp"))).toBe(false);
    expect(ks.gets).toBe(0);
  });

  it("uses a token that another process renewed meanwhile", async () => {
    saveExpiredToken({ refresh_token: "rt-raced" });
    const otherJwt = makeJwt(pubkey, 1800);
    const calls = installFetch(
      (c) =>
        oauthRoutes(c, () => {
          // Another CLI process rotated first and saved a fresh token.
          saveToken({
            jwt: otherJwt,
            expires_at: new Date(Date.now() + 1800_000).toISOString(),
            sub: pubkey,
            refresh_token: "rt-other",
          });
          return json({ error: "invalid_grant" }, 400);
        }) ?? rpc({ hits: [], total: 0 }),
    );
    await runRecall("q", { baseUrl: BASE });
    expect(calls[1]!.auth).toBe(`Bearer ${otherJwt}`);
  });

  it("sign --anchor re-logs in with the identity key it reads anyway", async () => {
    saveExpiredToken(); // no refresh token
    const calls = installFetch((c) => {
      const o = oauthRoutes(c, rotated);
      if (o) return o;
      if (c.url.endsWith("/mcp")) {
        return rpc({ status: "awaiting_signature", correlation_id: "c1" });
      }
      if (c.url.includes("/api/pending/")) {
        return new Response(new Uint8Array([0xa0]), { status: 200 });
      }
      return json({ attestation_id: "att-p", status: "anchored" });
    });
    await runSign("claim", { content: "claim", baseUrl: BASE, anchor: true });
    expect(calls.map((c) => c.url.split("?")[0])).toEqual([
      `${BASE}/oauth/authorize`,
      `${BASE}/oauth/authorize`,
      `${BASE}/oauth/token`,
      `${BASE}/mcp`,
      `${BASE}/api/pending/c1`,
      `${BASE}/api/sign-callback`,
    ]);
    expect(readTokenFile().refresh_token).toBe("rt-from-login");
    expect(ks.gets).toBe(1); // one read, shared by login + signature
  });

  it("re-logs in silently when the key is file-backed (no prompt possible)", async () => {
    const kp = mock.generate_keypair();
    pubkey = kp.pubkey_base58;
    writeFileSync(identityPath(), JSON.stringify(kp)); // secret in the file
    saveExpiredToken();
    const calls = installFetch(
      (c) => oauthRoutes(c, rotated) ?? rpc({ hits: [], total: 0 }),
    );
    await runRecall("q", { baseUrl: BASE });
    expect(calls.some((c) => c.url.endsWith("/mcp"))).toBe(true);
    expect(readTokenFile().refresh_token).toBe("rt-from-login");
    expect(ks.gets).toBe(0);
  });

  it("renews and retries once when the server rejects a fresh-looking token", async () => {
    saveValidToken({ refresh_token: "rt-1" });
    let mcpCalls = 0;
    const calls = installFetch((c) => {
      const o = oauthRoutes(c, rotated);
      if (o) return o;
      mcpCalls++;
      return mcpCalls === 1
        ? json({ error: "invalid JWT" }, 401)
        : rpc({ status: "verified", signer: pubkey });
    });
    await runVerify("att-1", { baseUrl: BASE });
    expect(calls.map((c) => c.url)).toEqual([
      `${BASE}/mcp`,
      `${BASE}/oauth/token`,
      `${BASE}/mcp`,
    ]);
    expect(readTokenFile().refresh_token).toBe("rt-2");
    expect(ks.gets).toBe(0);
  });
});

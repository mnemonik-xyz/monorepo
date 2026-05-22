/**
 * Unit tests for `pullFromWebappWithDeps` and `pushToWebappWithDeps`.
 *
 * All network calls are mocked via injected `fetch`; no real HTTP or
 * filesystem I/O occurs (except writeStub which is also injected).
 * Crypto round-trips use real tweetnacl so the encryption / decryption
 * logic is exercised end-to-end in-process.
 */

import { describe, expect, it, vi, beforeEach } from "vitest";
import nacl from "tweetnacl";
import bs58 from "bs58";

import { MemoryKeyStore } from "../../src/identity/keystore-memory.js";
import {
  pullFromWebappWithDeps,
  pushToWebappWithDeps,
  type PullDeps,
  type PushDeps,
} from "../../src/commands/identity.js";

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/**
 * Build a realistic CLI-origin server response for `pullFromWebappWithDeps`.
 *
 * Simulates what the Task-12 server does on /api/cli-bootstrap/redeem:
 *   1. It knows the 64-byte plaintext secret.
 *   2. It generates a fresh ephemeral x25519 keypair (server-side).
 *   3. It wraps plaintext to the redeemer's eph pub using nacl.box.
 *   4. It returns {wrapped_secret, eph_pub, issuer_pubkey_base58}.
 *
 * @param secret64       The 64-byte secret the "server" will re-wrap.
 * @param redeemerEphPub The redeemer (CLI) ephemeral public key (32 bytes).
 */
function buildServerRedeemResponse(
  secret64: Uint8Array,
  redeemerEphPub: Uint8Array,
): { wrapped_secret: string; eph_pub: string; issuer_pubkey_base58: string } {
  // Server generates a fresh ephemeral keypair.
  const serverEph = nacl.box.keyPair();
  const nonce = nacl.randomBytes(nacl.box.nonceLength);
  const ciphertext = nacl.box(
    secret64,
    nonce,
    redeemerEphPub,
    serverEph.secretKey,
  );

  const transport = Buffer.concat([
    Buffer.from(nonce),
    Buffer.from(ciphertext),
  ]);

  return {
    wrapped_secret: transport.toString("base64"),
    eph_pub: Buffer.from(serverEph.publicKey).toString("base64"),
    issuer_pubkey_base58: bs58.encode(secret64.slice(32)),
  };
}

/** Capture everything written to process.stderr during an async fn. */
async function captureStderr<T>(
  fn: () => Promise<T>,
): Promise<{ result: T; output: string }> {
  const chunks: string[] = [];
  const orig = process.stderr.write.bind(process.stderr);
  const spy = vi
    .spyOn(process.stderr, "write")
    .mockImplementation((chunk: unknown) => {
      chunks.push(String(chunk));
      return true;
    });
  try {
    const result = await fn();
    return { result, output: chunks.join("") };
  } finally {
    spy.mockRestore();
    void orig;
  }
}

/** Build a minimal PullDeps with overrides. */
function makePullDeps(overrides: Partial<PullDeps> = {}): PullDeps & {
  fileStore: MemoryKeyStore;
  writtenStubs: Array<{ path: string; pubkey: string }>;
} {
  const fileStore = new MemoryKeyStore();
  const writtenStubs: Array<{ path: string; pubkey: string }> = [];
  return {
    os: null,
    file: fileStore,
    identityFilePath: "/tmp/fake/identity.json",
    fetch: globalThis.fetch,
    serverUrl: "https://example.com",
    writeStub: async (p, pk) => {
      writtenStubs.push({ path: p, pubkey: pk });
    },
    stdoutWrite: () => {},
    fileStore,
    writtenStubs,
    ...overrides,
  };
}

/** Build a minimal PushDeps with overrides. */
function makePushDeps(overrides: Partial<PushDeps> = {}): PushDeps & {
  fileStore: MemoryKeyStore;
  stdoutChunks: string[];
} {
  const fileStore = new MemoryKeyStore();
  const stdoutChunks: string[] = [];
  return {
    os: null,
    file: fileStore,
    fetch: globalThis.fetch,
    serverUrl: "https://example.com",
    stdoutWrite: (s) => stdoutChunks.push(s),
    skipQr: true, // default: skip QR in tests (stdout is not a TTY)
    fileStore,
    stdoutChunks,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// pullFromWebappWithDeps
// ---------------------------------------------------------------------------

describe("pullFromWebappWithDeps — happy path", () => {
  it("decrypts the server response and stores the entry in the keystore", async () => {
    // A fixed 64-byte secret: bytes 32..63 are the "public key".
    const secret64 = new Uint8Array(64);
    for (let i = 0; i < 64; i++) secret64[i] = i + 1;
    const expectedPubkey = bs58.encode(secret64.slice(32));

    // The mock fetch intercepts the POST and returns a server-simulated response.
    // It captures the redeemer_eph_pub from the request body to build the box.
    const mockFetch = vi.fn(async (_url: string, init?: RequestInit) => {
      const body = JSON.parse((init?.body as string) ?? "{}") as {
        short_code: string;
        redeemer_eph_pub: string;
      };
      const redeemerEphPub = Uint8Array.from(
        Buffer.from(body.redeemer_eph_pub, "base64"),
      );
      const resp = buildServerRedeemResponse(secret64, redeemerEphPub);
      return new Response(JSON.stringify(resp), { status: 200 });
    });

    const deps = makePullDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });

    const { result } = await captureStderr(() =>
      pullFromWebappWithDeps("ABCD-1234", deps),
    );

    expect(result).toBe(0);

    // Keystore should now have the entry.
    const stored = await deps.fileStore.get();
    expect(stored).not.toBeNull();
    expect(stored!.pubkey_base58).toBe(expectedPubkey);
    expect(stored!.secret).toEqual(Array.from(secret64));

    // Stub should have been written.
    expect(deps.writtenStubs).toHaveLength(0); // os is null, so file.set is used, no stub
  });
});

describe("pullFromWebappWithDeps — 410 expired ticket", () => {
  it("returns exit code 4 and leaves keystore untouched", async () => {
    const mockFetch = vi.fn(async () => new Response("gone", { status: 410 }));
    const deps = makePullDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });

    const { result } = await captureStderr(() =>
      pullFromWebappWithDeps("XXXX-YYYY", deps),
    );

    expect(result).toBe(4);
    const stored = await deps.fileStore.get();
    expect(stored).toBeNull();
  });
});

describe("pullFromWebappWithDeps — 404 not found", () => {
  it("returns exit code 4", async () => {
    const mockFetch = vi.fn(
      async () => new Response("not found", { status: 404 }),
    );
    const deps = makePullDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });

    const { result } = await captureStderr(() =>
      pullFromWebappWithDeps("ZZZZ-0000", deps),
    );

    expect(result).toBe(4);
  });
});

describe("pullFromWebappWithDeps — malformed wrapped_secret", () => {
  it("returns exit code 4 when wrapped_secret is not valid base64", async () => {
    const mockFetch = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            wrapped_secret: "!!!not-valid-base64!!!",
            eph_pub: Buffer.from(new Uint8Array(32)).toString("base64"),
            issuer_pubkey_base58: "fakekey",
          }),
          { status: 200 },
        ),
    );
    const deps = makePullDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });

    const { result } = await captureStderr(() =>
      pullFromWebappWithDeps("ABCD-1234", deps),
    );

    expect(result).toBe(4);
  });

  it("returns exit code 4 when decryption fails (wrong keys)", async () => {
    // Encrypt to a DIFFERENT redeemer key — so nacl.box.open returns null.
    const secret64 = new Uint8Array(64).fill(42);
    const wrongRedeemer = nacl.box.keyPair(); // not the one the CLI generated
    const resp = buildServerRedeemResponse(secret64, wrongRedeemer.publicKey);

    const mockFetch = vi.fn(
      async () => new Response(JSON.stringify(resp), { status: 200 }),
    );
    const deps = makePullDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });

    const { result } = await captureStderr(() =>
      pullFromWebappWithDeps("ABCD-1234", deps),
    );

    expect(result).toBe(4);
  });
});

describe("pullFromWebappWithDeps — pubkey mismatch", () => {
  it("returns exit code 3 when issuer_pubkey_base58 does not match derived pubkey", async () => {
    const secret64 = new Uint8Array(64);
    for (let i = 0; i < 64; i++) secret64[i] = i + 1;

    const mockFetch = vi.fn(async (_url: string, init?: RequestInit) => {
      const body = JSON.parse((init?.body as string) ?? "{}") as {
        redeemer_eph_pub: string;
      };
      const redeemerEphPub = Uint8Array.from(
        Buffer.from(body.redeemer_eph_pub, "base64"),
      );
      const resp = buildServerRedeemResponse(secret64, redeemerEphPub);
      // Tamper with the pubkey after building the response.
      return new Response(
        JSON.stringify({
          ...resp,
          issuer_pubkey_base58: "TamperedPubkey111111111111111111111111111111",
        }),
        { status: 200 },
      );
    });

    const deps = makePullDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });

    const { result } = await captureStderr(() =>
      pullFromWebappWithDeps("ABCD-1234", deps),
    );

    expect(result).toBe(3);
  });
});

// ---------------------------------------------------------------------------
// pushToWebappWithDeps
// ---------------------------------------------------------------------------

describe("pushToWebappWithDeps — happy path", () => {
  it("POSTs wrapped_secret+eph_pub and prints short_code and URL", async () => {
    const secret64 = new Uint8Array(64).fill(7);
    const pubkey = bs58.encode(secret64.slice(32));

    // Server static x25519 keypair (simulated).
    const serverStaticKp = nacl.box.keyPair();
    const serverStaticPubB64 = Buffer.from(serverStaticKp.publicKey).toString(
      "base64",
    );

    let capturedIssueBody: Record<string, unknown> | null = null;

    const mockFetch = vi.fn(async (url: string, init?: RequestInit) => {
      if ((url as string).includes("server-pub")) {
        return new Response(
          JSON.stringify({ server_x25519_pub: serverStaticPubB64 }),
          { status: 200 },
        );
      }
      if ((url as string).includes("issue-from-cli")) {
        capturedIssueBody = JSON.parse(
          (init?.body as string) ?? "{}",
        ) as Record<string, unknown>;
        return new Response(
          JSON.stringify({
            ticket_id: "uuid-1234",
            short_code: "ABCD-5678",
            expires_at: "2026-05-22T12:00:00Z",
          }),
          { status: 200 },
        );
      }
      return new Response("not found", { status: 404 });
    });

    const deps = makePushDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
      skipQr: true,
    });
    await deps.fileStore.set({
      secret: Array.from(secret64),
      pubkey_base58: pubkey,
    });

    const { result } = await captureStderr(() => pushToWebappWithDeps(deps));

    expect(result).toBe(0);

    // Verify the POST body has the expected fields.
    expect(capturedIssueBody).not.toBeNull();
    expect(typeof capturedIssueBody!["wrapped_secret"]).toBe("string");
    expect(typeof capturedIssueBody!["eph_pub"]).toBe("string");
    expect(capturedIssueBody!["issuer_pubkey_base58"]).toBe(pubkey);

    // Verify wrapped_secret is valid base64 nonce||ciphertext.
    const wrappedBytes = Buffer.from(
      capturedIssueBody!["wrapped_secret"] as string,
      "base64",
    );
    expect(wrappedBytes.length).toBeGreaterThan(nacl.box.nonceLength);

    // Verify stdout contains the URL and short_code.
    const stdout = deps.stdoutChunks.join("");
    expect(stdout).toContain("ABCD-5678");
    expect(stdout).toContain("mnemonik.xyz/install?pull=ABCD-5678");
    expect(stdout).toContain("2026-05-22T12:00:00Z");
  });
});

describe("pushToWebappWithDeps — server error 500", () => {
  it("returns exit code 2", async () => {
    const secret64 = new Uint8Array(64).fill(9);
    const pubkey = bs58.encode(secret64.slice(32));
    const serverStaticPubB64 = Buffer.from(
      nacl.box.keyPair().publicKey,
    ).toString("base64");

    const mockFetch = vi.fn(async (url: string) => {
      if ((url as string).includes("server-pub")) {
        return new Response(
          JSON.stringify({ server_x25519_pub: serverStaticPubB64 }),
          { status: 200 },
        );
      }
      return new Response(JSON.stringify({ error: "internal error" }), {
        status: 500,
      });
    });

    const deps = makePushDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
    });
    await deps.fileStore.set({
      secret: Array.from(secret64),
      pubkey_base58: pubkey,
    });

    const { result } = await captureStderr(() => pushToWebappWithDeps(deps));

    expect(result).toBe(2);
  });
});

describe("pushToWebappWithDeps — no local identity", () => {
  it("returns exit code 1", async () => {
    const deps = makePushDeps();
    // fileStore has no entry.

    const { result } = await captureStderr(() => pushToWebappWithDeps(deps));

    expect(result).toBe(1);
  });
});

describe("pushToWebappWithDeps — code-only mode", () => {
  it("output does not contain QR characters but does contain URL and short code", async () => {
    const secret64 = new Uint8Array(64).fill(3);
    const pubkey = bs58.encode(secret64.slice(32));
    const serverStaticPubB64 = Buffer.from(
      nacl.box.keyPair().publicKey,
    ).toString("base64");

    const mockFetch = vi.fn(async (url: string) => {
      if ((url as string).includes("server-pub")) {
        return new Response(
          JSON.stringify({ server_x25519_pub: serverStaticPubB64 }),
          { status: 200 },
        );
      }
      return new Response(
        JSON.stringify({
          ticket_id: "uuid-xyz",
          short_code: "CODE-1234",
          expires_at: "2026-05-22T13:00:00Z",
        }),
        { status: 200 },
      );
    });

    const deps = makePushDeps({
      fetch: mockFetch as unknown as typeof globalThis.fetch,
      skipQr: true,
    });
    await deps.fileStore.set({
      secret: Array.from(secret64),
      pubkey_base58: pubkey,
    });

    await captureStderr(() => pushToWebappWithDeps(deps));

    const stdout = deps.stdoutChunks.join("");
    expect(stdout).toContain("CODE-1234");
    // No UTF-8 block characters that qrcode-terminal emits.
    expect(stdout).not.toMatch(/[█▄▀]/);
  });
});

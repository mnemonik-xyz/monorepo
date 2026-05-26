/**
 * Integration test: end-to-end CLI push → webapp redeem ticket flow.
 *
 * This test exercises the full symmetric ticket flow with a mock server
 * that does a REAL crypto round-trip using tweetnacl — matching what the
 * production server does with crypto_box::SalsaBox (bit-compatible).
 *
 * Flow:
 *   1. CLI A (push) fetches server static pub, wraps its secret to that pub,
 *      POSTs to /api/cli-bootstrap/issue-from-cli → gets {ticket_id, short_code}.
 *   2. Mock "server" stores the wrapped_secret + eph_pub.
 *   3. CLI B (pull) POSTs to /api/cli-bootstrap/redeem with {short_code,
 *      redeemer_eph_pub}. The mock server unwraps with server static SK,
 *      then re-wraps to CLI B's eph_pub.
 *   4. CLI B decrypts the re-wrapped payload and stores it.
 *   5. Assert: CLI B's stored secret === CLI A's original secret.
 *
 * Crypto note: The server uses crypto_box::SalsaBox (XSalsa20-Poly1305 +
 * Curve25519) with wire format nonce[24] || ciphertext. tweetnacl's nacl.box /
 * nacl.box.open is bit-compatible with that scheme — the same as the webapp's
 * Install.tsx. No gotchas observed; the nonce and MAC layout are identical.
 *
 * Deviation note: The Task-12 server does NOT expose a POST route for
 * short_code redemption — it only has GET /api/cli-bootstrap/redeem/{ticket}
 * which requires a UUID. The webapp's Install.tsx posts {short_code} to
 * /api/cli-bootstrap/redeem (POST) which is not in the T12 router. This mock
 * uses the POST shape matching what T14 and T13 expect, pending a T12 router
 * update. Wave-5 manual smoke will confirm true server E2E.
 */

import { describe, expect, it, beforeEach } from "vitest";
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
// Mock server state
// ---------------------------------------------------------------------------

interface TicketRecord {
  short_code: string;
  wrapped_secret: Uint8Array; // nonce[24] || ciphertext, encrypted to server static SK
  eph_pub: Uint8Array; // CLI A's ephemeral pub (32 bytes)
  issuer_pubkey_base58: string;
}

/** Shared mock-server state reset between tests. */
function buildMockServer() {
  const serverStaticKp = nacl.box.keyPair();
  const tickets = new Map<string, TicketRecord>();
  let ticketCounter = 0;

  /** GET /api/cli-bootstrap/server-pub */
  function handleServerPub(): Response {
    return new Response(
      JSON.stringify({
        server_x25519_pub: Buffer.from(serverStaticKp.publicKey).toString(
          "base64",
        ),
      }),
      { status: 200 },
    );
  }

  /** POST /api/cli-bootstrap/issue-from-cli */
  function handleIssueFromCli(body: {
    wrapped_secret: string;
    eph_pub: string;
    issuer_pubkey_base58: string;
  }): Response {
    const wrappedBytes = Uint8Array.from(
      Buffer.from(body.wrapped_secret, "base64"),
    );
    const ephPubBytes = Uint8Array.from(Buffer.from(body.eph_pub, "base64"));
    const shortCode = `T${++ticketCounter}ST-CODE`;
    tickets.set(shortCode, {
      short_code: shortCode,
      wrapped_secret: wrappedBytes,
      eph_pub: ephPubBytes,
      issuer_pubkey_base58: body.issuer_pubkey_base58,
    });
    const expiresAt = new Date(Date.now() + 600_000).toISOString();
    return new Response(
      JSON.stringify({
        ticket_id: `ticket-${ticketCounter}`,
        short_code: shortCode,
        expires_at: expiresAt,
      }),
      { status: 200 },
    );
  }

  /**
   * POST /api/cli-bootstrap/redeem
   * Body: { short_code, redeemer_eph_pub }
   *
   * Mirror of the Task-12 Rust server logic:
   *   1. Lookup ticket by short_code.
   *   2. Unwrap with server static SK + issuer eph_pub.
   *   3. Re-wrap to redeemer eph_pub with a fresh server ephemeral.
   *   4. Return {wrapped_secret, eph_pub, issuer_pubkey_base58}.
   */
  function handleRedeem(body: {
    short_code: string;
    redeemer_eph_pub: string;
  }): Response {
    const ticket = tickets.get(body.short_code);
    if (!ticket) {
      return new Response(JSON.stringify({ error: "not found" }), {
        status: 404,
      });
    }

    // Unwrap: server static SK + issuer eph_pub → plaintext.
    if (ticket.wrapped_secret.length < nacl.box.nonceLength) {
      return new Response(JSON.stringify({ error: "malformed ticket" }), {
        status: 500,
      });
    }
    const nonce = ticket.wrapped_secret.slice(0, nacl.box.nonceLength);
    const ct = ticket.wrapped_secret.slice(nacl.box.nonceLength);
    const plaintext = nacl.box.open(
      ct,
      nonce,
      ticket.eph_pub,
      serverStaticKp.secretKey,
    );
    if (plaintext === null) {
      return new Response(JSON.stringify({ error: "decryption failed" }), {
        status: 500,
      });
    }

    // Re-wrap to redeemer's eph_pub with a fresh server ephemeral.
    const serverEph = nacl.box.keyPair();
    const redeemerEphPub = Uint8Array.from(
      Buffer.from(body.redeemer_eph_pub, "base64"),
    );
    const outNonce = nacl.randomBytes(nacl.box.nonceLength);
    const rewrapped = nacl.box(
      plaintext,
      outNonce,
      redeemerEphPub,
      serverEph.secretKey,
    );

    const transport = Buffer.concat([
      Buffer.from(outNonce),
      Buffer.from(rewrapped),
    ]);

    // Single-use: consume the ticket.
    tickets.delete(body.short_code);

    return new Response(
      JSON.stringify({
        wrapped_secret: transport.toString("base64"),
        eph_pub: Buffer.from(serverEph.publicKey).toString("base64"),
        issuer_pubkey_base58: ticket.issuer_pubkey_base58,
      }),
      { status: 200 },
    );
  }

  /** Unified fetch mock routing by URL + method. */
  function mockFetch(url: string, init?: RequestInit): Promise<Response> {
    const u = url.toString();
    if (u.includes("server-pub")) {
      return Promise.resolve(handleServerPub());
    }
    if (u.includes("issue-from-cli")) {
      const body = JSON.parse((init?.body as string) ?? "{}") as {
        wrapped_secret: string;
        eph_pub: string;
        issuer_pubkey_base58: string;
      };
      return Promise.resolve(handleIssueFromCli(body));
    }
    if (u.includes("/redeem")) {
      const body = JSON.parse((init?.body as string) ?? "{}") as {
        short_code: string;
        redeemer_eph_pub: string;
      };
      return Promise.resolve(handleRedeem(body));
    }
    return Promise.resolve(new Response("not found", { status: 404 }));
  }

  return { mockFetch, tickets, serverStaticKp };
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

describe("ticket-flow integration: CLI push → webapp redeem (real crypto)", () => {
  let server: ReturnType<typeof buildMockServer>;

  beforeEach(() => {
    server = buildMockServer();
  });

  it("CLI A secret == CLI B stored secret after full round-trip", async () => {
    // CLI A has a known 64-byte secret.
    const cliASecret = new Uint8Array(64);
    for (let i = 0; i < 64; i++) cliASecret[i] = (i * 3 + 7) % 256;
    const cliAPubkey = bs58.encode(cliASecret.slice(32));

    const cliAFileStore = new MemoryKeyStore();
    await cliAFileStore.set({
      secret: Array.from(cliASecret),
      pubkey_base58: cliAPubkey,
    });

    let capturedShortCode = "";
    const cliAOutput: string[] = [];

    const pushDeps: PushDeps = {
      os: null,
      file: cliAFileStore,
      fetch: server.mockFetch as unknown as typeof globalThis.fetch,
      serverUrl: "https://mock.example.com",
      stdoutWrite: (s) => {
        cliAOutput.push(s);
        // Extract the short code from "Short code: XXXX" line.
        const m = s.match(/Short code:\s+(\S+)/);
        if (m) capturedShortCode = m[1];
        // Also match the URL param.
        const m2 = s.match(/\?pull=([A-Z0-9-]+)/);
        if (m2 && !capturedShortCode) capturedShortCode = m2[1];
      },
      skipQr: true,
    };

    const pushResult = await pushToWebappWithDeps(pushDeps);
    expect(pushResult).toBe(0);

    // Resolve the short_code from the server's stored ticket (avoids fragile
    // output-parsing; the mock server map has exactly one entry).
    const ticketEntries = Array.from(server.tickets.entries());
    expect(ticketEntries).toHaveLength(1);
    const [shortCode] = ticketEntries[0];

    // CLI B redeems the short code.
    const cliBFileStore = new MemoryKeyStore();
    const cliBWrittenStubs: Array<{ path: string; pubkey: string }> = [];

    const pullDeps: PullDeps = {
      os: null,
      file: cliBFileStore,
      identityFilePath: "/tmp/cli-b/identity.json",
      fetch: server.mockFetch as unknown as typeof globalThis.fetch,
      serverUrl: "https://mock.example.com",
      writeStub: async (p, pk) => {
        cliBWrittenStubs.push({ path: p, pubkey: pk });
      },
      stdoutWrite: () => {},
    };

    const pullResult = await pullFromWebappWithDeps(shortCode, pullDeps);
    expect(pullResult).toBe(0);

    // CLI B now has the same secret as CLI A.
    const cliBEntry = await cliBFileStore.get();
    expect(cliBEntry).not.toBeNull();
    expect(cliBEntry!.pubkey_base58).toBe(cliAPubkey);
    expect(cliBEntry!.secret).toEqual(Array.from(cliASecret));

    // Ticket was consumed — a second redeem fails.
    expect(server.tickets.size).toBe(0);
  });

  it("second redeem of the same short_code fails with 404", async () => {
    const secret64 = new Uint8Array(64).fill(5);
    const pubkey = bs58.encode(secret64.slice(32));

    const fileStore = new MemoryKeyStore();
    await fileStore.set({
      secret: Array.from(secret64),
      pubkey_base58: pubkey,
    });

    const pushDeps: PushDeps = {
      os: null,
      file: fileStore,
      fetch: server.mockFetch as unknown as typeof globalThis.fetch,
      serverUrl: "https://mock.example.com",
      stdoutWrite: () => {},
      skipQr: true,
    };
    await pushToWebappWithDeps(pushDeps);

    const [shortCode] = Array.from(server.tickets.keys());

    // First redeem succeeds.
    const pullDeps: PullDeps = {
      os: null,
      file: new MemoryKeyStore(),
      identityFilePath: "/tmp/pull-1/identity.json",
      fetch: server.mockFetch as unknown as typeof globalThis.fetch,
      serverUrl: "https://mock.example.com",
      writeStub: async () => {},
      stdoutWrite: () => {},
    };
    const first = await pullFromWebappWithDeps(shortCode, pullDeps);
    expect(first).toBe(0);

    // Second redeem — ticket consumed, server returns 404.
    const pullDeps2: PullDeps = {
      ...pullDeps,
      file: new MemoryKeyStore(),
    };
    const second = await pullFromWebappWithDeps(shortCode, pullDeps2);
    expect(second).toBe(4); // not found → exit 4
  });

  it("crypto round-trip: CLI A wrapped bytes can be decrypted by server static SK", async () => {
    // Verify at the byte level that the wrapped_secret CLI A sends is
    // decryptable by the server's static secret key + CLI A's eph_pub.
    const secret64 = new Uint8Array(64);
    for (let i = 0; i < 64; i++) secret64[i] = i;
    const pubkey = bs58.encode(secret64.slice(32));

    const fileStore = new MemoryKeyStore();
    await fileStore.set({
      secret: Array.from(secret64),
      pubkey_base58: pubkey,
    });

    let capturedIssueBody: {
      wrapped_secret: string;
      eph_pub: string;
      issuer_pubkey_base58: string;
    } | null = null;

    const interceptingFetch = async (
      url: string,
      init?: RequestInit,
    ): Promise<Response> => {
      if (url.includes("issue-from-cli")) {
        capturedIssueBody = JSON.parse((init?.body as string) ?? "{}") as {
          wrapped_secret: string;
          eph_pub: string;
          issuer_pubkey_base58: string;
        };
      }
      return server.mockFetch(url, init);
    };

    const pushDeps: PushDeps = {
      os: null,
      file: fileStore,
      fetch: interceptingFetch as unknown as typeof globalThis.fetch,
      serverUrl: "https://mock.example.com",
      stdoutWrite: () => {},
      skipQr: true,
    };
    await pushToWebappWithDeps(pushDeps);

    expect(capturedIssueBody).not.toBeNull();

    // Manually decrypt using the server's static SK + CLI A's eph_pub.
    const wrappedBytes = Uint8Array.from(
      Buffer.from(capturedIssueBody!.wrapped_secret, "base64"),
    );
    const ephPubBytes = Uint8Array.from(
      Buffer.from(capturedIssueBody!.eph_pub, "base64"),
    );

    expect(wrappedBytes.length).toBeGreaterThan(nacl.box.nonceLength);

    const nonce = wrappedBytes.slice(0, nacl.box.nonceLength);
    const ct = wrappedBytes.slice(nacl.box.nonceLength);
    const plaintext = nacl.box.open(
      ct,
      nonce,
      ephPubBytes,
      server.serverStaticKp.secretKey,
    );

    expect(plaintext).not.toBeNull();
    expect(Array.from(plaintext!)).toEqual(Array.from(secret64));
    expect(bs58.encode(plaintext!.slice(32))).toBe(pubkey);
  });
});

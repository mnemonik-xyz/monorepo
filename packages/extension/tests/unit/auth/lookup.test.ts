// Lookup tests — exercise the four documented branches of
// `POST /oauth/google/lookup` (`mcp/src/oauth/google.rs`):
//
//   1. existing pubkey + escrow_present → restore-on-new-device path,
//   2. existing pubkey + no escrow      → manual-import edge case,
//   3. no existing pubkey               → first-link path (link_challenge
//                                          present),
//   4. server 500                        → surfaces as `AuthError`.
//
// Plus rate-limit (429), missing JWT, and malformed-body coverage.

// Uses vitest imports — works under vitest natively and under `bun test`
// via bun's vitest-compat shim (Decision 11 of the repo CI policy).
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { lookupExisting } from "../../../src/auth/lookup.js";
import { AuthError } from "../../../src/auth/types.js";

let originalFetch: typeof globalThis.fetch | undefined;
let fetchCalls: { url: string; init?: RequestInit }[] = [];

function installFetchMock(
  handler: (url: string, init?: RequestInit) => Response,
): void {
  fetchCalls = [];
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    // `exactOptionalPropertyTypes` rejects `{ init: undefined }`.
    const entry: { url: string; init?: RequestInit } = { url };
    if (init !== undefined) entry.init = init;
    fetchCalls.push(entry);
    return Promise.resolve(handler(url, init));
  }) as typeof globalThis.fetch;
}

beforeEach(() => {
  originalFetch = globalThis.fetch;
});

afterEach(() => {
  if (originalFetch) globalThis.fetch = originalFetch;
});

const FAKE_JWT = "header.payload.signature";

describe("lookupExisting — happy path: existing pubkey + escrow", () => {
  it("returns the pubkey + escrowPresent=true and posts to /oauth/google/lookup with the bearer JWT", async () => {
    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            existing_pubkey: "H8x_existing_pubkey_base58",
            escrow_present: true,
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    );
    const result = await lookupExisting(FAKE_JWT, {
      serverOrigin: "https://mc.test",
    });
    expect(result.existingPubkey).toBe("H8x_existing_pubkey_base58");
    expect(result.escrowPresent).toBe(true);
    // No link_challenge expected on the existing-pubkey path.
    expect(result.linkChallenge).toBeNull();

    expect(fetchCalls.length).toBe(1);
    const call = fetchCalls[0];
    expect(call?.url).toBe("https://mc.test/oauth/google/lookup");
    const headers = call?.init?.headers as Record<string, string>;
    expect(headers.authorization).toBe(`Bearer ${FAKE_JWT}`);
    expect(call?.init?.method).toBe("POST");
  });
});

describe("lookupExisting — first-link path: no existing pubkey", () => {
  it("returns existingPubkey=null, escrowPresent=false, and surfaces link_challenge", async () => {
    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            existing_pubkey: null,
            escrow_present: false,
            link_challenge: "base64-nonce==",
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    );
    const result = await lookupExisting(FAKE_JWT, {
      serverOrigin: "https://mc.test",
    });
    expect(result.existingPubkey).toBeNull();
    expect(result.escrowPresent).toBe(false);
    expect(result.linkChallenge).toBe("base64-nonce==");
  });
});

describe("lookupExisting — edge case: existing pubkey but no escrow", () => {
  it("returns existingPubkey set, escrowPresent=false, no link_challenge", async () => {
    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            existing_pubkey: "H8x_existing_pubkey_base58",
            escrow_present: false,
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    );
    const result = await lookupExisting(FAKE_JWT, {
      serverOrigin: "https://mc.test",
    });
    expect(result.existingPubkey).toBe("H8x_existing_pubkey_base58");
    expect(result.escrowPresent).toBe(false);
    expect(result.linkChallenge).toBeNull();
  });
});

describe("lookupExisting — server 500", () => {
  it("throws AuthError surfacing the status code", async () => {
    installFetchMock(
      () =>
        new Response(JSON.stringify({ error: "internal error" }), {
          status: 500,
          headers: { "content-type": "application/json" },
        }),
    );
    let caught: unknown = undefined;
    try {
      await lookupExisting(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("500");
  });
});

describe("lookupExisting — server 429 rate-limit", () => {
  it("throws AuthError flagged with the rate-limit status", async () => {
    installFetchMock(
      () =>
        new Response(JSON.stringify({ error: "rate limit" }), {
          status: 429,
          headers: { "content-type": "application/json" },
        }),
    );
    let caught: unknown = undefined;
    try {
      await lookupExisting(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message.toLowerCase()).toContain("rate-limit");
  });
});

describe("lookupExisting — server 401 unauthorized", () => {
  it("throws AuthError flagged with 401", async () => {
    installFetchMock(() => new Response("", { status: 401 }));
    let caught: unknown = undefined;
    try {
      await lookupExisting(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("401");
  });
});

describe("lookupExisting — input validation", () => {
  it("throws when jwt is empty", async () => {
    installFetchMock(() => new Response("never reached", { status: 500 }));
    let caught: unknown = undefined;
    try {
      await lookupExisting("", { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    // No request should have been issued.
    expect(fetchCalls.length).toBe(0);
  });
});

describe("lookupExisting — malformed response shape", () => {
  it("throws AuthError when escrow_present is missing", async () => {
    installFetchMock(
      () =>
        new Response(JSON.stringify({ existing_pubkey: "x" }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
    );
    let caught: unknown = undefined;
    try {
      await lookupExisting(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("escrow_present");
  });

  it("throws AuthError when response is not JSON", async () => {
    installFetchMock(() => new Response("not json at all", { status: 200 }));
    let caught: unknown = undefined;
    try {
      await lookupExisting(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });
});

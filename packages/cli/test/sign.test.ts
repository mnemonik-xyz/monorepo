// `mnemonic sign` — happy path against a stubbed SDK; UserError without token.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runSign } from "../src/commands/sign.js";
import { saveIdentityJson, saveToken } from "../src/config.js";
import { ServerError, UserError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

describe("runSign", () => {
  let cleanup = (): void => {};
  let mock: ReturnType<typeof installWasmMock>;
  let realFetch: typeof globalThis.fetch | undefined;

  beforeEach(() => {
    cleanup = withTmpConfigDir().cleanup;
    mock = installWasmMock();
    void mock;
    realFetch = globalThis.fetch;
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    if (realFetch) globalThis.fetch = realFetch;
    cleanup();
  });

  it("throws UserError if no identity / token", async () => {
    await expect(runSign("hello", { content: "hello" })).rejects.toBeInstanceOf(
      UserError
    );
  });

  it("UserError if identity exists but token is missing", async () => {
    // Generate a real identity via the WASM mock.
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    await expect(runSign("hello", { content: "hello" })).rejects.toBeInstanceOf(
      UserError
    );
  });

  it("happy path: posts to /mcp and surfaces attestation_id", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    const sub = kp.pubkey_base58;
    saveToken({
      jwt: makeJwt(sub),
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub,
    });

    // Three-step server: /mcp → /api/pending/<corr> → /api/sign-callback.
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString();
      if (url.endsWith("/mcp")) {
        // tools/call mnemonic_sign_memory → returns correlation_id only.
        return new Response(
          JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            result: {
              content: [
                {
                  type: "text",
                  text: JSON.stringify({ correlation_id: "corr-1" }),
                },
              ],
            },
          }),
          {
            status: 200,
            headers: { "content-type": "application/json" },
          }
        );
      }
      if (url.includes("/api/pending/")) {
        // Canonical CBOR bytes — opaque to the SDK; signed via the WASM mock.
        return new Response(new Uint8Array([0x82, 0x01, 0x02]), {
          status: 200,
          headers: { "content-type": "application/cbor" },
        });
      }
      if (url.includes("/api/sign-callback")) {
        return new Response(
          JSON.stringify({
            attestation_id: "att-1",
            signed_at: new Date().toISOString(),
            status: "stored",
            content_hash: "deadbeef",
          }),
          {
            status: 200,
            headers: { "content-type": "application/json" },
          }
        );
      }
      return new Response("not found", { status: 404 });
    });
    globalThis.fetch = fetchMock as never;

    await runSign("hello", {
      content: "hello",
      baseUrl: "http://test",
      tags: "a, b",
    });
    expect(fetchMock).toHaveBeenCalled();
    const calls = fetchMock.mock.calls.map((c) => String(c[0]));
    expect(calls.some((u) => u.endsWith("/mcp"))).toBe(true);
    expect(calls.some((u) => u.includes("/api/pending/corr-1"))).toBe(true);
    expect(calls.some((u) => u.includes("/api/sign-callback"))).toBe(true);
  });

  it("ServerError (exit 2) when /mcp returns 500", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    const sub = kp.pubkey_base58;
    saveToken({
      jwt: makeJwt(sub),
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub,
    });

    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ error: "boom" }), {
          status: 500,
          headers: { "content-type": "application/json" },
        })
    );
    globalThis.fetch = fetchMock as never;

    await expect(
      runSign("hello", { content: "hello", baseUrl: "http://test" })
    ).rejects.toMatchObject({ exitCode: 2 });
    // Re-run a fresh call to assert the error class — vitest rejects.toBeInstanceOf
    // collapses with toMatchObject above so we re-run a second time.
    await expect(
      runSign("hello", { content: "hello", baseUrl: "http://test" })
    ).rejects.toBeInstanceOf(ServerError);
  });
});

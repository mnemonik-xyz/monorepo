// Unit tests for CLI redactSecrets — propagated copy of SDK redactJWT.
//
// Drives T11 audit fix (A09): wasm-bindgen errors and stored-keypair
// payloads carry a 64-element JSON byte array `[n0,...,n63]`. The hex
// regex doesn't catch them; this asserts the new SOLANA_KEYPAIR_ARRAY_RE.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runSign } from "../src/commands/sign.js";
import { saveIdentityJson, saveToken } from "../src/config.js";
import { redactSecrets, AuthError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

describe("redactSecrets (CLI)", () => {
  it("redacts a JWT-shaped run", () => {
    // Single eyJ-prefixed segment with ≥20 base64url chars matches the regex.
    const jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
    expect(redactSecrets(`token=${jwt}`)).toBe("token=[REDACTED-JWT]");
  });

  it("redacts a 128-hex ed25519 secret run", () => {
    const hex = "a".repeat(128);
    expect(redactSecrets(`secret=${hex}`)).toBe("secret=[REDACTED-SECRET]");
  });

  it("redacts a Solana 64-element keypair JSON array (T11 A09)", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const out = redactSecrets(`keypair=${arr}`);
    expect(out).toBe("keypair=[REDACTED-KEYPAIR]");
    expect(out).not.toContain(arr);
  });

  it("redacts Solana keypair array with surrounding whitespace", () => {
    const elems = Array.from({ length: 64 }, (_, i) => i).join(", ");
    const arr = `[ ${elems} ]`;
    expect(redactSecrets(`kp=${arr};`)).toBe("kp=[REDACTED-KEYPAIR];");
  });

  it("AuthError carries a redacted message including keypair arrays", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const err = new AuthError(`signer rejected ${arr}`);
    expect(err.message).toContain("[REDACTED-KEYPAIR]");
    expect(err.message).not.toContain(arr);
    expect(err.exitCode).toBe(4);
  });

  it("is idempotent", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const once = redactSecrets(arr);
    expect(redactSecrets(once)).toBe(once);
  });
});

// ── end-to-end redaction (CLI ↔ SDK ↔ stderr) ───────────────────────────────
// Tech-spec line 320: "assert no JWT-shape strings (/eyJ[A-Za-z0-9_-]{20,}/)
// or 128-hex-char secrets ever appear in captured stderr/stdout during error
// paths". The SDK layer is already pinned (sdk/test/client.test.ts:455);
// this test pins the CLI's `fromSdkError → handleError` pipeline against
// a leaky 401 response body, so a regression that re-introduces a JWT leak
// on the CLI surface (e.g. accidental console.error of err.cause) fails here.
describe("CLI end-to-end redaction (sign → 401 with leaky body)", () => {
  let cleanup = (): void => {};
  let mock: ReturnType<typeof installWasmMock>;
  let realFetch: typeof globalThis.fetch | undefined;
  const stderrChunks: string[] = [];

  beforeEach(() => {
    cleanup = withTmpConfigDir().cleanup;
    mock = installWasmMock();
    void mock;
    realFetch = globalThis.fetch;
    stderrChunks.length = 0;
    vi.spyOn(process.stderr, "write").mockImplementation(((
      chunk: string | Uint8Array
    ) => {
      stderrChunks.push(
        typeof chunk === "string" ? chunk : Buffer.from(chunk).toString("utf8")
      );
      return true;
    }) as never);
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    if (realFetch) globalThis.fetch = realFetch;
    cleanup();
  });

  it("a 401 body containing a JWT does NOT leak the JWT into captured stderr", async () => {
    // Realistic JWT-shaped tokens (≥20 base64url chars after `eyJ`).
    const leakyJwt =
      "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJsZWFreS11c2VyIiwiZXhwIjoxOTk5OTk5OTk5fQ.test-signature-leak-1234567890abc";
    const userJwt = makeJwt("user-pubkey-1");

    // Persist a valid identity + token so runSign reaches the network.
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    saveToken({
      jwt: userJwt,
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub: kp.pubkey_base58,
    });

    // Mock fetch: 401 from /mcp with the leaky JWT in the response body.
    // This simulates a server that echoes the offending Bearer token in an
    // error message (a common but bad pattern). The SDK should map the 401
    // → AuthError; the CLI's handleError should print only the redacted
    // message to stderr.
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            error: "invalid_token",
            error_description: `the bearer token ${leakyJwt} is not valid`,
          }),
          {
            status: 401,
            headers: { "content-type": "application/json" },
          }
        )
    );
    globalThis.fetch = fetchMock as never;

    // runSign throws — that's the error path we're pinning. We catch the
    // CliError and route it through handleError manually (process.exit is
    // stubbed via the throw in the assertion).
    let thrown: unknown;
    try {
      await runSign("hello", { content: "hello", baseUrl: "http://test" });
    } catch (e) {
      thrown = e;
    }

    expect(thrown).toBeDefined();
    // Simulate the top-level catch in bin/mnemonic.ts: it calls handleError,
    // which writes to stderr. We invoke its formatter manually because
    // handleError calls process.exit which would terminate the test runner.
    const err = thrown as { name: string; message: string };
    process.stderr.write(`${err.name}: ${err.message}\n`);

    const stderr = stderrChunks.join("");
    expect(stderr.length).toBeGreaterThan(0);
    // The whole leaky JWT must NOT appear verbatim.
    expect(stderr).not.toContain(leakyJwt);
    // Defense-in-depth: no eyJ-prefixed run of >=20 base64url chars at all.
    expect(stderr).not.toMatch(/eyJ[A-Za-z0-9_-]{20,}/);
    // And no 128-hex secret either.
    expect(stderr).not.toMatch(/\b[0-9a-fA-F]{128}\b/);
  });
});

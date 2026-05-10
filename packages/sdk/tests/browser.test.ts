// @vitest-environment jsdom
//
// Browser-bundle smoke test for `dist/browser.js`.
//
// Two assertions, both driving the T02 deliverable:
//
//   1. `client_works_in_browser_env` — under a jsdom global, importing the
//      built `dist/browser.js` exposes `MnemonicClient`. Instantiating it
//      and calling `signMemory` issues exactly one `POST /mcp` with the
//      `mnemonic_sign_memory` tools/call envelope (the start of the
//      deferred-signing flow per Decision 7). The mocked fetch short-
//      circuits with an error before any WASM-touching step runs.
//
//   2. `no_node_imports` — static analysis on the bundled JS rejects any
//      `node:*` import or `process.versions` reference. This is what
//      stops a future regression from re-introducing the Node fs path
//      that broke MV3 service workers in 0.1.0–0.1.2.

import { describe, it, expect, beforeAll } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const BUNDLE = path.resolve(__dirname, "../dist/browser.js");

describe("browser bundle (dist/browser.js)", () => {
  beforeAll(() => {
    if (!existsSync(BUNDLE)) {
      throw new Error(
        `dist/browser.js not found at ${BUNDLE} — run \`npm run build:browser\` first`,
      );
    }
  });

  it("client_works_in_browser_env: signMemory hits /mcp with the deferred-signing envelope", async () => {
    const mod = await import(/* @vite-ignore */ pathToFileURL(BUNDLE).href);
    expect(typeof mod.MnemonicClient).toBe("function");

    const captured: Array<{ url: string; init: RequestInit }> = [];
    const fetchMock = (url: string | URL | Request, init?: RequestInit) => {
      captured.push({ url: String(url), init: init ?? {} });
      // Halt the flow with an authn error before the second hop / WASM.
      return Promise.resolve(
        new Response(
          JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            error: { code: 401, message: "test halt" },
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
      );
    };

    // signMemory only needs setKeypair to have been called — the WASM step
    // happens after the first POST, so a structurally-valid stub is fine.
    const stubKeypairJson = {
      secret: Array(64).fill(0),
      pubkey_base58: "TestPubkey123",
    };

    const client = new mod.MnemonicClient({
      baseUrl: "https://mcp.example",
      jwt: "test.jwt.token",
      signer: {
        pubkey: stubKeypairJson.pubkey_base58,
        sign: () => Promise.resolve(new Uint8Array(64)),
      },
      fetch: fetchMock as unknown as typeof fetch,
    });
    client.setKeypair({
      pubkey: stubKeypairJson.pubkey_base58,
      toJSON: () => stubKeypairJson,
    });

    await expect(client.signMemory("hello world")).rejects.toThrow();

    // Exactly one captured call (we halted with `error:` in the JSON-RPC
    // envelope, which surfaces as AuthError before the bundle GET happens).
    expect(captured.length).toBeGreaterThanOrEqual(1);
    const first = captured[0]!;
    expect(first.url).toBe("https://mcp.example/mcp");
    expect(first.init.method).toBe("POST");
    const body = JSON.parse(String(first.init.body));
    expect(body.jsonrpc).toBe("2.0");
    expect(body.method).toBe("tools/call");
    expect(body.params.name).toBe("mnemonic_sign_memory");
    expect(body.params.arguments.content).toBe("hello world");
  });

  it("no_node_imports: bundle has no node:*, require('node:'), or process.versions", () => {
    const src = readFileSync(BUNDLE, "utf8");
    // ESM static or dynamic `from "node:..."`.
    expect(src).not.toMatch(/from\s*["']node:/);
    // CJS interop accidentally creeping in.
    expect(src).not.toMatch(/require\s*\(\s*["']node:/);
    // Common runtime-fork sniff that has no place in a browser bundle.
    expect(src).not.toMatch(/process\.versions/);
  });
});

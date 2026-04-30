// Integration: recall happy path + verify discriminants (verified / tampered
// / not_found) against the mock server.

import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
} from "vitest";

import { MnemonicClient } from "../../src/client.js";
import { AuthError } from "../../src/errors.js";
import { Keypair } from "../../src/keypair.js";
import { LocalSigner } from "../../src/signer.js";
import { __setWasmForTesting } from "../../src/wasm.js";
import { buildWasmMock } from "../helpers/wasm-mock.js";
import { startMockServer, type MockServer } from "../mock-server.js";

let server: MockServer;

beforeAll(async () => {
  server = await startMockServer();
});

afterAll(async () => {
  await server.close();
});

beforeEach(() => {
  server.reset();
  __setWasmForTesting(buildWasmMock() as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

async function makeClient(): Promise<MnemonicClient> {
  const keypair = await Keypair.generate();
  const client = new MnemonicClient({
    baseUrl: server.url,
    signer: new LocalSigner(keypair),
    jwt: "eyJfake-jwt-1234567890abcdefghijk",
  });
  client.setKeypair(keypair);
  return client;
}

describe("recall", () => {
  it("returns hits with similarity + tags", async () => {
    const client = await makeClient();
    const result = await client.recall("test query", {
      topK: 5,
      tags: ["a"],
    });
    expect(result.total).toBe(1);
    expect(result.hits).toHaveLength(1);
    expect(result.hits[0]!.attestationId).toBe("att-mock-1");
    expect(result.hits[0]!.similarity).toBeGreaterThan(0.9);
    expect(result.hits[0]!.tags).toEqual(["a"]);
  });
});

describe("verify discriminants", () => {
  it("verified", async () => {
    const client = await makeClient();
    const result = await client.verify("att-good-1");
    expect(result.status).toBe("verified");
    if (result.status === "verified") {
      expect(result.signer).toBe("MockSigner1");
      expect(result.arweaveTx).toBe("mock-arweave");
    }
  });

  it("tampered", async () => {
    const client = await makeClient();
    const result = await client.verify("tampered-1");
    expect(result.status).toBe("tampered");
    if (result.status === "tampered") {
      expect(result.reason).toBe("content_hash mismatch");
    }
  });

  it("not_found", async () => {
    const client = await makeClient();
    const result = await client.verify("missing-1");
    expect(result.status).toBe("not_found");
  });
});

// ── fault: expired-jwt-after-N-requests ─────────────────────────────────────
// Tech-spec § Testing Strategy lists this as one of five required fault paths
// and demands "each integration test exercises at least one fault path".
// The fault gates /mcp + /api/* with HTTP 401 after N successful requests;
// the SDK MUST surface that as `AuthError` (CLI exit 4 per Decision 10),
// never silently degrade or swallow it mid-flight.
describe("fault: expired-jwt-after-N-requests", () => {
  it("after N OK calls, the (N+1)th surfaces AuthError on /mcp", async () => {
    server.withFault("expired-jwt-after-N-requests", {
      requestsBeforeExpiry: 2,
    });
    const client = await makeClient();

    // Two recall calls succeed (the mock counts /mcp + /api/* hits;
    // each recall is a single /mcp POST).
    await client.recall("first", { topK: 1 });
    await client.recall("second", { topK: 1 });

    // Third call: server flips to 401, SDK MUST throw AuthError.
    await expect(client.recall("third", { topK: 1 })).rejects.toBeInstanceOf(
      AuthError
    );
  });
});

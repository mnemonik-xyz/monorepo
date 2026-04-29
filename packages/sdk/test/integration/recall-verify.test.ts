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

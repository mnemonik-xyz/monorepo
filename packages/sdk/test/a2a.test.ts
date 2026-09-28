// Unit tests for MnemonicClient A2A methods:
//   attestA2ATask, attestA2AMessage, attestA2AArtifact, recallA2AContext
//
// Plus T14 conformance vectors for sealed A2A DataPart helpers:
//   buildSealedDataPart, extractSealedDataPart, SEALED_CBOR_MEDIA_TYPE
//
// Uses the same mock-fetch pattern as client.test.ts.

import { beforeEach, afterEach, describe, expect, it } from "vitest";

import { MnemonicClient } from "../src/client.js";
import { ServerError, UserError } from "../src/errors.js";
import { Keypair } from "../src/keypair.js";
import { LocalSigner } from "../src/signer.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";
import {
  SEALED_CBOR_MEDIA_TYPE,
  buildSealedDataPart,
  extractSealedDataPart,
} from "../src/a2a.js";
import type {
  A2AArtifact,
  A2AMessage,
  A2ATask,
} from "../src/types.js";

// ── mock-fetch helpers ──────────────────────────────────────────────────────

interface CapturedCall {
  url: string;
  method: string;
  body: unknown;
}

function mcpResult(result: unknown): Record<string, unknown> {
  return {
    jsonrpc: "2.0",
    id: 1,
    result: {
      content: [{ type: "text", text: JSON.stringify(result) }],
    },
  };
}

function makeMockFetch(responses: Array<{ status?: number; body: unknown }>): {
  fetchImpl: typeof fetch;
  calls: CapturedCall[];
} {
  const calls: CapturedCall[] = [];
  let cursor = 0;
  const fetchImpl = (async (
    url: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const u = typeof url === "string" ? url : url.toString();
    let parsedBody: unknown = init?.body;
    if (typeof init?.body === "string") {
      try {
        parsedBody = JSON.parse(init.body);
      } catch {
        parsedBody = init.body;
      }
    }
    calls.push({ url: u, method: (init?.method ?? "GET").toUpperCase(), body: parsedBody });
    const r = responses[cursor++] ?? { status: 200, body: {} };
    return new Response(JSON.stringify(r.body), {
      status: r.status ?? 200,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
  return { fetchImpl, calls };
}

// ── shared client factory ───────────────────────────────────────────────────

async function makeClient(responses: Array<{ status?: number; body: unknown }>): Promise<{
  client: MnemonicClient;
  calls: CapturedCall[];
}> {
  const wasm = buildWasmMock();
  const kp = wasm.generate_keypair();
  const keypair = await Keypair.fromJSON(kp);
  const signer = new LocalSigner(keypair);
  const { fetchImpl, calls } = makeMockFetch(responses);
  const client = new MnemonicClient({
    baseUrl: "https://mcp.mnemonik.xyz",
    signer,
    jwt: "test-jwt",
    fetch: fetchImpl,
  });
  return { client, calls };
}

// ── fixtures ─────────────────────────────────────────────────────────────────

const TASK: A2ATask = {
  id: "task-001",
  contextId: "ctx-abc",
  status: { state: "completed", timestamp: "2026-01-01T00:00:00Z" },
};

const MESSAGE: A2AMessage = {
  messageId: "msg-001",
  taskId: "task-001",
  contextId: "ctx-abc",
  role: "agent",
  parts: [{ type: "text", text: "Hello" }],
};

const ARTIFACT: A2AArtifact = {
  artifactId: "art-001",
  taskId: "task-001",
  name: "output.txt",
  parts: [{ type: "text", text: "result" }],
};

// ── setup / teardown ──────────────────────────────────────────────────────────

beforeEach(() => {
  __setWasmForTesting(buildWasmMock() as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

// ── attestA2ATask ─────────────────────────────────────────────────────────────

describe("attestA2ATask", () => {
  it("calls mnemonic_attest_a2a with kind=task and returns attestation_id", async () => {
    const { client, calls } = await makeClient([
      { body: mcpResult({ attestation_id: "att-task-1" }) },
    ]);

    const id = await client.attestA2ATask(TASK, "ctx-abc");

    expect(id).toBe("att-task-1");
    expect(calls).toHaveLength(1);
    const reqBody = calls[0]!.body as { params: { name: string; arguments: Record<string, unknown> } };
    expect(reqBody.params.name).toBe("mnemonic_attest_a2a");
    expect(reqBody.params.arguments.kind).toBe("task");
    expect(reqBody.params.arguments.context_id).toBe("ctx-abc");
    const payload = reqBody.params.arguments.payload as A2ATask;
    expect(payload.id).toBe("task-001");
  });

  it("forwards prevId as prev_id", async () => {
    const { client, calls } = await makeClient([
      { body: mcpResult({ attestation_id: "att-2" }) },
    ]);
    await client.attestA2ATask(TASK, "ctx-abc", { prevId: "att-prev" });
    const args = (calls[0]!.body as { params: { arguments: Record<string, unknown> } }).params.arguments;
    expect(args.prev_id).toBe("att-prev");
  });

  it("throws UserError when task.id is missing", async () => {
    const { client } = await makeClient([]);
    await expect(
      client.attestA2ATask({ ...TASK, id: "" }, "ctx-abc")
    ).rejects.toBeInstanceOf(UserError);
  });

  it("throws UserError when contextId is missing", async () => {
    const { client } = await makeClient([]);
    await expect(
      client.attestA2ATask(TASK, "")
    ).rejects.toBeInstanceOf(UserError);
  });

  it("throws ServerError when server omits attestation_id", async () => {
    const { client } = await makeClient([
      { body: mcpResult({ status: "ok" }) },
    ]);
    await expect(client.attestA2ATask(TASK, "ctx-abc")).rejects.toBeInstanceOf(
      ServerError
    );
  });
});

// ── attestA2AMessage ──────────────────────────────────────────────────────────

describe("attestA2AMessage", () => {
  it("calls mnemonic_attest_a2a with kind=message and returns attestation_id", async () => {
    const { client, calls } = await makeClient([
      { body: mcpResult({ attestation_id: "att-msg-1" }) },
    ]);

    const id = await client.attestA2AMessage(MESSAGE, "ctx-abc");

    expect(id).toBe("att-msg-1");
    const args = (calls[0]!.body as { params: { arguments: Record<string, unknown> } }).params.arguments;
    expect(args.kind).toBe("message");
    expect(args.context_id).toBe("ctx-abc");
  });

  it("throws UserError when messageId is missing", async () => {
    const { client } = await makeClient([]);
    await expect(
      client.attestA2AMessage({ ...MESSAGE, messageId: "" }, "ctx-abc")
    ).rejects.toBeInstanceOf(UserError);
  });
});

// ── attestA2AArtifact ─────────────────────────────────────────────────────────

describe("attestA2AArtifact", () => {
  it("calls mnemonic_attest_a2a with kind=artifact and returns attestation_id", async () => {
    const { client, calls } = await makeClient([
      { body: mcpResult({ attestation_id: "att-art-1" }) },
    ]);

    const id = await client.attestA2AArtifact(ARTIFACT, "ctx-abc");

    expect(id).toBe("att-art-1");
    const args = (calls[0]!.body as { params: { arguments: Record<string, unknown> } }).params.arguments;
    expect(args.kind).toBe("artifact");
  });

  it("throws UserError when artifactId is missing", async () => {
    const { client } = await makeClient([]);
    await expect(
      client.attestA2AArtifact({ ...ARTIFACT, artifactId: "" }, "ctx-abc")
    ).rejects.toBeInstanceOf(UserError);
  });
});

// ── recallA2AContext ──────────────────────────────────────────────────────────

describe("recallA2AContext", () => {
  it("calls mnemonic_recall_a2a with context_id and returns attestations", async () => {
    const { client, calls } = await makeClient([
      {
        body: mcpResult({
          attestations: [
            {
              attestation_id: "att-1",
              kind: "task",
              signed_at: "2026-01-01T00:00:00Z",
              payload: { id: "task-001" },
            },
            {
              attestation_id: "att-2",
              kind: "message",
              signed_at: "2026-01-01T00:01:00Z",
              payload: { messageId: "msg-001" },
            },
          ],
        }),
      },
    ]);

    const results = await client.recallA2AContext("ctx-abc");

    expect(results).toHaveLength(2);
    expect(results[0]!.attestationId).toBe("att-1");
    expect(results[0]!.kind).toBe("task");
    expect(results[1]!.kind).toBe("message");

    const args = (calls[0]!.body as { params: { arguments: Record<string, unknown> } }).params.arguments;
    expect(args.context_id).toBe("ctx-abc");
  });

  it("passes limit and kind filter when provided", async () => {
    const { client, calls } = await makeClient([
      { body: mcpResult({ attestations: [] }) },
    ]);
    await client.recallA2AContext("ctx-abc", { limit: 3, kind: "message" });
    const args = (calls[0]!.body as { params: { arguments: Record<string, unknown> } }).params.arguments;
    expect(args.limit).toBe(3);
    expect(args.kind).toBe("message");
  });

  it("does NOT send kind when kind='all'", async () => {
    const { client, calls } = await makeClient([
      { body: mcpResult({ attestations: [] }) },
    ]);
    await client.recallA2AContext("ctx-abc", { kind: "all" });
    const args = (calls[0]!.body as { params: { arguments: Record<string, unknown> } }).params.arguments;
    expect(args.kind).toBeUndefined();
  });

  it("handles hits array alias", async () => {
    const { client } = await makeClient([
      {
        body: mcpResult({
          hits: [
            {
              attestation_id: "att-x",
              kind: "artifact",
              signed_at: "2026-01-01T00:00:00Z",
              payload: { artifactId: "a1" },
            },
          ],
        }),
      },
    ]);
    const results = await client.recallA2AContext("ctx-xyz");
    expect(results[0]!.attestationId).toBe("att-x");
    expect(results[0]!.kind).toBe("artifact");
  });

  it("returns empty array when server returns no attestations", async () => {
    const { client } = await makeClient([
      { body: mcpResult({}) },
    ]);
    const results = await client.recallA2AContext("ctx-abc");
    expect(results).toEqual([]);
  });

  it("throws UserError when contextId is empty", async () => {
    const { client } = await makeClient([]);
    await expect(client.recallA2AContext("")).rejects.toBeInstanceOf(UserError);
  });
});

// ── T14 conformance: sealed A2A DataPart ──────────────────────────────────────

describe("buildSealedDataPart / extractSealedDataPart (T14 §9.1)", () => {
  it("SEALED_CBOR_MEDIA_TYPE has the expected value", () => {
    expect(SEALED_CBOR_MEDIA_TYPE).toBe("application/vnd.mnemonic.sealed+cbor");
  });

  it("buildSealedDataPart produces a DataPart with correct mimeType", () => {
    const fakeSealed = new Uint8Array([1, 2, 3, 4]);
    const part = buildSealedDataPart(fakeSealed, []);
    expect(part.kind).toBe("data");
    expect(part.mimeType).toBe(SEALED_CBOR_MEDIA_TYPE);
    expect(typeof part.data.sealed).toBe("string");
    expect(part.data.grants).toEqual([]);
  });

  it("round-trips sealed + grants through build/extract", () => {
    const sealed = new Uint8Array([10, 20, 30, 40, 50]);
    const grant1 = new Uint8Array([11, 22, 33]);
    const grant2 = new Uint8Array([44, 55, 66]);

    const part = buildSealedDataPart(sealed, [grant1, grant2]);
    const { sealedCbor, grants } = extractSealedDataPart(part);

    expect(sealedCbor).toEqual(sealed);
    expect(grants).toHaveLength(2);
    expect(grants[0]).toEqual(grant1);
    expect(grants[1]).toEqual(grant2);
  });

  it("extractSealedDataPart throws UserError for non-data kind", () => {
    expect(() =>
      extractSealedDataPart({
        kind: "text",
        mimeType: SEALED_CBOR_MEDIA_TYPE,
      })
    ).toThrow(UserError);
  });

  it("extractSealedDataPart throws UserError for wrong mimeType", () => {
    expect(() =>
      extractSealedDataPart({
        kind: "data",
        data: { sealed: "AAAA", grants: [] },
        mimeType: "application/json",
      })
    ).toThrow(UserError);
  });

  it("extractSealedDataPart throws UserError when data.sealed is missing", () => {
    expect(() =>
      extractSealedDataPart({
        kind: "data",
        data: { grants: [] },
        mimeType: SEALED_CBOR_MEDIA_TYPE,
      })
    ).toThrow(UserError);
  });

  it("buildSealedDataPart with no grants produces empty grants array", () => {
    const part = buildSealedDataPart(new Uint8Array([99]));
    expect(part.data.grants).toEqual([]);
    // extracting with no grants should work
    const { grants } = extractSealedDataPart(part);
    expect(grants).toEqual([]);
  });

  it("empty Uint8Array round-trips", () => {
    const part = buildSealedDataPart(new Uint8Array(0));
    const { sealedCbor } = extractSealedDataPart(part);
    expect(sealedCbor).toEqual(new Uint8Array(0));
  });
});

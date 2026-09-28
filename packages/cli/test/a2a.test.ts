// CLI a2a subcommands: attest, recall, verify.
//
// Tests verify:
//   - runA2AAttest: reads a JSON fixture, calls attestA2A*, prints attestation_id
//   - runA2ARecall: calls recallA2AContext, prints results
//   - runA2AVerify: delegates to runVerify with the attestation id
//   - UserError on missing/invalid options

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runA2AAttest, runA2ARecall, runA2AVerify } from "../src/commands/a2a.js";
import { saveIdentityJson, saveToken } from "../src/config.js";
import { UserError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

// ── helpers ──────────────────────────────────────────────────────────────────

function mcpReply(result: unknown): Response {
  return new Response(
    JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      result: { content: [{ type: "text", text: JSON.stringify(result) }] },
    }),
    { status: 200, headers: { "content-type": "application/json" } }
  );
}

// ── setup / teardown ──────────────────────────────────────────────────────────

let cleanup = (): void => {};
let mock: ReturnType<typeof installWasmMock>;
let realFetch: typeof globalThis.fetch | undefined;
let tmpDir: string;

beforeEach(() => {
  const result = withTmpConfigDir();
  cleanup = result.cleanup;
  tmpDir = result.dir;
  mock = installWasmMock();
  realFetch = globalThis.fetch;
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  vi.spyOn(process.stderr, "write").mockImplementation(() => true);

  const kp = mock.generate_keypair();
  saveIdentityJson(kp);
  saveToken({
    jwt: makeJwt(kp.pubkey_base58),
    expires_at: new Date(Date.now() + 3600_000).toISOString(),
    sub: kp.pubkey_base58,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  clearWasmMock();
  if (realFetch) globalThis.fetch = realFetch;
  cleanup();
});

// ── runA2AAttest ──────────────────────────────────────────────────────────────

describe("runA2AAttest", () => {
  const TASK_FIXTURE = {
    id: "task-001",
    contextId: "ctx-abc",
    status: { state: "completed" },
  };

  function writeFixture(name: string, obj: unknown): string {
    const path = join(tmpDir, name);
    writeFileSync(path, JSON.stringify(obj), "utf-8");
    return path;
  }

  it("attests a task file and prints attestation_id", async () => {
    const file = writeFixture("task.json", TASK_FIXTURE);
    globalThis.fetch = vi.fn(async () =>
      mcpReply({ attestation_id: "att-task-cli-1" })
    );

    await runA2AAttest({
      kind: "task",
      file,
      context: "ctx-abc",
    });

    const written = (process.stdout.write as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0])
      .join("");
    expect(written).toContain("att-task-cli-1");
  });

  it("attests a message file with --kind message", async () => {
    const msg = { messageId: "msg-001", role: "agent", parts: [] };
    const file = writeFixture("message.json", msg);
    globalThis.fetch = vi.fn(async () =>
      mcpReply({ attestation_id: "att-msg-cli-1" })
    );

    await runA2AAttest({ kind: "message", file, context: "ctx-abc" });

    const written = (process.stdout.write as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0])
      .join("");
    expect(written).toContain("att-msg-cli-1");
  });

  it("attests an artifact file with --kind artifact", async () => {
    const art = { artifactId: "art-001", parts: [] };
    const file = writeFixture("art.json", art);
    globalThis.fetch = vi.fn(async () =>
      mcpReply({ attestation_id: "att-art-cli-1" })
    );

    await runA2AAttest({ kind: "artifact", file, context: "ctx-abc" });

    const written = (process.stdout.write as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0])
      .join("");
    expect(written).toContain("att-art-cli-1");
  });

  it("throws UserError for invalid kind", async () => {
    const file = writeFixture("task.json", TASK_FIXTURE);
    await expect(
      // @ts-expect-error testing invalid kind
      runA2AAttest({ kind: "unknown", file, context: "ctx-abc" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("throws UserError when --file is missing", async () => {
    await expect(
      runA2AAttest({ kind: "task", file: "", context: "ctx-abc" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("throws UserError when --context is missing", async () => {
    const file = writeFixture("task.json", TASK_FIXTURE);
    await expect(
      runA2AAttest({ kind: "task", file, context: "" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("throws UserError when file does not exist", async () => {
    await expect(
      runA2AAttest({ kind: "task", file: "/nonexistent/task.json", context: "ctx" })
    ).rejects.toBeInstanceOf(UserError);
  });
});

// ── runA2ARecall ──────────────────────────────────────────────────────────────

describe("runA2ARecall", () => {
  it("prints attestations returned by the server", async () => {
    globalThis.fetch = vi.fn(async () =>
      mcpReply({
        attestations: [
          {
            attestation_id: "att-recall-1",
            kind: "task",
            signed_at: "2026-01-01T00:00:00Z",
            payload: { id: "task-001" },
          },
        ],
      })
    );

    await runA2ARecall({ context: "ctx-abc" });

    const written = (process.stdout.write as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0])
      .join("");
    expect(written).toContain("att-recall-1");
    expect(written).toContain("task");
  });

  it("prints 'no attestations' when server returns empty list", async () => {
    globalThis.fetch = vi.fn(async () => mcpReply({ attestations: [] }));
    await runA2ARecall({ context: "ctx-empty" });
    const written = (process.stdout.write as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0])
      .join("");
    expect(written).toContain("no attestations");
  });

  it("throws UserError when --context is missing", async () => {
    await expect(runA2ARecall({ context: "" })).rejects.toBeInstanceOf(UserError);
  });
});

// ── runA2AVerify ──────────────────────────────────────────────────────────────

describe("runA2AVerify", () => {
  it("delegates to runVerify and prints verified status", async () => {
    globalThis.fetch = vi.fn(async () =>
      mcpReply({ status: "verified", signer: "pubkey-abc" })
    );

    await runA2AVerify({ attestation: "att-to-verify-1" });

    const written = (process.stdout.write as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0])
      .join("");
    expect(written).toContain("verified");
  });

  it("throws UserError when --attestation is missing", async () => {
    await expect(runA2AVerify({ attestation: "" })).rejects.toBeInstanceOf(
      UserError
    );
  });
});

// A2A attestation helpers — mixin methods attached to MnemonicClient.
//
// All four methods wrap `callTool("mnemonic_attest_a2a" | "mnemonic_recall_a2a", ...)`
// through the same MCP HTTP surface used by `signMemory` / `recall`.
//
// attestA2ATask / attestA2AMessage / attestA2AArtifact
//   → POST /mcp  tools/call mnemonic_attest_a2a
//   → returns `AttestationId`
//
// recallA2AContext
//   → POST /mcp  tools/call mnemonic_recall_a2a
//   → returns `Attestation[]`
//
// The methods are declared here and merged onto MnemonicClient in client.ts
// via `Object.assign(MnemonicClient.prototype, a2aMethods)`. They share the
// private `callTool` surface via the `WithCallTool` interface below.

import { ServerError, UserError } from "./errors.js";
import type {
  A2AArtifact,
  A2AMessage,
  A2ATask,
  AttestA2AOptions,
  AttestationId,
  Attestation,
  RecallA2AContextOptions,
} from "./types.js";

// ── Internal interface so a2aMethods can reach into MnemonicClient ───────────

/**
 * Minimal interface that the A2A mixin requires from its host class.
 * `MnemonicClient` satisfies this contract.
 */
export interface WithCallTool {
  /** Exposed as a protected-by-convention underscore method for mixin access. */
  _callToolA2A(name: string, args: Record<string, unknown>): Promise<unknown>;
}

// ── Helper ──────────────────────────────────────────────────────────────────

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Parse the `attestation_id` out of a `mnemonic_attest_a2a` response.
 */
function parseAttestationId(result: unknown, tool: string): AttestationId {
  const raw = isRecord(result) ? result : {};
  const id = typeof raw.attestation_id === "string" ? raw.attestation_id : null;
  if (!id) {
    throw new ServerError(
      `${tool}: server did not return attestation_id; got ${JSON.stringify(raw)}`
    );
  }
  return id;
}

/**
 * Parse `Attestation[]` out of a `mnemonic_recall_a2a` response.
 */
function parseAttestations(result: unknown): Attestation[] {
  const raw = isRecord(result) ? result : {};
  const items = Array.isArray(raw.attestations)
    ? raw.attestations
    : Array.isArray(raw.hits)
    ? raw.hits
    : [];
  return items.filter(isRecord).map((item) => ({
    attestationId:
      typeof item.attestation_id === "string" ? item.attestation_id : "",
    kind: (item.kind === "task" || item.kind === "message" || item.kind === "artifact"
      ? item.kind
      : "task") as "task" | "message" | "artifact",
    signedAt: typeof item.signed_at === "string" ? item.signed_at : new Date().toISOString(),
    payload: isRecord(item.payload) ? item.payload : {},
  }));
}

// ── A2A method implementations ───────────────────────────────────────────────

/**
 * Attest an A2A task object. Serialises the task to JSON, sends it to
 * `mnemonic_attest_a2a` on the MCP server, and returns the opaque
 * `attestation_id`.
 */
export async function attestA2ATask(
  this: WithCallTool,
  task: A2ATask,
  contextId: string,
  opts: AttestA2AOptions = {}
): Promise<AttestationId> {
  if (!task || !task.id) {
    throw new UserError("attestA2ATask: task must have a non-empty id");
  }
  if (!contextId || typeof contextId !== "string") {
    throw new UserError("attestA2ATask: contextId must be a non-empty string");
  }
  const args: Record<string, unknown> = {
    kind: "task",
    context_id: contextId,
    payload: task,
  };
  if (opts.prevId) args.prev_id = opts.prevId;
  const result = await this._callToolA2A("mnemonic_attest_a2a", args);
  return parseAttestationId(result, "attestA2ATask");
}

/**
 * Attest an A2A message object and return the `attestation_id`.
 */
export async function attestA2AMessage(
  this: WithCallTool,
  msg: A2AMessage,
  contextId: string,
  opts: AttestA2AOptions = {}
): Promise<AttestationId> {
  if (!msg || !msg.messageId) {
    throw new UserError("attestA2AMessage: msg must have a non-empty messageId");
  }
  if (!contextId || typeof contextId !== "string") {
    throw new UserError("attestA2AMessage: contextId must be a non-empty string");
  }
  const args: Record<string, unknown> = {
    kind: "message",
    context_id: contextId,
    payload: msg,
  };
  if (opts.prevId) args.prev_id = opts.prevId;
  const result = await this._callToolA2A("mnemonic_attest_a2a", args);
  return parseAttestationId(result, "attestA2AMessage");
}

/**
 * Attest an A2A artifact object and return the `attestation_id`.
 */
export async function attestA2AArtifact(
  this: WithCallTool,
  art: A2AArtifact,
  contextId: string,
  opts: AttestA2AOptions = {}
): Promise<AttestationId> {
  if (!art || !art.artifactId) {
    throw new UserError(
      "attestA2AArtifact: artifact must have a non-empty artifactId"
    );
  }
  if (!contextId || typeof contextId !== "string") {
    throw new UserError(
      "attestA2AArtifact: contextId must be a non-empty string"
    );
  }
  const args: Record<string, unknown> = {
    kind: "artifact",
    context_id: contextId,
    payload: art,
  };
  if (opts.prevId) args.prev_id = opts.prevId;
  const result = await this._callToolA2A("mnemonic_attest_a2a", args);
  return parseAttestationId(result, "attestA2AArtifact");
}

/**
 * Recall attestations for a given A2A context.
 *
 * Returns all attestations (tasks, messages, artifacts) anchored under
 * `contextId`, optionally filtered by `kind` and limited to `limit` items.
 */
export async function recallA2AContext(
  this: WithCallTool,
  contextId: string,
  opts: RecallA2AContextOptions = {}
): Promise<Attestation[]> {
  if (!contextId || typeof contextId !== "string") {
    throw new UserError(
      "recallA2AContext: contextId must be a non-empty string"
    );
  }
  const args: Record<string, unknown> = { context_id: contextId };
  if (typeof opts.limit === "number") args.limit = opts.limit;
  if (opts.kind && opts.kind !== "all") args.kind = opts.kind;
  const result = await this._callToolA2A("mnemonic_recall_a2a", args);
  return parseAttestations(result);
}

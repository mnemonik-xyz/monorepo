// `mnemonic a2a <attest|recall|verify>` — A2A attestation subcommands.
//
// Subcommands:
//   mnemonic a2a attest --kind <task|message|artifact> --file <path> --context <id> [--prev <id>]
//     Read an A2A object from a JSON file and attest it. Prints the attestation_id.
//
//   mnemonic a2a recall --context <id> [--limit N] [--kind <task|message|artifact|all>]
//     Recall attestations for an A2A context.
//
//   mnemonic a2a verify --attestation <id>
//     Verify an attestation by its ID (delegates to the existing verify command).

import { readFileSync } from "node:fs";

import { fromSdkError, UserError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";
import { openSession } from "../session.js";
import { runVerify } from "./verify.js";

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

// ── attest ────────────────────────────────────────────────────────────────────

export interface A2AAttestOptions extends OutputOptions {
  /** Object kind: task | message | artifact */
  kind: "task" | "message" | "artifact";
  /** Path to a JSON file containing the A2A object */
  file: string;
  /** Context identifier */
  context: string;
  /** Optional previous attestation ID */
  prev?: string;
  baseUrl?: string;
}

export async function runA2AAttest(opts: A2AAttestOptions): Promise<void> {
  const { kind, file, context, prev } = opts;

  if (!kind || !["task", "message", "artifact"].includes(kind)) {
    throw new UserError(
      "a2a attest: --kind must be one of: task, message, artifact"
    );
  }
  if (!file || file.length === 0) {
    throw new UserError("a2a attest: --file is required");
  }
  if (!context || context.length === 0) {
    throw new UserError("a2a attest: --context is required");
  }

  // Read the A2A object from the JSON file.
  let payload: unknown;
  try {
    const raw = readFileSync(file, "utf-8");
    payload = JSON.parse(raw);
  } catch (e) {
    throw new UserError(
      `a2a attest: failed to read or parse --file ${file}: ${e instanceof Error ? e.message : String(e)}`
    );
  }

  const baseUrl = opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const { client } = await openSession(baseUrl, opts, false);

  const attestOpts = prev ? { prevId: prev } : {};
  let attestationId: string;

  try {
    if (kind === "task") {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      attestationId = await client.attestA2ATask(payload as any, context, attestOpts);
    } else if (kind === "message") {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      attestationId = await client.attestA2AMessage(payload as any, context, attestOpts);
    } else {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      attestationId = await client.attestA2AArtifact(payload as any, context, attestOpts);
    }
  } catch (e) {
    throw fromSdkError(e);
  }

  format({ attestationId }, opts, (_d, _color) => {
    return `attestation_id: ${attestationId}`;
  });
}

// ── recall ────────────────────────────────────────────────────────────────────

export interface A2ARecallOptions extends OutputOptions {
  context: string;
  limit?: number;
  kind?: "task" | "message" | "artifact" | "all";
  baseUrl?: string;
}

export async function runA2ARecall(opts: A2ARecallOptions): Promise<void> {
  if (!opts.context || opts.context.length === 0) {
    throw new UserError("a2a recall: --context is required");
  }

  const baseUrl = opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const { client } = await openSession(baseUrl, opts, false);

  let results;
  try {
    results = await client.recallA2AContext(opts.context, {
      ...(typeof opts.limit === "number" ? { limit: opts.limit } : {}),
      ...(opts.kind ? { kind: opts.kind } : {}),
    });
  } catch (e) {
    throw fromSdkError(e);
  }

  format({ results }, opts, (_d, _color) => {
    if (results.length === 0) {
      return `no attestations found for context: ${opts.context}`;
    }
    const lines = [`${results.length} attestation(s) for context ${opts.context}:`];
    for (const a of results) {
      lines.push(
        `  ${a.attestationId.slice(0, 16)}  kind=${a.kind}  signed_at=${a.signedAt}`
      );
    }
    return lines.join("\n");
  });
}

// ── verify (re-uses existing verify command) ──────────────────────────────────

export interface A2AVerifyOptions extends OutputOptions {
  attestation: string;
  baseUrl?: string;
}

export async function runA2AVerify(opts: A2AVerifyOptions): Promise<void> {
  if (!opts.attestation || opts.attestation.length === 0) {
    throw new UserError("a2a verify: --attestation is required");
  }
  // Delegate to the existing verify command; same contract.
  return runVerify(opts.attestation, { ...opts });
}

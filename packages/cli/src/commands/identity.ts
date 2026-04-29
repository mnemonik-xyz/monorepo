// `mnemonic identity {import,export}` — round-trip the keypair between
// the webapp's "Send to CLI" bootstrap flow (Decision 7) and a local file.
//
// `import --ticket <uuid>`: fetch GET ${baseUrl}/api/cli-bootstrap/redeem/:ticket
//   (no Bearer — the UUID itself is the capability, single-use server-side),
//   parse JSON {secret, pubkey_base58}, save via saveIdentity. Refuses to
//   overwrite an existing identity unless --force.
// `import --file <path>`: read JSON from disk, validate shape, save.
// `export --file <path>`: write current ~/.mnemonic/identity.json to <path>
//   with mode 0600 (or icacls-restricted ACL on Windows). NO clipboard flag.

import { readFileSync, writeFileSync } from "node:fs";

import type { KeypairJson } from "@mnemonik-xyz/sdk";

import {
  identityExists,
  identityPath,
  loadIdentityJson,
  restrictFileMode,
  saveIdentityJson,
} from "../config.js";
import { AuthError, fromSdkError, ServerError, UserError } from "../errors.js";
import { format, hint, type OutputOptions } from "../output.js";

const DEFAULT_BASE_URL = "https://mc.mnemonik.xyz";

export interface IdentityImportOptions extends OutputOptions {
  ticket?: string;
  file?: string;
  force?: boolean;
  baseUrl?: string;
}

export interface IdentityExportOptions extends OutputOptions {
  file?: string;
}

/** Validate parsed JSON looks like a `KeypairJson`. */
function validateKeypairJson(v: unknown): KeypairJson {
  if (!v || typeof v !== "object") {
    throw new UserError("identity payload is not an object");
  }
  const o = v as Record<string, unknown>;
  if (
    !Array.isArray(o.secret) ||
    o.secret.length !== 64 ||
    !o.secret.every((n) => typeof n === "number")
  ) {
    throw new UserError(
      "identity payload: `secret` must be a 64-element number array"
    );
  }
  if (typeof o.pubkey_base58 !== "string" || o.pubkey_base58.length === 0) {
    throw new UserError(
      "identity payload: `pubkey_base58` must be a non-empty string"
    );
  }
  return { secret: o.secret as number[], pubkey_base58: o.pubkey_base58 };
}

export async function runIdentityImport(
  opts: IdentityImportOptions
): Promise<void> {
  if (!opts.ticket && !opts.file) {
    throw new UserError(
      "identity import: pass either --ticket <uuid> or --file <path>"
    );
  }
  if (opts.ticket && opts.file) {
    throw new UserError(
      "identity import: --ticket and --file are mutually exclusive"
    );
  }

  // Refuse to overwrite an existing identity unless --force.
  if (identityExists() && !opts.force) {
    throw new UserError(
      `identity already exists at ${identityPath()}; pass --force to overwrite ` +
        `(this will replace your keypair — use \`mnemonic identity export\` first)`
    );
  }

  let payload: KeypairJson;
  if (opts.ticket) {
    payload = await fetchTicket(opts.ticket, opts);
  } else {
    payload = readKeypairFile(opts.file as string);
  }

  saveIdentityJson(payload);

  format(
    {
      pubkey: payload.pubkey_base58,
      did: `did:sol:${payload.pubkey_base58}`,
      path: identityPath(),
    },
    opts,
    () =>
      [
        `identity imported: ${identityPath()}`,
        `pubkey: ${payload.pubkey_base58}`,
        `did:    did:sol:${payload.pubkey_base58}`,
      ].join("\n")
  );
}

async function fetchTicket(
  ticket: string,
  opts: IdentityImportOptions
): Promise<KeypairJson> {
  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const url = `${baseUrl.replace(
    /\/$/,
    ""
  )}/api/cli-bootstrap/redeem/${encodeURIComponent(ticket)}`;
  hint(`redeeming ticket against ${baseUrl}...`, opts);

  let res: Response;
  try {
    res = await fetch(url, { method: "GET" });
  } catch (e) {
    throw new ServerError(
      `network error fetching ${url}: ${
        e instanceof Error ? e.message : String(e)
      }`,
      e
    );
  }

  if (res.status === 410) {
    throw new UserError(
      `ticket ${ticket} has already been redeemed (server returned 410 Gone)`
    );
  }
  if (res.status === 404) {
    throw new UserError(
      `ticket ${ticket} not found or expired (server returned 404)`
    );
  }
  if (res.status === 401 || res.status === 403) {
    throw new AuthError(`ticket redemption rejected (HTTP ${res.status})`);
  }
  if (!res.ok) {
    let body = "";
    try {
      body = await res.text();
    } catch {
      // ignore
    }
    throw new ServerError(
      `ticket redemption failed (HTTP ${res.status}): ${body.slice(0, 500)}`
    );
  }

  let parsed: unknown;
  try {
    parsed = await res.json();
  } catch (e) {
    throw fromSdkError(e);
  }
  return validateKeypairJson(parsed);
}

function readKeypairFile(path: string): KeypairJson {
  let raw: string;
  try {
    raw = readFileSync(path, "utf8");
  } catch (e) {
    throw new UserError(
      `cannot read ${path}: ${e instanceof Error ? e.message : String(e)}`,
      e
    );
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (e) {
    throw new UserError(
      `${path} is not valid JSON: ${
        e instanceof Error ? e.message : String(e)
      }`,
      e
    );
  }
  return validateKeypairJson(parsed);
}

export async function runIdentityExport(
  opts: IdentityExportOptions
): Promise<void> {
  if (!opts.file) {
    throw new UserError("identity export: --file <path> is required");
  }
  const json = loadIdentityJson(); // throws UserError if missing
  const path = opts.file;
  try {
    writeFileSync(path, JSON.stringify(json, null, 2), { mode: 0o600 });
  } catch (e) {
    throw new UserError(
      `cannot write ${path}: ${e instanceof Error ? e.message : String(e)}`,
      e
    );
  }
  restrictFileMode(path);

  format(
    {
      pubkey: json.pubkey_base58,
      path,
    },
    opts,
    () =>
      [
        `identity exported: ${path}`,
        `pubkey: ${json.pubkey_base58}`,
        `mode:   0600 (file permissions restricted to current user)`,
      ].join("\n")
  );
}

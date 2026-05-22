// `mnemonic identity {import,export,status}` — round-trip the keypair between
// the webapp's "Send to CLI" bootstrap flow (Decision 7) and a local file,
// plus local drift detection.
//
// `import --ticket <uuid>`: fetch GET ${baseUrl}/api/cli-bootstrap/redeem/:ticket
//   (no Bearer — the UUID itself is the capability, single-use server-side),
//   parse JSON {secret, pubkey_base58}, save via saveIdentity. Refuses to
//   overwrite an existing identity unless --force.
// `import --file <path>`: read JSON from disk, validate shape, save.
// `export --file <path>`: write current ~/.mnemonic/identity.json to <path>
//   with mode 0600 (or icacls-restricted ACL on Windows). NO clipboard flag.
// `status`: compare local identity (KeyStore/file) vs cached JWT; no network.

import { readFileSync, writeFileSync } from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import type { KeypairJson } from "@mnemonik-xyz/sdk";

import {
  identityExists,
  identityPath,
  loadIdentityJson,
  restrictFileMode,
  saveIdentityJson,
  tokenPath,
} from "../config.js";
import { AuthError, fromSdkError, ServerError, UserError } from "../errors.js";
import {
  colors,
  colorEnabled,
  format,
  hint,
  type OutputOptions,
} from "../output.js";
import { FileKeyStore } from "../identity/keystore-file.js";
import { OsKeyStore } from "../identity/keystore-os.js";
import type { KeyStore } from "../identity/keystore.js";

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

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
      "identity payload: `secret` must be a 64-element number array",
    );
  }
  if (typeof o.pubkey_base58 !== "string" || o.pubkey_base58.length === 0) {
    throw new UserError(
      "identity payload: `pubkey_base58` must be a non-empty string",
    );
  }
  return { secret: o.secret as number[], pubkey_base58: o.pubkey_base58 };
}

export async function runIdentityImport(
  opts: IdentityImportOptions,
): Promise<void> {
  if (!opts.ticket && !opts.file) {
    throw new UserError(
      "identity import: pass either --ticket <uuid> or --file <path>",
    );
  }
  if (opts.ticket && opts.file) {
    throw new UserError(
      "identity import: --ticket and --file are mutually exclusive",
    );
  }

  // Refuse to overwrite an existing identity unless --force.
  if (identityExists() && !opts.force) {
    throw new UserError(
      `identity already exists at ${identityPath()}; pass --force to overwrite ` +
        `(this will replace your keypair — use \`mnemonic identity export\` first)`,
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
      ].join("\n"),
  );
}

async function fetchTicket(
  ticket: string,
  opts: IdentityImportOptions,
): Promise<KeypairJson> {
  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const url = `${baseUrl.replace(
    /\/$/,
    "",
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
      e,
    );
  }

  if (res.status === 410) {
    throw new UserError(
      `ticket ${ticket} has already been redeemed (server returned 410 Gone)`,
    );
  }
  if (res.status === 404) {
    throw new UserError(
      `ticket ${ticket} not found or expired (server returned 404)`,
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
      `ticket redemption failed (HTTP ${res.status}): ${body.slice(0, 500)}`,
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
      e,
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
      e,
    );
  }
  return validateKeypairJson(parsed);
}

export async function runIdentityExport(
  opts: IdentityExportOptions,
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
      e,
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
      ].join("\n"),
  );
}

// ---------------------------------------------------------------------------
// identity status — local drift detector (no network calls)
// ---------------------------------------------------------------------------

export type IdentityStatus =
  | "synced"
  | "diverged"
  | "webapp-unknown"
  | "no-identity"
  | "malformed";

export interface StatusDeps {
  /** OS keychain backend, or null if unavailable. */
  os: KeyStore | null;
  /** File-backed store. */
  file: KeyStore;
  /** Absolute path to identity.json (for stub-shape detection). */
  identityFilePath: string;
  /** Absolute path to token.json. */
  tokenFilePath: string;
}

/** Human-readable OS keychain label per platform. */
export function platformName(): string {
  switch (process.platform) {
    case "darwin":
      return "macOS Keychain";
    case "linux":
      return "Secret Service";
    case "win32":
      return "Windows Credential Manager";
    default:
      return "unavailable";
  }
}

/**
 * Decode the `sub` claim from a JWT string without verifying the signature.
 * Returns null (and sets malformed=true output) when the token is unparseable.
 */
export function decodeJwtSub(jwt: string): {
  sub: string | null;
  malformed: boolean;
} {
  try {
    const parts = jwt.split(".");
    if (parts.length < 3) return { sub: null, malformed: true };
    const payload = parts[1];
    if (!payload) return { sub: null, malformed: true };
    const json = Buffer.from(payload, "base64url").toString("utf8");
    const parsed = JSON.parse(json) as unknown;
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      !("sub" in parsed) ||
      typeof (parsed as Record<string, unknown>)["sub"] !== "string"
    ) {
      return { sub: null, malformed: true };
    }
    return {
      sub: (parsed as Record<string, unknown>)["sub"] as string,
      malformed: false,
    };
  } catch {
    return { sub: null, malformed: true };
  }
}

/**
 * Read token.json and extract a JWT string.
 *
 * The file shape is `{ jwt: string, expires_at: string, sub: string }`.
 * Returns null when the file is absent.
 */
async function readTokenJwt(filePath: string): Promise<{ jwt: string } | null> {
  let raw: string;
  try {
    raw = await fs.readFile(filePath, "utf8");
  } catch (err) {
    if (
      typeof err === "object" &&
      err !== null &&
      "code" in err &&
      (err as NodeJS.ErrnoException).code === "ENOENT"
    ) {
      return null;
    }
    return null; // treat unreadable token as absent
  }
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      "jwt" in parsed &&
      typeof (parsed as Record<string, unknown>)["jwt"] === "string"
    ) {
      return { jwt: (parsed as Record<string, unknown>)["jwt"] as string };
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Read identity.json raw to detect stub shape (has `keychain_ref`, no `secret`).
 * Returns `{ pubkey_base58, isStub }` or null when the file is absent/unreadable.
 */
async function readIdentityFileMeta(
  filePath: string,
): Promise<{ pubkey_base58: string; isStub: boolean } | null> {
  let raw: string;
  try {
    raw = await fs.readFile(filePath, "utf8");
  } catch {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (typeof parsed !== "object" || parsed === null) return null;
    const p = parsed as Record<string, unknown>;
    if (typeof p["pubkey_base58"] !== "string") return null;
    const isStub = "keychain_ref" in p && !("secret" in p);
    return { pubkey_base58: p["pubkey_base58"] as string, isStub };
  } catch {
    return null;
  }
}

export function computeStatus(
  localPubkey: string | null,
  jwtSub: string | null,
  malformed: boolean,
): IdentityStatus {
  if (localPubkey === null) return "no-identity";
  if (malformed) return "malformed";
  if (jwtSub === null) return "webapp-unknown";
  if (localPubkey === jwtSub) return "synced";
  return "diverged";
}

export function exitCodeFor(status: IdentityStatus): number {
  switch (status) {
    case "synced":
    case "webapp-unknown":
      return 0;
    case "no-identity":
      return 1;
    case "diverged":
    case "malformed":
      return 3;
  }
}

const SUGGESTED_ACTIONS = [
  "mnemonic identity pull-from-webapp",
  "mnemonic identity push-to-webapp",
  "mnemonic login",
];

function buildStatusResult(
  localPubkey: string | null,
  jwtSub: string | null,
  storage: string,
  status: IdentityStatus,
): Record<string, unknown> {
  const base: Record<string, unknown> = {
    local: localPubkey,
    jwt_sub: jwtSub,
    storage,
    status,
  };
  if (status === "diverged" || status === "malformed") {
    base["suggested_actions"] = SUGGESTED_ACTIONS;
  }
  return base;
}

function printHumanStatus(
  localPubkey: string | null,
  jwtSub: string | null,
  storage: string,
  status: IdentityStatus,
  noColor: boolean,
): void {
  const color = !noColor && Boolean(process.stdout.isTTY);

  const abbrev = (s: string | null): string => {
    if (!s) return "(none)";
    if (s.length <= 10) return s;
    return `${s.slice(0, 4)}...${s.slice(-3)}`;
  };

  const localDisplay = localPubkey
    ? `${abbrev(localPubkey)}  (did:sol:${localPubkey})`
    : "(no local identity)";

  const jwtDisplay = jwtSub ?? "(no token.json)";

  let statusDisplay: string;
  if (status === "synced") {
    statusDisplay = colors.green("synced", color);
  } else if (status === "diverged" || status === "malformed") {
    statusDisplay = colors.red(status.toUpperCase(), color);
  } else {
    statusDisplay = status;
  }

  const lines = [
    `local identity:    ${localDisplay}`,
    `storage:           ${storage}`,
    `cached JWT.sub:    ${jwtDisplay}`,
    `status:            ${statusDisplay}`,
  ];

  process.stdout.write(lines.join("\n") + "\n");

  if (status === "diverged" || status === "malformed") {
    process.stdout.write(
      [
        "Suggested actions:",
        `  mnemonic identity pull-from-webapp   # adopt the webapp identity`,
        `  mnemonic identity push-to-webapp     # push CLI identity to webapp`,
        `  mnemonic login                       # re-authenticate`,
      ].join("\n") + "\n",
    );
  }
}

/** Production default deps wired to OS keychain + real paths. */
export function defaultStatusDeps(): StatusDeps {
  const homeDir = os.homedir();
  const mnemonicDir = path.join(homeDir, ".mnemonic");
  const identityFilePath = path.join(mnemonicDir, "identity.json");
  return {
    os: new OsKeyStore(),
    file: new FileKeyStore(identityFilePath),
    identityFilePath,
    tokenFilePath: tokenPath(),
  };
}

/**
 * Core logic for `mnemonic identity status`, injectable for tests.
 *
 * Algorithm:
 *   1. Try OS keychain (if available) — preferred source.
 *   2. Fall back to identity.json (legacy full shape or stub shape).
 *   3. Read token.json, decode JWT.sub.
 *   4. Compare and report.
 */
export async function statusWithDeps(
  opts: { json?: boolean; noColor?: boolean },
  deps: StatusDeps,
): Promise<number> {
  // --- Resolve local pubkey ---
  let localPubkey: string | null = null;
  let storage = "none";

  const osAvail = deps.os !== null && (await deps.os.available());
  if (osAvail && deps.os !== null) {
    const osEntry = await deps.os.get().catch(() => null);
    if (osEntry !== null) {
      localPubkey = osEntry.pubkey_base58;
      storage = `OS keychain (${platformName()})`;
    }
  }

  if (localPubkey === null) {
    // Try file store for legacy shape (secret present).
    const fileEntry = await deps.file.get().catch(() => null);
    if (fileEntry !== null) {
      localPubkey = fileEntry.pubkey_base58;
      storage = "file (legacy)";
    } else {
      // Check raw identity.json for stub shape (keychain_ref, no secret).
      const fileMeta = await readIdentityFileMeta(deps.identityFilePath);
      if (fileMeta !== null) {
        localPubkey = fileMeta.pubkey_base58;
        storage = fileMeta.isStub
          ? `OS keychain (stub-referenced; not yet pulled)`
          : "file";
      }
    }
  }

  // --- Resolve JWT sub ---
  let jwtSub: string | null = null;
  let malformed = false;

  const tokenData = await readTokenJwt(deps.tokenFilePath);
  if (tokenData !== null) {
    const decoded = decodeJwtSub(tokenData.jwt);
    jwtSub = decoded.sub;
    malformed = decoded.malformed;
  }

  const status = computeStatus(localPubkey, jwtSub, malformed);

  if (opts.json) {
    const result = buildStatusResult(localPubkey, jwtSub, storage, status);
    process.stdout.write(JSON.stringify(result) + "\n");
  } else {
    printHumanStatus(
      localPubkey,
      jwtSub,
      storage,
      status,
      opts.noColor ?? false,
    );
  }

  return exitCodeFor(status);
}

/** Public entry point used by bin/mnemonic.ts. */
export async function statusCommand(opts: {
  json?: boolean;
  noColor?: boolean;
}): Promise<number> {
  return statusWithDeps(opts, defaultStatusDeps());
}

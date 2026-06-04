// `mnemonic identity {import,export,status,pull-from-webapp,push-to-webapp}`
//
// `pull-from-webapp <short_code>`: redeem a CLI-issued ticket from the server.
//   The short_code (XXXX-XXXX) was produced by a previous `push-to-webapp` run.
//   The server re-wraps the 64-byte Ed25519 secret in a NaCl sealed box
//   (XSalsa20-Poly1305, wire format: nonce[24] || ciphertext) targeted at an
//   ephemeral x25519 keypair generated here. Bit-compatible with tweetnacl's
//   nacl.box.open() and the Rust server's crypto_box::SalsaBox.
//
//   Deviation (T12 vs T14): The server's GET /api/cli-bootstrap/redeem/{ticket}
//   only accepts UUID path parameters; it does not expose a short_code POST
//   route. The webapp's Install.tsx posts {short_code} to
//   /api/cli-bootstrap/redeem (POST) which does not exist in T12's router.
//   pull-from-webapp therefore sends POST /api/cli-bootstrap/redeem with
//   {short_code, redeemer_eph_pub} — matching what the webapp sends — and
//   expects T12/T14 alignment to add that POST route in a future iteration.
//   Tests mock the endpoint shape. Wave-5 manual smoke will confirm E2E.
//
// `push-to-webapp`: issue a server ticket from CLI identity, print short_code
//   and install URL. The webapp redeems it via the `?pull=<short_code>` URL.
//
// `import --ticket <uuid>`: fetch GET ${baseUrl}/api/cli-bootstrap/redeem/:ticket
//   (no Bearer — the UUID itself is the capability, single-use server-side),
//   parse JSON {secret, pubkey_base58}, save via saveIdentity. Refuses to
//   overwrite an existing identity unless --force.
// `import --file <path>`: read JSON from disk, validate shape, save.
// `export --file <path>`: write current ~/.mnemonic/identity.json to <path>
//   with mode 0600 (or icacls-restricted ACL on Windows). NO clipboard flag.
// `status`: compare local identity (KeyStore/file) vs cached JWT; no network.

import { createInterface } from "node:readline";
import { readFileSync, writeFileSync } from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import nacl from "tweetnacl";
import bs58 from "bs58";

import { IdentityRequiresKeystore } from "@mnemonik-xyz/sdk";
import type { KeypairJson } from "@mnemonik-xyz/sdk";

import {
  identityExists,
  identityPath,
  loadIdentityJson,
  resolveKeypairFromKeystore,
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
  // identity.json may be a stub (keychain-backed). Export must produce the
  // full legacy shape `{secret, pubkey_base58}` so the backup is portable —
  // resolve via the keychain when the file is a stub.
  let json: KeypairJson;
  try {
    json = loadIdentityJson();
  } catch (e) {
    if (e instanceof IdentityRequiresKeystore) {
      const kp = await resolveKeypairFromKeystore(e.pubkey_base58);
      json = kp.toJSON();
    } else {
      throw e;
    }
  }
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

// ---------------------------------------------------------------------------
// pull-from-webapp — redeem a short_code issued by `push-to-webapp`
// ---------------------------------------------------------------------------

/**
 * Dependency-injection seam for pullFromWebapp — allows unit tests to provide
 * controlled fetch, keystore, and I/O without touching the filesystem or
 * network.
 */
export interface PullDeps {
  /** OS keychain backend, or null if unavailable. */
  os: KeyStore | null;
  /** File-backed store. */
  file: KeyStore;
  /** Absolute path to identity.json stub. */
  identityFilePath: string;
  /** Fetch implementation (injectable for tests). */
  fetch: typeof globalThis.fetch;
  /** Server base URL (without trailing slash). */
  serverUrl: string;
  /** Atomic write for the stub file (injectable for tests). */
  writeStub: (identityFilePath: string, pubkey_base58: string) => Promise<void>;
  /** Stdout write (injectable to capture in tests). */
  stdoutWrite: (s: string) => void;
}

async function writeStubAtomicPull(
  targetPath: string,
  pubkey_base58: string,
): Promise<void> {
  const content = JSON.stringify({
    pubkey_base58,
    did_sol: `did:sol:${pubkey_base58}`,
    keychain_ref: "xyz.mnemonik.identity/default",
    created_at: new Date().toISOString(),
  });
  const tmpPath = targetPath + ".tmp";
  await fs.mkdir(path.dirname(targetPath), { recursive: true, mode: 0o700 });
  await fs.writeFile(tmpPath, content, { encoding: "utf8", mode: 0o600 });
  await fs.rename(tmpPath, targetPath);
}

/**
 * Core logic for `mnemonic identity pull-from-webapp`, injectable for tests.
 *
 * Calls POST /api/cli-bootstrap/redeem with {short_code, redeemer_eph_pub},
 * decrypts the re-wrapped secret via nacl.box.open, stores via the best
 * available KeyStore, and rewrites identity.json as a stub.
 *
 * Exit codes:
 *   0  success
 *   3  integrity error (pubkey mismatch or decryption failure)
 *   4  ticket error (not found, expired, bad short_code)
 *   2  server/network error
 */
export async function pullFromWebappWithDeps(
  shortCode: string,
  deps: PullDeps,
): Promise<number> {
  // Generate ephemeral x25519 keypair for the NaCl box.
  const eph = nacl.box.keyPair();
  const ephPubB64 = Buffer.from(eph.publicKey).toString("base64");

  // POST /api/cli-bootstrap/redeem with short_code + redeemer_eph_pub.
  const url = `${deps.serverUrl.replace(/\/$/, "")}/api/cli-bootstrap/redeem`;
  let res: Response;
  try {
    res = await deps.fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        short_code: shortCode,
        redeemer_eph_pub: ephPubB64,
      }),
    });
  } catch (e) {
    process.stderr.write(
      `mnemonic: network error reaching ${url}: ${
        e instanceof Error ? e.message : String(e)
      }\n`,
    );
    return 2;
  }

  if (res.status === 404) {
    process.stderr.write(
      `mnemonic: ticket not found or already consumed (HTTP 404)\n`,
    );
    return 4;
  }
  if (res.status === 410) {
    process.stderr.write(
      `mnemonic: ticket expired or already redeemed (HTTP 410)\n`,
    );
    return 4;
  }
  if (!res.ok) {
    let body = "";
    try {
      body = await res.text();
    } catch {
      // ignore
    }
    process.stderr.write(
      `mnemonic: server error (HTTP ${res.status}): ${body.slice(0, 300)}\n`,
    );
    return 2;
  }

  let parsed: unknown;
  try {
    parsed = await res.json();
  } catch {
    process.stderr.write(`mnemonic: server returned non-JSON response\n`);
    return 2;
  }

  if (
    typeof parsed !== "object" ||
    parsed === null ||
    typeof (parsed as Record<string, unknown>)["wrapped_secret"] !== "string" ||
    typeof (parsed as Record<string, unknown>)["eph_pub"] !== "string" ||
    typeof (parsed as Record<string, unknown>)["issuer_pubkey_base58"] !==
      "string"
  ) {
    process.stderr.write(`mnemonic: server response missing required fields\n`);
    return 2;
  }

  const body = parsed as {
    wrapped_secret: string;
    eph_pub: string;
    issuer_pubkey_base58: string;
  };

  // Decode and decrypt the sealed box.
  let wrapped: Uint8Array;
  let serverEphPub: Uint8Array;
  try {
    wrapped = Uint8Array.from(Buffer.from(body.wrapped_secret, "base64"));
    serverEphPub = Uint8Array.from(Buffer.from(body.eph_pub, "base64"));
  } catch {
    process.stderr.write(
      `mnemonic: server returned malformed base64 in wrapped_secret or eph_pub\n`,
    );
    return 4;
  }

  if (wrapped.length < nacl.box.nonceLength) {
    process.stderr.write(
      `mnemonic: wrapped_secret too short to contain a nonce\n`,
    );
    return 4;
  }

  const nonce = wrapped.slice(0, nacl.box.nonceLength);
  const ciphertext = wrapped.slice(nacl.box.nonceLength);

  const plaintext = nacl.box.open(
    ciphertext,
    nonce,
    serverEphPub,
    eph.secretKey,
  );
  if (plaintext === null) {
    process.stderr.write(
      `mnemonic: decryption failed — authentication tag mismatch\n`,
    );
    return 4;
  }

  if (plaintext.length !== 64) {
    process.stderr.write(
      `mnemonic: decrypted secret has wrong length (expected 64, got ${plaintext.length})\n`,
    );
    return 4;
  }

  // Verify pubkey: last 32 bytes of the Ed25519 secret are the public key.
  const derivedPubkey = bs58.encode(plaintext.slice(32));
  if (derivedPubkey !== body.issuer_pubkey_base58) {
    process.stderr.write(
      `mnemonic: integrity error — decrypted pubkey does not match server-reported issuer\n`,
    );
    return 3;
  }

  const entry = {
    secret: Array.from(plaintext),
    pubkey_base58: body.issuer_pubkey_base58,
  };

  // Store via best available backend.
  const osAvail = deps.os !== null && (await deps.os.available());
  if (osAvail && deps.os !== null) {
    await deps.os.set(entry);
    await deps.writeStub(deps.identityFilePath, entry.pubkey_base58);
  } else {
    await deps.file.set(entry);
  }

  process.stderr.write(
    `mnemonic: pulled identity from webapp; pubkey did:sol:${entry.pubkey_base58}\n`,
  );
  return 0;
}

/** Production default deps for pullFromWebapp. */
export function defaultPullDeps(): PullDeps {
  const homeDir = os.homedir();
  const mnemonicDir = path.join(homeDir, ".mnemonic");
  const identityFilePath = path.join(mnemonicDir, "identity.json");
  return {
    os: new OsKeyStore(),
    file: new FileKeyStore(identityFilePath),
    identityFilePath,
    fetch: globalThis.fetch,
    serverUrl:
      process.env.MNEMONIC_SERVER_URL ??
      process.env.MNEMONIC_BASE_URL ??
      "https://mcp.mnemonik.xyz",
    writeStub: writeStubAtomicPull,
    stdoutWrite: (s) => process.stdout.write(s),
  };
}

/**
 * Public entry point for `mnemonic identity pull-from-webapp`.
 * Accepts the short_code as a positional arg, or reads it from stdin
 * when argv is `-` or `--stdin` (avoids shell history leaking the code).
 */
export async function pullFromWebappCommand(
  shortCode: string,
  opts: { stdin?: boolean },
): Promise<number> {
  let code = shortCode;

  if (opts.stdin || shortCode === "-") {
    // Read first line from stdin without echoing.
    const rl = createInterface({ input: process.stdin, terminal: false });
    code = await new Promise<string>((resolve) => {
      rl.once("line", (line) => {
        rl.close();
        resolve(line.trim());
      });
      rl.once("close", () => resolve(""));
    });
    if (!code) {
      process.stderr.write(`mnemonic: no short code received from stdin\n`);
      return 1;
    }
  }

  return pullFromWebappWithDeps(code, defaultPullDeps());
}

// ---------------------------------------------------------------------------
// push-to-webapp — issue a ticket from CLI identity, print QR + short code
// ---------------------------------------------------------------------------

/**
 * Dependency-injection seam for pushToWebapp.
 */
export interface PushDeps {
  /** OS keychain backend, or null if unavailable. */
  os: KeyStore | null;
  /** File-backed store. */
  file: KeyStore;
  /** Fetch implementation (injectable for tests). */
  fetch: typeof globalThis.fetch;
  /** Server base URL (without trailing slash). */
  serverUrl: string;
  /** Stdout write (injectable to capture in tests). */
  stdoutWrite: (s: string) => void;
  /** Whether to skip QR printing (e.g. non-TTY or --code-only). */
  skipQr?: boolean;
}

/**
 * Core logic for `mnemonic identity push-to-webapp`, injectable for tests.
 *
 * Reads local secret, fetches server's static x25519 pub, wraps secret in a
 * NaCl box, POSTs to /api/cli-bootstrap/issue-from-cli, prints short_code
 * and install URL.
 *
 * Exit codes:
 *   0  success
 *   1  no local identity
 *   2  server/network error
 */
export async function pushToWebappWithDeps(deps: PushDeps): Promise<number> {
  // Resolve local secret from keystore.
  let entry: { secret: number[]; pubkey_base58: string } | null = null;
  const osAvail = deps.os !== null && (await deps.os.available());
  if (osAvail && deps.os !== null) {
    entry = await deps.os.get().catch(() => null);
  }
  if (entry === null) {
    entry = await deps.file.get().catch(() => null);
  }
  if (entry === null) {
    process.stderr.write(
      `mnemonic: no local identity found; run \`mnemonic init\` or \`mnemonic identity pull-from-webapp\`\n`,
    );
    return 1;
  }

  const serverUrlBase = deps.serverUrl.replace(/\/$/, "");

  // Fetch server's static x25519 pub.
  let serverPubBytes: Uint8Array;
  try {
    const pubRes = await deps.fetch(
      `${serverUrlBase}/api/cli-bootstrap/server-pub`,
    );
    if (!pubRes.ok) {
      process.stderr.write(
        `mnemonic: failed to fetch server public key (HTTP ${pubRes.status})\n`,
      );
      return 2;
    }
    const pubBody = (await pubRes.json()) as { server_x25519_pub?: string };
    if (typeof pubBody.server_x25519_pub !== "string") {
      process.stderr.write(
        `mnemonic: server returned unexpected shape from /api/cli-bootstrap/server-pub\n`,
      );
      return 2;
    }
    serverPubBytes = Uint8Array.from(
      Buffer.from(pubBody.server_x25519_pub, "base64"),
    );
  } catch (e) {
    process.stderr.write(
      `mnemonic: network error fetching server pub key: ${
        e instanceof Error ? e.message : String(e)
      }\n`,
    );
    return 2;
  }

  // Generate ephemeral keypair and wrap the 64-byte secret.
  const eph = nacl.box.keyPair();
  const secretBytes = new Uint8Array(entry.secret);
  const nonce = nacl.randomBytes(nacl.box.nonceLength);
  const ciphertext = nacl.box(
    secretBytes,
    nonce,
    serverPubBytes,
    eph.secretKey,
  );

  const transport = Buffer.concat([
    Buffer.from(nonce),
    Buffer.from(ciphertext),
  ]);
  const wrappedB64 = transport.toString("base64");
  const ephPubB64 = Buffer.from(eph.publicKey).toString("base64");

  // POST issue-from-cli.
  let issueBody: { ticket_id: string; short_code: string; expires_at: string };
  try {
    const issueRes = await deps.fetch(
      `${serverUrlBase}/api/cli-bootstrap/issue-from-cli`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          wrapped_secret: wrappedB64,
          eph_pub: ephPubB64,
          issuer_pubkey_base58: entry.pubkey_base58,
        }),
      },
    );
    if (!issueRes.ok) {
      let errBody = "";
      try {
        errBody = await issueRes.text();
      } catch {
        // ignore
      }
      process.stderr.write(
        `mnemonic: server rejected ticket issuance (HTTP ${issueRes.status}): ${errBody.slice(0, 300)}\n`,
      );
      return 2;
    }
    issueBody = (await issueRes.json()) as {
      ticket_id: string;
      short_code: string;
      expires_at: string;
    };
  } catch (e) {
    process.stderr.write(
      `mnemonic: network error issuing ticket: ${
        e instanceof Error ? e.message : String(e)
      }\n`,
    );
    return 2;
  }

  const { short_code, expires_at } = issueBody;
  const installUrl = `https://mnemonik.xyz/install?pull=${encodeURIComponent(short_code)}`;

  if (!deps.skipQr) {
    deps.stdoutWrite(`\n`);
    deps.stdoutWrite(
      `Open this URL on your logged-in webapp browser:\n  ${installUrl}\n`,
    );
    deps.stdoutWrite(
      `\nOr enter this short code on the webapp: ${short_code}\n`,
    );
    deps.stdoutWrite(`Expires: ${expires_at}\n`);

    // Print QR if stdout is a TTY.
    if (process.stdout.isTTY) {
      try {
        const qrcode = await import("qrcode-terminal");
        await new Promise<void>((resolve) => {
          qrcode.default.generate(
            installUrl,
            { small: true },
            (output: string) => {
              deps.stdoutWrite(output + "\n");
              resolve();
            },
          );
        });
      } catch {
        // QR generation is best-effort — if it fails, the URL is enough.
      }
    }
  } else {
    // --code-only: just print the short code and URL.
    deps.stdoutWrite(`${installUrl}\n`);
    deps.stdoutWrite(`Short code: ${short_code}\n`);
    deps.stdoutWrite(`Expires: ${expires_at}\n`);
  }

  return 0;
}

/** Production default deps for pushToWebapp. */
export function defaultPushDeps(opts: {
  codeOnly?: boolean;
  qrOnly?: boolean;
  serverUrl?: string;
}): PushDeps {
  const homeDir = os.homedir();
  const mnemonicDir = path.join(homeDir, ".mnemonic");
  const identityFilePath = path.join(mnemonicDir, "identity.json");
  return {
    os: new OsKeyStore(),
    file: new FileKeyStore(identityFilePath),
    fetch: globalThis.fetch,
    serverUrl:
      opts.serverUrl ??
      process.env.MNEMONIC_SERVER_URL ??
      process.env.MNEMONIC_BASE_URL ??
      "https://mcp.mnemonik.xyz",
    stdoutWrite: (s) => process.stdout.write(s),
    skipQr: opts.codeOnly ?? false,
  };
}

/**
 * Public entry point for `mnemonic identity push-to-webapp`.
 */
export async function pushToWebappCommand(opts: {
  qrOnly?: boolean;
  codeOnly?: boolean;
  serverUrl?: string;
}): Promise<number> {
  return pushToWebappWithDeps(defaultPushDeps(opts));
}

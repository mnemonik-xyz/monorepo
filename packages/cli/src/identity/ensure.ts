/**
 * identity/ensure.ts — invisible bootstrap for the CLI identity.
 *
 * Mirrors the Rust `core/src/identity/ensure.rs` five-path algorithm:
 *   1. ENOENT  → CREATE: generate keypair, store in OS keychain (if available)
 *      or fall back to file.
 *   2. LEGACY  → MIGRATE: file has `secret` key → move to OS keychain,
 *      rewrite file as stub.
 *   3. STUB    → VERIFY: file has `keychain_ref` → confirm OS keychain entry
 *      exists.
 *   (implicitly: no-op when already in correct stub state)
 *
 * SECURITY: KeystoreEntry.secret bytes MUST NEVER appear in logs, error
 * messages, or Error.cause.  All error paths must redact the secret.
 */

import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { Keypair } from "@mnemonik-xyz/sdk";

import { OsKeyStore } from "./keystore-os.js";
import { FileKeyStore } from "./keystore-file.js";
import type { KeyStore } from "./keystore.js";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

export interface KeyStores {
  /** OS keychain backend — null signals "unavailable on this platform". */
  os: KeyStore | null;
  /** File-backed fallback store. */
  file: KeyStore;
  /** Absolute path to identity.json (the stub or legacy file). */
  identityPath: string;
  /** Absolute path to README.txt (written once, existence errors silenced). */
  readmePath: string;
  /**
   * Optional override for the atomic stub-file write step.  Used in tests to
   * simulate file-write failure and verify OS keychain rollback.  Production
   * code never sets this field.
   */
  _writeStub?: (identityPath: string, pubkey_base58: string) => Promise<void>;
}

export interface EnsureResult {
  pubkey_base58: string;
  storage: "os-keychain" | "file";
  /** true if a new keypair was generated during this call. */
  created: boolean;
  /** true if a legacy file was migrated to the OS keychain during this call. */
  migrated: boolean;
}

// ---------------------------------------------------------------------------
// README content (verbatim per tech-spec §Data Models)
// ---------------------------------------------------------------------------

const README_CONTENT = `This directory holds your Mnemonic identity and session state.

  identity.json — your Ed25519 public key (private key is in the OS keychain).
  token.json    — your current JWT for the Mnemonic webapp (regenerated on \`mnemonic login\`).

If you back up this folder, your private key is NOT included — back up via
\`mnemonic identity push-to-webapp\` or your OS keychain export.

Docs: https://mnemonik.xyz/docs/cli
`;

// ---------------------------------------------------------------------------
// Stub-file content builder
// ---------------------------------------------------------------------------

const KEYCHAIN_REF = "xyz.mnemonik.identity/default";

function buildStubContent(pubkey_base58: string): string {
  return JSON.stringify({
    pubkey_base58,
    did_sol: `did:sol:${pubkey_base58}`,
    keychain_ref: KEYCHAIN_REF,
    created_at: new Date().toISOString(),
  });
}

// ---------------------------------------------------------------------------
// shouldSkipEnsure — centralized skip-list for the CLI entrypoint.
//
// Exported here (not in bin/mnemonic.ts) so tests can import it without
// loading Commander or any command modules.
// ---------------------------------------------------------------------------

const HELP_FLAGS = new Set(["--help", "-h", "--version", "-V"]);

/**
 * Return true when `ensure()` should be skipped.
 *
 * Skip conditions:
 *   - Any help/version flag in argv (no I/O before printing usage).
 *   - `identity status` subcommand (must run even with no identity present).
 *   - `init --force` (init manages its own identity lifecycle).
 */
export function shouldSkipEnsure(argv: string[]): boolean {
  if (argv.some((a) => HELP_FLAGS.has(a))) return true;
  if (argv[2] === "identity" && argv[3] === "status") return true;
  if (argv[2] === "init" && argv.includes("--force")) return true;
  return false;
}

// ---------------------------------------------------------------------------
// Quiet-check helper
// ---------------------------------------------------------------------------

function shouldPrintStatus(): boolean {
  if (process.env.MNEMONIC_QUIET === "1") return false;
  if (process.argv.includes("--quiet")) return false;
  return true;
}

function log(line: string): void {
  if (shouldPrintStatus()) {
    process.stderr.write(line + "\n");
  }
}

// ---------------------------------------------------------------------------
// Atomic stub-file write  (tmp → rename, mode 0o600)
// ---------------------------------------------------------------------------

async function writeStubAtomic(
  targetPath: string,
  pubkey_base58: string,
): Promise<void> {
  const content = buildStubContent(pubkey_base58);
  const tmpPath = targetPath + ".tmp";
  await fs.writeFile(tmpPath, content, { encoding: "utf8", mode: 0o600 });
  await fs.rename(tmpPath, targetPath);
}

// ---------------------------------------------------------------------------
// README — written once; silently ignored if the file already exists.
// ---------------------------------------------------------------------------

async function writeReadmeOnce(readmePath: string): Promise<void> {
  try {
    await fs.writeFile(readmePath, README_CONTENT, { flag: "wx" });
  } catch (err) {
    // EEXIST means it was already written — that is the normal path after the
    // first run.  Any other error is also silently swallowed (non-critical).
    void err;
  }
}

// ---------------------------------------------------------------------------
// Directory helper
// ---------------------------------------------------------------------------

async function ensureParentDir(filePath: string): Promise<void> {
  const dir = path.dirname(filePath);
  await fs.mkdir(dir, { recursive: true, mode: 0o700 });
}

// ---------------------------------------------------------------------------
// Core algorithm
// ---------------------------------------------------------------------------

export async function ensure(): Promise<EnsureResult> {
  return ensureWithStores(defaultStores());
}

export async function ensureWithStores(
  stores: KeyStores,
): Promise<EnsureResult> {
  // 1. Ensure the directory exists before any file reads/writes.
  await ensureParentDir(stores.identityPath);
  await writeReadmeOnce(stores.readmePath);

  // 2. Try to read the identity file.
  let raw: string | null = null;
  try {
    raw = await fs.readFile(stores.identityPath, "utf8");
  } catch (err) {
    if (!isEnoent(err)) throw err;
    // ENOENT → CREATE path (handled below).
  }

  // ---------- CREATE ----------
  if (raw === null) {
    return createIdentity(stores);
  }

  // ---------- PARSE ----------
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(
      "identity.json is not valid JSON; delete it and re-run to create a new identity",
    );
  }

  if (!parsed || typeof parsed !== "object") {
    throw new Error("identity.json has an unexpected format");
  }

  const obj = parsed as Record<string, unknown>;

  // ---------- LEGACY → MIGRATE or FILE-ONLY ----------
  if ("secret" in obj) {
    return handleLegacy(obj, stores);
  }

  // ---------- STUB → VERIFY ----------
  if ("keychain_ref" in obj && typeof obj.pubkey_base58 === "string") {
    return handleStub(obj.pubkey_base58, stores);
  }

  throw new Error(
    "identity.json has an unrecognised shape; expected legacy or stub format",
  );
}

// ---------------------------------------------------------------------------
// CREATE path
// ---------------------------------------------------------------------------

async function createIdentity(stores: KeyStores): Promise<EnsureResult> {
  const kp = await Keypair.generate();
  const json = kp.toJSON();
  const pubkey_base58 = json.pubkey_base58;
  const entry = { secret: json.secret, pubkey_base58 };

  // Try OS keychain first.
  const osAvail = stores.os !== null && (await stores.os.available());

  const doWriteStub = stores._writeStub ?? writeStubAtomic;

  if (osAvail && stores.os !== null) {
    // Write to OS keychain.
    await stores.os.set(entry);

    // Write stub file atomically.
    try {
      await doWriteStub(stores.identityPath, pubkey_base58);
    } catch (err) {
      // File write failed — roll back keychain entry.
      await safeOsRemove(stores.os);
      throw err;
    }

    log(
      `mnemonic: identity created did:sol:${pubkey_base58} stored in OS keychain`,
    );
    return {
      pubkey_base58,
      storage: "os-keychain",
      created: true,
      migrated: false,
    };
  }

  // OS unavailable — store secret in file (legacy-shape for compatibility with
  // existing loadIdentity() which reads { secret, pubkey_base58 }).
  const unavailReason = "OS keychain unavailable";

  await stores.file.set(entry);

  log(
    `mnemonic: identity created did:sol:${pubkey_base58} stored in ~/.mnemonic/identity.json (${unavailReason})`,
  );
  return { pubkey_base58, storage: "file", created: true, migrated: false };
}

// ---------------------------------------------------------------------------
// LEGACY → MIGRATE path
// ---------------------------------------------------------------------------

async function handleLegacy(
  obj: Record<string, unknown>,
  stores: KeyStores,
): Promise<EnsureResult> {
  // Validate basic shape.
  if (!Array.isArray(obj.secret) || typeof obj.pubkey_base58 !== "string") {
    throw new Error(
      "identity.json legacy format is malformed (expected {secret: number[], pubkey_base58: string})",
    );
  }

  const entry = {
    secret: Array.from(obj.secret as ArrayLike<number>),
    pubkey_base58: obj.pubkey_base58,
  };
  const pubkey_base58 = entry.pubkey_base58;

  const osAvail = stores.os !== null && (await stores.os.available());

  const doWriteStub = stores._writeStub ?? writeStubAtomic;

  if (osAvail && stores.os !== null) {
    // Migrate: write to OS keychain, then rewrite file as stub.
    await stores.os.set(entry);

    try {
      await doWriteStub(stores.identityPath, pubkey_base58);
    } catch (err) {
      // File rewrite failed — roll back keychain entry.
      await safeOsRemove(stores.os);
      throw err;
    }

    log("mnemonic: legacy identity migrated to OS keychain");
    return {
      pubkey_base58,
      storage: "os-keychain",
      created: false,
      migrated: true,
    };
  }

  // OS unavailable — keep file as-is (legacy shape stays on disk).
  return { pubkey_base58, storage: "file", created: false, migrated: false };
}

// ---------------------------------------------------------------------------
// STUB → VERIFY path
// ---------------------------------------------------------------------------

async function handleStub(
  pubkey_base58: string,
  stores: KeyStores,
): Promise<EnsureResult> {
  if (stores.os === null) {
    throw new Error(
      `identity.json is a stub referencing the OS keychain, but OS keychain is unavailable on this platform. ` +
        `Run \`mnemonic identity pull-from-webapp\` to re-import your key.`,
    );
  }

  const entry = await stores.os.get();
  if (entry === null) {
    throw new Error(
      `identity.json references the OS keychain but the keychain entry is missing. ` +
        `Run \`mnemonic identity pull-from-webapp\` to re-import your key.`,
    );
  }

  return {
    pubkey_base58,
    storage: "os-keychain",
    created: false,
    migrated: false,
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function safeOsRemove(ks: KeyStore): Promise<void> {
  try {
    await ks.remove();
  } catch {
    // Best-effort rollback — swallow errors.
  }
}

function isEnoent(err: unknown): boolean {
  return (
    typeof err === "object" &&
    err !== null &&
    "code" in err &&
    (err as NodeJS.ErrnoException).code === "ENOENT"
  );
}

// ---------------------------------------------------------------------------
// defaultStores() — production wiring
// ---------------------------------------------------------------------------

export function defaultStores(): KeyStores {
  const home = os.homedir();
  if (!home) {
    throw new Error(
      "Cannot determine home directory; set HOME environment variable",
    );
  }
  const mnemonicDir = path.join(home, ".mnemonic");
  const identityPath = path.join(mnemonicDir, "identity.json");
  const readmePath = path.join(mnemonicDir, "README.txt");

  return {
    os: new OsKeyStore(),
    file: new FileKeyStore(identityPath),
    identityPath,
    readmePath,
  };
}

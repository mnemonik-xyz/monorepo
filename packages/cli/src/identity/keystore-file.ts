/**
 * FileKeyStore — filesystem-backed KeyStore implementation.
 *
 * Reads and writes a JSON file at a caller-supplied path using the canonical
 * shape: {"secret":[...64 bytes...],"pubkey_base58":"..."}.
 *
 * `set` uses an atomic tempfile-then-rename pattern so a crash mid-write never
 * leaves a partially-written file at the target path.  Mode 0o600 is enforced
 * on Unix via writeFile options.  On Windows, fs.chmod is a no-op — this is a
 * documented limitation; the ACL helper from mnemonic-cli (if present) could
 * be called here in a future iteration.
 *
 * SECURITY: `secret` bytes are NEVER logged, included in error messages, or
 * placed in Error.cause in any retrievable form.
 */

import fs from "node:fs/promises";
import path from "node:path";
import { randomBytes } from "node:crypto";
import {
  type KeyStore,
  type KeystoreEntry,
  KeystoreError,
  canonicalEntryJson,
} from "./keystore.js";

export class FileKeyStore implements KeyStore {
  public readonly name = "file" as const;

  constructor(private readonly filePath: string) {}

  /**
   * Read the entry from disk.
   *
   * Returns:
   * - entry  — full shape with `secret` present.
   * - null   — file absent, or stub shape (no `secret` key).
   * Throws:
   * - KeystoreError('io')    — unexpected I/O error.
   * - KeystoreError('serde') — file is not valid JSON.
   */
  async get(): Promise<KeystoreEntry | null> {
    let raw: string;
    try {
      raw = await fs.readFile(this.filePath, "utf8");
    } catch (err) {
      if (isEnoent(err)) return null;
      throw new KeystoreError(
        "io",
        `FileKeyStore.get: cannot read file (path length=${this.filePath.length})`,
        // Omit cause — it contains the fs path which is benign but not the
        // secret, so technically fine; we omit it for uniformity.
      );
    }

    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      throw new KeystoreError(
        "serde",
        "FileKeyStore.get: file is not valid JSON",
      );
    }

    if (typeof parsed !== "object" || parsed === null) {
      throw new KeystoreError(
        "serde",
        "FileKeyStore.get: top-level value is not an object",
      );
    }

    const p = parsed as Record<string, unknown>;

    // Stub shape — secret lives in OS keychain, not in this file.
    if (!("secret" in p)) {
      return null;
    }

    if (!Array.isArray(p["secret"])) {
      throw new KeystoreError(
        "serde",
        "FileKeyStore.get: 'secret' is not an array",
      );
    }
    if (typeof p["pubkey_base58"] !== "string") {
      throw new KeystoreError(
        "serde",
        "FileKeyStore.get: 'pubkey_base58' is not a string",
      );
    }

    return {
      secret: Array.from(p["secret"] as ArrayLike<number>),
      pubkey_base58: p["pubkey_base58"] as string,
    };
  }

  /**
   * Write the entry atomically.
   *
   * Writes to `<path>.tmp.<random>` with mode 0o600, then renames to the
   * final path.  The rename is atomic on POSIX (same filesystem).  On Windows,
   * rename may fail if the target exists in some Node versions — we handle
   * that by catching EPERM and retrying with unlink+rename.
   *
   * NOTE: fs.chmod is a no-op on Windows.  The 0o600 mode in writeFile options
   * is equally a no-op there.  This is a known limitation; Windows ACLs require
   * the win-acl helper (future iteration).
   */
  async set(entry: KeystoreEntry): Promise<void> {
    const json = canonicalEntryJson(entry);
    const suffix = randomBytes(4).toString("hex");
    const tmpPath = this.filePath + ".tmp." + suffix;

    try {
      // Write to temp file with mode 0o600 (no-op on Windows — see above).
      await fs.writeFile(tmpPath, json, { encoding: "utf8", mode: 0o600 });
    } catch (err) {
      throw new KeystoreError(
        "io",
        "FileKeyStore.set: cannot write temporary file",
      );
    }

    try {
      await fs.rename(tmpPath, this.filePath);
    } catch (err) {
      // On Windows, rename over an existing file may throw EPERM.
      // Retry with unlink + rename.
      if (isEperm(err)) {
        try {
          await fs.unlink(this.filePath);
          await fs.rename(tmpPath, this.filePath);
        } catch {
          // Best-effort cleanup of the temp file; ignore secondary errors.
          void fs.unlink(tmpPath).catch(() => undefined);
          throw new KeystoreError(
            "io",
            "FileKeyStore.set: atomic rename failed (Windows EPERM fallback also failed)",
          );
        }
      } else {
        // Clean up the temp file before surfacing the error.
        void fs.unlink(tmpPath).catch(() => undefined);
        throw new KeystoreError("io", "FileKeyStore.set: atomic rename failed");
      }
    }
  }

  /** Delete the file. Idempotent — no error if already absent. */
  async remove(): Promise<void> {
    try {
      await fs.unlink(this.filePath);
    } catch (err) {
      if (isEnoent(err)) return;
      throw new KeystoreError("io", "FileKeyStore.remove: unlink failed");
    }
  }

  /**
   * Return true if the parent directory is writable.
   * Does NOT create the directory — callers (e.g. ensure()) must do that.
   */
  async available(): Promise<boolean> {
    const parent = path.dirname(this.filePath);
    try {
      await fs.access(parent, fs.constants.W_OK);
      return true;
    } catch {
      return false;
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isEnoent(err: unknown): boolean {
  return (
    typeof err === "object" &&
    err !== null &&
    "code" in err &&
    (err as NodeJS.ErrnoException).code === "ENOENT"
  );
}

function isEperm(err: unknown): boolean {
  return (
    typeof err === "object" &&
    err !== null &&
    "code" in err &&
    (err as NodeJS.ErrnoException).code === "EPERM"
  );
}

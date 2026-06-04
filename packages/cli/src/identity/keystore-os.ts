/**
 * OsKeyStore — wraps @napi-rs/keyring to store the Ed25519 keypair in the
 * platform credential store (macOS Keychain, Windows Credential Manager,
 * Linux libsecret/D-Bus).
 *
 * Fixed service/account names so the entry is always addressable:
 *   service  = "xyz.mnemonik.identity"
 *   account  = "default"
 *
 * SECURITY: `secret` bytes are NEVER logged, included in error messages, or
 * placed in Error.cause in any retrievable form.
 */

import { execFileSync } from "node:child_process";

import { Entry } from "@napi-rs/keyring";
import {
  type KeyStore,
  type KeystoreEntry,
  KeystoreError,
  canonicalEntryJson,
} from "./keystore.js";

const SERVICE = "xyz.mnemonik.identity";
const ACCOUNT = "default";

/**
 * macOS-only: write the keychain entry via `security add-generic-password
 * -U -A` so the resulting item's ACL is `kSecACLAuthorizationAny` (allow any
 * application to access without warning). See `set()` doc for why.
 *
 * Args:
 *   -U  Update an existing item if present (idempotent)
 *   -A  Allow any application to access this item without warning
 *   -s  Service name (Decision 1)
 *   -a  Account name (Decision 1)
 *   -w  Password / value (the JSON-encoded `KeystoreEntry`)
 *
 * Argv exposure: the JSON (including the 64-byte secret) appears in `argv`
 * for the duration of the subprocess (~50ms). Documented trade-off; see
 * the Rust mirror in `core/src/identity/keystore_os.rs`.
 */
function setPermissiveMacos(json: string): void {
  try {
    execFileSync(
      "/usr/bin/security",
      [
        "add-generic-password",
        "-U",
        "-A",
        "-s",
        SERVICE,
        "-a",
        ACCOUNT,
        "-w",
        json,
      ],
      { stdio: ["ignore", "ignore", "pipe"] },
    );
  } catch (err) {
    // Do NOT propagate the raw error message — it may contain the json on
    // some shell-error paths. Surface a generic kind only.
    throw new KeystoreError("other", `OsKeyStore.set (macOS shell-out) failed`);
  }
}

/** Classify a thrown error from @napi-rs/keyring into a KeystoreErrorKind. */
function classifyKeychainError(
  err: unknown,
): "platform-unavailable" | "locked" | "other" {
  const msg =
    err instanceof Error
      ? err.message.toLowerCase()
      : String(err).toLowerCase();
  if (
    msg.includes("platform") ||
    msg.includes("no such service") ||
    msg.includes("service") ||
    msg.includes("dbus") ||
    msg.includes("d-bus") ||
    msg.includes("secret service") ||
    msg.includes("not available") ||
    msg.includes("unavailable")
  ) {
    return "platform-unavailable";
  }
  if (
    msg.includes("locked") ||
    msg.includes("denied") ||
    msg.includes("user interaction not allowed")
  ) {
    return "locked";
  }
  return "other";
}

export class OsKeyStore implements KeyStore {
  public readonly name = "os" as const;

  /** Get the stored entry, or null if absent. */
  async get(): Promise<KeystoreEntry | null> {
    let raw: string | null;
    try {
      const entry = new Entry(SERVICE, ACCOUNT);
      raw = entry.getPassword();
    } catch (err) {
      const kind = classifyKeychainError(err);
      // "No matching entry" semantics — treat as absent.
      const msg =
        err instanceof Error
          ? err.message.toLowerCase()
          : String(err).toLowerCase();
      if (msg.includes("no matching") || msg.includes("not found")) {
        return null;
      }
      throw new KeystoreError(
        kind,
        // Do NOT include raw error message text that might contain key material.
        `OsKeyStore.get failed (kind=${kind})`,
        // Omit cause entirely — it may contain the raw JSON with secret bytes.
      );
    }

    if (raw === null) {
      return null;
    }

    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      throw new KeystoreError(
        "serde",
        "OsKeyStore: keychain payload is not valid JSON",
      );
    }

    if (
      typeof parsed !== "object" ||
      parsed === null ||
      !("secret" in parsed) ||
      !("pubkey_base58" in parsed)
    ) {
      throw new KeystoreError(
        "serde",
        "OsKeyStore: keychain payload missing required fields",
      );
    }

    const p = parsed as Record<string, unknown>;
    if (!Array.isArray(p["secret"])) {
      throw new KeystoreError(
        "serde",
        "OsKeyStore: 'secret' field is not an array",
      );
    }
    if (typeof p["pubkey_base58"] !== "string") {
      throw new KeystoreError(
        "serde",
        "OsKeyStore: 'pubkey_base58' field is not a string",
      );
    }

    // Normalise to number[] — the stored bytes may come back as a Buffer/Uint8Array
    // after a round-trip through the keychain on some platforms.
    const secretArr: number[] = Array.from(p["secret"] as ArrayLike<number>);

    return {
      secret: secretArr,
      pubkey_base58: p["pubkey_base58"] as string,
    };
  }

  /** Persist the entry in the OS keychain.
   *
   *  **macOS**: shells out to `security add-generic-password -U -A` so the
   *  resulting item's ACL allows any application to read without prompt
   *  (Decision 3 — "no prompts at bootstrap"). `@napi-rs/keyring`'s
   *  `setPassword` creates entries with a restrictive default ACL that
   *  triggers an "Always Allow" GUI prompt for every new caller (e.g. the
   *  Rust mcp-server vs the Node CLI vs a third-party SDK consumer).
   *
   *  **Other platforms**: use `@napi-rs/keyring` directly — Linux Secret
   *  Service and Windows Credential Manager don't have per-item ACLs that
   *  prompt on cross-binary access.
   */
  async set(entry: KeystoreEntry): Promise<void> {
    const json = canonicalEntryJson(entry);

    if (process.platform === "darwin") {
      setPermissiveMacos(json);
      return;
    }

    try {
      const e = new Entry(SERVICE, ACCOUNT);
      e.setPassword(json);
    } catch (err) {
      const kind = classifyKeychainError(err);
      throw new KeystoreError(
        kind,
        `OsKeyStore.set failed (kind=${kind})`,
        // Omit cause — json contains secret bytes.
      );
    }
  }

  /** Remove the entry from the OS keychain. Idempotent. */
  async remove(): Promise<void> {
    try {
      const e = new Entry(SERVICE, ACCOUNT);
      e.deletePassword();
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message.toLowerCase()
          : String(err).toLowerCase();
      if (msg.includes("no matching") || msg.includes("not found")) {
        return; // already absent — idempotent
      }
      const kind = classifyKeychainError(err);
      throw new KeystoreError(kind, `OsKeyStore.remove failed (kind=${kind})`);
    }
  }

  /**
   * Return true if the OS keychain is available on this platform.
   *
   * Constructs a new Entry and performs a lightweight probe WITHOUT calling
   * getPassword() (which would trigger OS prompts). We rely on the fact that
   * @napi-rs/keyring throws synchronously at construction time on platforms
   * where the backend is completely absent (e.g. headless Linux without D-Bus).
   *
   * If construction succeeds, we assume the backend exists; we cannot know
   * whether it is locked without actually calling getPassword().
   */
  async available(): Promise<boolean> {
    try {
      new Entry(SERVICE, ACCOUNT);
      return true;
    } catch (err) {
      const kind = classifyKeychainError(err);
      if (kind === "platform-unavailable") {
        return false;
      }
      // Unexpected error — re-throw so callers know something went wrong.
      throw new KeystoreError(
        kind,
        `OsKeyStore.available probe failed (kind=${kind})`,
      );
    }
  }
}

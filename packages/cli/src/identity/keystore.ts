/**
 * KeyStore interface and shared types for Ed25519 identity persistence.
 *
 * SECURITY: KeystoreEntry.secret (64 raw bytes) MUST NEVER appear in logs,
 * error messages, or Error.cause. All error paths must redact the secret.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Ed25519 keypair stored in a KeyStore backend. */
export interface KeystoreEntry {
  /** Raw Ed25519 seed+public bytes, 64 bytes total. */
  secret: number[];
  /** Base58-encoded Ed25519 public key. */
  pubkey_base58: string;
}

// ---------------------------------------------------------------------------
// Canonical JSON serialisation (Decision 13)
// ---------------------------------------------------------------------------

/**
 * Serialize a KeystoreEntry to canonical JSON with a guaranteed field order:
 * `secret` first, then `pubkey_base58` — no whitespace, no trailing newline.
 *
 * We build the string manually instead of relying on `JSON.stringify({secret,
 * pubkey_base58})` because the ECMAScript spec does NOT guarantee insertion-
 * order key enumeration for all V8 builds/versions. This byte-for-byte output
 * must match the Rust serde_json output (Decision 13).
 */
export function canonicalEntryJson(e: KeystoreEntry): string {
  return (
    '{"secret":' +
    JSON.stringify(e.secret) +
    ',"pubkey_base58":' +
    JSON.stringify(e.pubkey_base58) +
    "}"
  );
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

export type KeystoreErrorKind =
  | "platform-unavailable" // Linux/headless without D-Bus, etc.
  | "locked" // OS keychain locked; user can retry
  | "not-found" // entry absent
  | "io" // filesystem I/O error
  | "serde" // JSON parse / serialise error
  | "other"; // backend / unknown

export class KeystoreError extends Error {
  public readonly kind: KeystoreErrorKind;
  public readonly cause?: unknown;

  constructor(kind: KeystoreErrorKind, message: string, cause?: unknown) {
    super(message);
    this.name = "KeystoreError";
    this.kind = kind;
    // Do NOT embed `cause` in `message`; callers log kind+message only.
    this.cause = cause;
  }
}

// ---------------------------------------------------------------------------
// Interface
// ---------------------------------------------------------------------------

export interface KeyStore {
  /** Read the stored entry, or null if absent / stub shape. */
  get(): Promise<KeystoreEntry | null>;

  /** Persist the entry. Atomic on File backend; immediate on OS/Memory. */
  set(entry: KeystoreEntry): Promise<void>;

  /** Delete the entry. Idempotent — no error if already absent. */
  remove(): Promise<void>;

  /**
   * Return true if this backend is functional on the current platform.
   * Must NOT trigger OS credential prompts.
   */
  available(): Promise<boolean>;

  /** Human-readable backend identifier. */
  readonly name: "os" | "file" | "memory";
}

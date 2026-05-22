// Keypair helper — wraps WASM `generate_keypair`, `import_keypair_json`,
// `export_keypair_json` behind a small ergonomic class.
//
// The on-the-wire shape is the same as the webapp's
// `localStorage["mnemonic.identity"]`: `{secret: number[64], pubkey_base58: string}`.
// Sharing the shape is what lets the bootstrap-ticket flow round-trip a
// keypair from browser to CLI without re-serialization.

import { IdentityRequiresKeystore, UserError } from "./errors.js";
import { loadWasm } from "./wasm.js";

/** On-the-wire keypair representation, identical to webapp localStorage. */
export interface KeypairJson {
  /** 64-byte Solana keypair format (32 bytes seed + 32 bytes pubkey). */
  secret: number[];
  /** Base58 Ed25519 public key — must match what `secret` derives to. */
  pubkey_base58: string;
}

/**
 * Owning wrapper around an Ed25519 `KeypairJson`.
 *
 * Construction is async-only — `Keypair.generate()` and `Keypair.fromJSON()`
 * load the WASM module lazily. `fromBytes` is intentionally absent; consumers
 * that already have the JSON object call `fromJSON` (which validates).
 */
export class Keypair {
  private readonly _json: KeypairJson;

  /** @internal — use `Keypair.generate()` or `Keypair.fromJSON()`. */
  constructor(json: KeypairJson) {
    this._json = json;
  }

  /** Base58 Ed25519 public key. */
  get pubkey(): string {
    return this._json.pubkey_base58;
  }

  /** Plain-object form — safe to JSON.stringify, mode-0600 to disk, etc. */
  toJSON(): KeypairJson {
    // Defensive copy — callers should not be able to mutate our secret in place.
    return {
      secret: [...this._json.secret],
      pubkey_base58: this._json.pubkey_base58,
    };
  }

  /**
   * Serialize to a backup-safe JSON string via WASM `export_keypair_json`.
   * The export validates that the secret/pubkey pair is internally
   * consistent before emitting the string — a corrupted keypair refuses
   * to export.
   *
   * @returns A JSON string identical to the webapp's "Download keypair"
   *          format.
   * @throws Any WASM-level error if validation fails.
   */
  async toBackupString(): Promise<string> {
    const wasm = await loadWasm();
    return wasm.export_keypair_json(this._json);
  }

  /**
   * Generate a fresh keypair using WASM `generate_keypair` (entropy from
   * `crypto.getRandomValues` via `getrandom`'s `js` feature).
   *
   * @returns A freshly generated, validated `Keypair`.
   * @throws `UserError` if the WASM module returns an unexpected shape.
   */
  static async generate(): Promise<Keypair> {
    const wasm = await loadWasm();
    const raw = wasm.generate_keypair() as unknown;
    return new Keypair(coerceKeypairJson(raw));
  }

  /**
   * Construct from a parsed identity.json object. Accepts two shapes:
   *
   * **Legacy shape** — `{secret: number[64], pubkey_base58: string}`:
   * Validated via WASM `import_keypair_json` (checks secret length and
   * pubkey derivation) and returned as a fully-loaded `Keypair`.
   *
   * **Stub shape** — `{pubkey_base58: string, keychain_ref: string}`:
   * The identity.json was written by the CLI in invisible-bootstrap mode;
   * the secret bytes live in the OS keychain. This method cannot load them
   * — it throws {@link IdentityRequiresKeystore} so the caller can resolve
   * the secret via the appropriate keychain API.
   *
   * @param json - A parsed identity.json value — typically read from
   *               `localStorage["mnemonic.identity"]` or
   *               `~/.mnemonic/identity.json`.
   * @returns The validated `Keypair` (legacy shape only).
   * @throws `TypeError` if `json` is not an object.
   * @throws `IdentityRequiresKeystore` if `json` is a stub shape
   *         referencing an OS keychain entry.
   * @throws `UserError` if the secret length is wrong, the pubkey does not
   *         derive from the secret, or WASM validation fails.
   */
  static async fromJSON(json: KeypairJson | unknown): Promise<Keypair> {
    if (!json || typeof json !== "object") {
      throw new TypeError(
        "Keypair.fromJSON: expected {secret: number[64], pubkey_base58: string}",
      );
    }
    const obj = json as Record<string, unknown>;

    // Legacy shape: secret bytes present — validate and construct.
    if (Array.isArray(obj.secret) && typeof obj.pubkey_base58 === "string") {
      if (obj.secret.length !== 64) {
        throw new UserError(
          `Keypair.fromJSON: secret must be 64 bytes, got ${obj.secret.length}`,
        );
      }
      const wasm = await loadWasm();
      const jsonStr = JSON.stringify(obj);
      let validated: unknown;
      try {
        validated = wasm.import_keypair_json(jsonStr);
      } catch (e) {
        // WASM-bindgen errors arrive as `Error` instances; re-throw under our
        // typed hierarchy so callers can `instanceof UserError`.
        throw new UserError(`invalid keypair: ${describeError(e)}`, e);
      }
      return new Keypair(coerceKeypairJson(validated));
    }

    // Stub shape: keychain reference — caller must resolve via OS keychain.
    if (
      typeof obj.pubkey_base58 === "string" &&
      typeof obj.keychain_ref === "string"
    ) {
      throw new IdentityRequiresKeystore(obj.pubkey_base58, obj.keychain_ref);
    }

    throw new TypeError(
      "Keypair.fromJSON: expected {secret: number[64], pubkey_base58: string}",
    );
  }

  /**
   * Parse a backup-string (the output of {@link toBackupString} or the
   * webapp's "Download keypair" button) and return a validated `Keypair`.
   *
   * @param json - The backup string previously emitted by
   *               {@link toBackupString}.
   * @returns The validated `Keypair`.
   * @throws `UserError` if the input is malformed or fails WASM
   *         validation.
   */
  static async fromBackupString(json: string): Promise<Keypair> {
    const wasm = await loadWasm();
    let raw: unknown;
    try {
      raw = wasm.import_keypair_json(json);
    } catch (e) {
      throw new UserError(`malformed keypair backup: ${describeError(e)}`, e);
    }
    return new Keypair(coerceKeypairJson(raw));
  }
}

// --------------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------------

function isKeypairJsonShape(v: unknown): v is KeypairJson {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return Array.isArray(o.secret) && typeof o.pubkey_base58 === "string";
}

/**
 * Tighten the wide `unknown` from WASM into our typed shape, throwing a
 * UserError if the WASM module returned something unexpected (defense in
 * depth — this should never fire if the WASM module is the canonical build).
 */
function coerceKeypairJson(v: unknown): KeypairJson {
  if (!isKeypairJsonShape(v)) {
    throw new UserError("internal: WASM returned unexpected keypair shape");
  }
  // The WASM `serde_wasm_bindgen` boundary marshals `Vec<u8>` as a JS array
  // of numbers; verify that and clone defensively.
  const secret = v.secret.map((n) => {
    const num = typeof n === "number" ? n : Number(n);
    if (!Number.isInteger(num) || num < 0 || num > 255) {
      throw new UserError("internal: WASM secret contains non-byte value");
    }
    return num;
  });
  return { secret, pubkey_base58: v.pubkey_base58 };
}

function describeError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return JSON.stringify(e);
}

/**
 * MemoryKeyStore — in-process KeyStore backed by a plain variable.
 *
 * Intended for testing and ephemeral CLI sessions only.  This module is kept
 * out of production import paths (it is not re-exported from index.ts).
 *
 * `available` returns true by default and can be overridden for test scenarios
 * by calling `setAvailable(false)`.
 */

import { type KeyStore, type KeystoreEntry } from "./keystore.js";

export class MemoryKeyStore implements KeyStore {
  public readonly name = "memory" as const;

  private entry: KeystoreEntry | null = null;
  private _available = true;

  async get(): Promise<KeystoreEntry | null> {
    return this.entry;
  }

  async set(entry: KeystoreEntry): Promise<void> {
    this.entry = entry;
  }

  async remove(): Promise<void> {
    this.entry = null;
  }

  async available(): Promise<boolean> {
    return this._available;
  }

  /** Override availability for test scenarios. */
  setAvailable(value: boolean): void {
    this._available = value;
  }
}

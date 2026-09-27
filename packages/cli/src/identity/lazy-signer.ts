/**
 * identity/lazy-signer.ts — a `Signer` that knows the public key up front
 * and reads the private key only on the first `sign()` / `keypair()` call.
 *
 * Owner rule: the CLI touches the OS keychain only when a command really
 * needs the private key (today: `sign --anchor`, `prove`, `login`,
 * `identity export`). `recall`, `verify`, `whoami` and a local `sign` work
 * from the public key alone. `MnemonicClient` requires a signer object even
 * for read-only calls, so those commands pass this lazy one: its `pubkey`
 * comes from `identity.json` (stub or full), and nothing else is read.
 *
 * SECURITY: never log secret bytes.
 */

import {
  type Keypair,
  LocalSigner,
  type SignerInterface,
} from "@mnemonik-xyz/sdk";

import { loadIdentity, loadIdentityPubkey } from "../config.js";
import { UserError } from "../errors.js";

export class LazyIdentitySigner implements SignerInterface {
  readonly pubkey: string;
  private pending: Promise<Keypair> | null = null;
  private loadedFlag = false;

  /**
   * @param pubkey - Base58 public key from `identity.json`.
   * @param load   - Reads the full keypair (default: {@link loadIdentity},
   *                 which may read the OS keychain).
   */
  constructor(
    pubkey: string,
    private readonly load: () => Promise<Keypair> = loadIdentity,
  ) {
    this.pubkey = pubkey;
  }

  /** True once the private key has been read. */
  get loaded(): boolean {
    return this.loadedFlag;
  }

  /**
   * Read the full keypair (once; memoised). Throws `UserError` when the
   * stored secret derives to a different pubkey than `identity.json`.
   * A failed read is forgotten so a later call can retry.
   */
  keypair(): Promise<Keypair> {
    if (!this.pending) {
      const p = this.load().then((kp) => {
        if (kp.pubkey !== this.pubkey) {
          throw new UserError(
            `identity drift: identity.json pubkey ${this.pubkey} but the stored private key is for ${kp.pubkey}. ` +
              "Run `mnemonic identity status` to inspect.",
          );
        }
        this.loadedFlag = true;
        return kp;
      });
      p.catch(() => {
        if (this.pending === p) this.pending = null;
      });
      this.pending = p;
    }
    return this.pending;
  }

  async sign(bytes: Uint8Array): Promise<Uint8Array> {
    return new LocalSigner(await this.keypair()).sign(bytes);
  }
}

/**
 * Build a lazy signer for the current identity. Reads only the public key
 * (never the OS keychain). Throws `UserError` when no identity exists.
 */
export function lazyIdentitySigner(): LazyIdentitySigner {
  return new LazyIdentitySigner(loadIdentityPubkey());
}

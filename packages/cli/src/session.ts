// session.ts — keep the saved JWT usable without a manual re-login (#33).
//
// Renewal order when `token.json` is expired (or the server rejects it):
//   1. OAuth refresh token (saved by `mnemonic login` since CLI 0.3.0).
//      Needs no private key and never touches the OS keychain.
//   2. `token.json` renewed meanwhile by another CLI process → use it.
//   3. Browserless identity login (signs the server challenge). This needs
//      the private key, so it runs silently ONLY when reading the key
//      cannot prompt: the key is in a file, or the command reads it anyway
//      (`sign --anchor`).
//   4. Otherwise: one clear `UserError` hint. We do NOT fall through with
//      the expired token — the server treats an expired token on
//      `mnemonic_recall` as anonymous and would return the public pool
//      instead of the user's own memories, silently.

import {
  loginWithIdentity,
  MnemonicClient,
  refreshAccessToken,
  type TokenRefresher,
} from "@mnemonik-xyz/sdk";

import {
  identitySecretInFile,
  isTokenFresh,
  readTokenFile,
  saveToken,
  type TokenJson,
} from "./config.js";
import { fromSdkError, UserError } from "./errors.js";
import {
  type LazyIdentitySigner,
  lazyIdentitySigner,
} from "./identity/lazy-signer.js";
import { type OutputOptions, verbose } from "./output.js";
import { assertIdentityMatchesToken } from "./preflight.js";

export const CLIENT_ID = "mnemonic-cli";
/** Renew a token that expires within this window (same as the SDK). */
export const TOKEN_SKEW_MS = 60_000;

export interface SessionOptions {
  baseUrl: string;
  /** Lazy signer of the current identity (read only for identity login). */
  signer: LazyIdentitySigner;
  /**
   * True when the command reads the private key anyway (`sign --anchor`),
   * so an identity re-login may read the OS keychain. When false, identity
   * re-login runs only if the key is stored in a file (no prompt).
   */
  allowKeychain: boolean;
  output: OutputOptions;
}

export interface Session {
  client: MnemonicClient;
  /** Lazy signer: `pubkey` is known; the secret is read on first use. */
  signer: LazyIdentitySigner;
  /** The token the client starts with (fresh or just renewed). */
  token: TokenJson;
}

/**
 * Build an authenticated client for a command:
 *   identity pubkey (no keychain) → token.json → identity/JWT preflight
 *   (before any fetch) → silent renewal if expired → client with a lazy
 *   signer and a token refresher.
 *
 * @param allowKeychain - true only for commands that read the private key
 *                        anyway (`sign --anchor`).
 */
export async function openSession(
  baseUrl: string,
  output: OutputOptions,
  allowKeychain: boolean,
): Promise<Session> {
  const signer = lazyIdentitySigner();
  const raw = readTokenFile();
  assertIdentityMatchesToken(raw);
  const opts: SessionOptions = { baseUrl, signer, allowKeychain, output };
  const token = await freshToken(opts, raw);
  const client = new MnemonicClient({
    baseUrl,
    signer,
    jwt: token.jwt,
    tokenRefresher: tokenRefresher(opts, token),
  });
  return { client, signer, token };
}

/** Return a usable token: the saved one, or a silently renewed one. */
export async function freshToken(
  opts: SessionOptions,
  token: TokenJson = readTokenFile(),
): Promise<TokenJson> {
  if (isTokenFresh(token, TOKEN_SKEW_MS)) return token;
  return renewToken(opts, token, "expired");
}

/**
 * Renew `stale` regardless of its local expiry (see the file header for
 * the order). Saves the new token to `token.json`.
 *
 * @throws `UserError` with a one-line hint when no silent path works.
 */
export async function renewToken(
  opts: SessionOptions,
  stale: TokenJson,
  reason: "expired" | "rejected",
): Promise<TokenJson> {
  const newer = renewedElsewhere(stale);
  if (newer) return newer;

  let refreshFailed = false;
  if (stale.refresh_token) {
    try {
      const r = await refreshAccessToken({
        baseUrl: opts.baseUrl,
        refreshToken: stale.refresh_token,
        clientId: CLIENT_ID,
      });
      const next: TokenJson = {
        jwt: r.jwt,
        expires_at: r.expiresAt,
        sub: r.sub,
        refresh_token: r.refreshToken,
      };
      saveToken(next);
      verbose(`token renewed with refresh token (expires ${next.expires_at})`, opts.output);
      return next;
    } catch (e) {
      refreshFailed = true;
      verbose(`token refresh failed: ${describe(e)}`, opts.output);
      // Another process may have rotated the refresh token first.
      const raced = renewedElsewhere(stale);
      if (raced) return raced;
    }
  }

  if (opts.allowKeychain || identitySecretInFile()) {
    const kp = await opts.signer.keypair();
    let r;
    try {
      r = await loginWithIdentity({
        baseUrl: opts.baseUrl,
        clientId: CLIENT_ID,
        keypair: kp,
      });
    } catch (e) {
      throw fromSdkError(e);
    }
    const next: TokenJson = {
      jwt: r.jwt,
      expires_at: r.expiresAt,
      sub: r.sub,
      ...(r.refreshToken ? { refresh_token: r.refreshToken } : {}),
    };
    saveToken(next);
    verbose(`token renewed with identity login (expires ${next.expires_at})`, opts.output);
    return next;
  }

  throw new UserError(renewalHint(stale, reason, refreshFailed));
}

/**
 * SDK `TokenRefresher` for `MnemonicClient`. The client calls it when its
 * JWT is about to expire or after a 401 / 403; both cases renew.
 */
export function tokenRefresher(
  opts: SessionOptions,
  initial: TokenJson,
): TokenRefresher {
  let current = initial;
  return async () => {
    const reason = isTokenFresh(current, TOKEN_SKEW_MS) ? "rejected" : "expired";
    current = await renewToken(opts, current, reason);
    return current.jwt;
  };
}

function renewedElsewhere(stale: TokenJson): TokenJson | null {
  let onDisk: TokenJson;
  try {
    onDisk = readTokenFile();
  } catch {
    return null;
  }
  if (onDisk.jwt === stale.jwt || onDisk.sub !== stale.sub) return null;
  return isTokenFresh(onDisk, TOKEN_SKEW_MS) ? onDisk : null;
}

function renewalHint(
  stale: TokenJson,
  reason: "expired" | "rejected",
  refreshFailed: boolean,
): string {
  const what =
    reason === "expired"
      ? `token expired at ${stale.expires_at}`
      : "the server rejected the saved token";
  const why = refreshFailed
    ? "the saved refresh token was rejected (expired or revoked)"
    : "no refresh token is saved (it was created by an older CLI or by `login --token`)";
  return (
    `${what}; ${why}. ` +
    "Run `mnemonic login` once — after that the CLI renews the session automatically. " +
    "(Silent re-login would need your private key from the OS keychain, which this command does not read.)"
  );
}

function describe(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

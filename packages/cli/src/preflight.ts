// Client-side identity/JWT mismatch detection (bug 3 / Decision 7 of tech-spec).
//
// Background: a user can run `mnemonic init` (generates fresh local keypair
// at ~/.mnemonic/identity.json) and then `mnemonic login` (browser flow signs
// the OAuth challenge using the WEBAPP's localStorage keypair, not the CLI's).
// The resulting JWT.sub is the webapp's pubkey, not the CLI's. Subsequent
// sign/recall/verify produce HTTP 403 from `/api/sign-callback` because the
// COSE envelope's `signer_pubkey` ≠ `pending_bundle.jwt_sub`.
//
// Surfacing the server's 403 is unactionable. Instead we detect the mismatch
// upfront — purely client-side by comparing identity.pubkey_base58 to the
// JWT's `sub` claim that we already persist in token.json — and throw a
// UserError with three remediation paths.
//
// `assertIdentityMatchesToken` is called at the top of every authenticated
// command (sign / recall / verify, and whoami's --with-count) BEFORE any
// HTTP request is constructed.

import {
  identityPath,
  loadIdentityJson,
  loadToken,
  tokenPath,
} from "./config.js";
import { UserError } from "./errors.js";

export interface MismatchContext {
  identityPubkey: string;
  tokenSub: string;
  identityPath: string;
  tokenPath: string;
}

/**
 * Build the multi-line UserError message describing the mismatch and the
 * three actionable remediation paths. Paths are resolved via the
 * MNEMONIC_CONFIG_DIR-aware helpers so the message reflects the actual
 * on-disk locations (not literal "~/.mnemonic/").
 */
export function formatMismatchError(ctx: MismatchContext): string {
  const lines = [
    "identity mismatch:",
    `  CLI keypair (${ctx.identityPath}):     ${ctx.identityPubkey}`,
    `  logged-in JWT (${ctx.tokenPath}):       ${ctx.tokenSub}`,
    "",
    "The server will reject any sign/recall/verify because these don't match.",
    "",
    "To fix:",
    "  • mnemonic identity import --ticket <uuid>",
    '    Use webapp\'s "Send to CLI" button at https://mnemonik.xyz/install',
    "    to align CLI keypair with logged-in identity.",
    "",
    "  • mnemonic identity import --file <path>",
    "    Import an existing keypair JSON file.",
    "",
    `  • rm ${ctx.tokenPath}`,
    "    Clear logged-in JWT, then re-auth in a browser session that",
    `    has CLI keypair \`${ctx.identityPubkey}\` as its localStorage identity.`,
  ];
  return lines.join("\n");
}

/**
 * Pre-flight guard: load identity + token (both required), then assert
 * pubkey_base58 == JWT.sub. Throws UserError with the formatted remediation
 * message on mismatch — the caller has not yet built any fetch.
 *
 * Both `loadIdentityJson` and `loadToken` already throw UserError on missing
 * files, so a failed pre-flight in those branches surfaces the appropriate
 * "no identity" / "no token" hint instead of crashing.
 */
export function assertIdentityMatchesToken(): void {
  const id = loadIdentityJson();
  const tok = loadToken();
  if (id.pubkey_base58 !== tok.sub) {
    throw new UserError(
      formatMismatchError({
        identityPubkey: id.pubkey_base58,
        tokenSub: tok.sub,
        identityPath: identityPath(),
        tokenPath: tokenPath(),
      })
    );
  }
}

// `POST /oauth/google/lookup` client. Asks the server whether the
// Google account behind `jwt` is already linked to an Ed25519 pubkey
// and whether a key-escrow blob exists for it. Returns a strict TS
// shape; the underlying server response (`mcp/src/oauth/google.rs`)
// includes a `link_challenge` nonce when no link exists yet — T17
// (key-escrow + link flow) consumes that directly from a separate
// helper so this module's surface stays minimal.

import { AuthError, type LookupResult } from "./types.js";
import { DEFAULT_SERVER_ORIGIN } from "./google-oauth.js";

export interface LookupOptions {
  serverOrigin?: string;
  fetchImpl?: typeof fetch;
}

/**
 * Server response shape — mirrors `mcp/src/oauth/google.rs::LookupResponse`.
 * Exported so T17 can reuse it for the link-challenge consumer without
 * duplicating field names.
 */
export interface RawLookupResponse {
  existing_pubkey: string | null;
  escrow_present: boolean;
  link_challenge?: string;
}

/**
 * Call `POST /oauth/google/lookup` with the server-issued JWT. Maps
 * the response into the popup-friendly `LookupResult` shape
 * (`{existingPubkey, escrowPresent, linkChallenge}`) so the caller
 * branches on three fields rather than parsing JSON twice.
 *
 * The server rate-limits to 5 calls / 24h / google_sub for first-touch
 * lookups; the popup MUST call `lookupExisting` at most once per
 * sign-in to stay well under the cap. T17's onboarding flow is the
 * only intended caller.
 */
export async function lookupExisting(
  jwt: string,
  opts: LookupOptions = {},
): Promise<LookupResult> {
  if (typeof jwt !== "string" || jwt.length === 0) {
    throw new AuthError("lookup: jwt is required");
  }
  const serverOrigin = (opts.serverOrigin ?? DEFAULT_SERVER_ORIGIN).replace(
    /\/+$/u,
    "",
  );
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  const url = new URL("/oauth/google/lookup", serverOrigin).toString();

  let response: Response;
  try {
    response = await fetchImpl(url, {
      method: "POST",
      headers: {
        authorization: `Bearer ${jwt}`,
        "content-type": "application/json",
      },
      // No body fields — the JWT carries `google_sub` already.
      body: "{}",
    });
  } catch (cause) {
    throw new AuthError("lookup: server unreachable", { cause });
  }
  if (response.status === 401) {
    throw new AuthError("lookup: server rejected JWT (401)");
  }
  if (response.status === 429) {
    throw new AuthError("lookup: rate-limited (429)");
  }
  if (!response.ok) {
    throw new AuthError(`lookup: server returned ${String(response.status)}`);
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    throw new AuthError("lookup: response is not valid JSON", { cause });
  }
  if (!body || typeof body !== "object") {
    throw new AuthError("lookup: response is not an object");
  }
  const obj = body as Record<string, unknown>;
  // `existing_pubkey` can be null (server omits it via Option<None>)
  // or a non-empty string. Anything else is malformed.
  const existingPubkey =
    typeof obj.existing_pubkey === "string" && obj.existing_pubkey.length > 0
      ? obj.existing_pubkey
      : null;
  if (
    obj.existing_pubkey !== null &&
    obj.existing_pubkey !== undefined &&
    typeof obj.existing_pubkey !== "string"
  ) {
    throw new AuthError("lookup: existing_pubkey has wrong type");
  }
  if (typeof obj.escrow_present !== "boolean") {
    throw new AuthError("lookup: escrow_present missing or non-boolean");
  }
  const escrowPresent: boolean = obj.escrow_present;
  // `link_challenge` is set ONLY when `existing_pubkey` is null. We
  // surface it on the typed result so T17's link-flow consumer can
  // sign the nonce without a second HTTP call.
  let linkChallenge: string | null = null;
  if (typeof obj.link_challenge === "string" && obj.link_challenge.length > 0) {
    linkChallenge = obj.link_challenge;
  }
  return { existingPubkey, escrowPresent, linkChallenge };
}

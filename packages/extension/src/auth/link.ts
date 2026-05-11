// `/oauth/google/link` client. Posts the possession-proof signature
// that binds the user's Ed25519 pubkey to their Google `sub` (T14
// server-side). The link endpoint runs the standard `verify_jwt`
// validator, so the Bearer header carries the user's existing
// `aud=mcp` JWT (NOT the `aud=extension` JWT that escrow endpoints
// require — those are independent server-side audiences).
//
// Lives in a dedicated module so popup components route through the
// runtime facade rather than reaching into `SetPassphrase.tsx`'s
// inline copy of the same fetch loop. Audit M3 / AUD-C-08 closes
// the layering violation that previously had SetPassphrase importing
// `DEFAULT_SERVER_ORIGIN` + `AuthError` directly.

import { AuthError } from "./types.js";
import { DEFAULT_SERVER_ORIGIN } from "./google-oauth.js";

/** Body shape mirrors `mcp/src/oauth/google.rs::LinkRequest`. */
export interface LinkGoogleBody {
  pubkey_base58: string;
  possession_proof_base64: string;
  challenge: string;
}

export interface LinkGoogleOptions {
  serverOrigin?: string;
  fetchImpl?: typeof fetch;
}

/**
 * POST `/oauth/google/link`. Throws {@link AuthError} on every non-2xx
 * (401, 403, 409, 429, generic). The popup catches and surfaces a
 * typed UI message.
 */
export async function linkGoogleAccount(
  jwt: string,
  body: LinkGoogleBody,
  opts: LinkGoogleOptions = {},
): Promise<void> {
  if (typeof jwt !== "string" || jwt.length === 0) {
    throw new AuthError("linkGoogleAccount: jwt is required");
  }
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  const origin = (opts.serverOrigin ?? DEFAULT_SERVER_ORIGIN).replace(
    /\/+$/u,
    "",
  );
  const url = new URL("/oauth/google/link", origin).toString();
  let resp: Response;
  try {
    resp = await fetchImpl(url, {
      method: "POST",
      headers: {
        authorization: `Bearer ${jwt}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(body),
    });
  } catch (cause) {
    throw new AuthError("linkGoogleAccount: server unreachable", { cause });
  }
  if (resp.status === 401) throw new AuthError("linkGoogleAccount: 401");
  if (resp.status === 403) throw new AuthError("linkGoogleAccount: 403");
  if (resp.status === 409)
    throw new AuthError("linkGoogleAccount: already linked");
  if (resp.status === 429)
    throw new AuthError("linkGoogleAccount: 429 rate-limited");
  if (!resp.ok) {
    throw new AuthError(
      `linkGoogleAccount: server returned ${String(resp.status)}`,
    );
  }
}

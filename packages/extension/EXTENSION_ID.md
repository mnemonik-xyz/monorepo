# Mnemonik — Chrome extension ID

**Stable extension ID:** `iegoicpcogbnnnajgfdbljfickgfnfoj`

This is the Chrome Web Store Item ID assigned to the Mnemonik
extension. It is the identifier Google's OAuth flow binds the
sign-in redirect to, and the identifier the server's
`GOOGLE_OAUTH_REDIRECT_URI` must match.

## Why it matters

`chrome.identity.launchWebAuthFlow` uses a redirect URI of the form

```
https://<extension-id>.chromiumapp.org/<path>
```

T16 passes `path = "google"` (see
`packages/extension/src/auth/google-oauth.ts::REDIRECT_PATH`), so the
extension's redirect URI is:

```
https://iegoicpcogbnnnajgfdbljfickgfnfoj.chromiumapp.org/google
```

The extension sends this URI on the `/oauth/google/start` request.
The server stores it in `PendingGoogleFlow.redirect_uri` and, after
exchanging Google's auth code for an `id_token`, redirects the
`chrome.identity.launchWebAuthFlow` flow back to this URI with the
server-minted JWT.

**This URI is NEVER whitelisted in Google Cloud Console.** Google
never redirects to it directly — only the server does, via the
final `redirect_to(&to)` in `mcp/src/oauth/google.rs::google_call-
back_handler`. Whitelisting the chromiumapp URI in Google would
cause Google to redirect there directly, bypassing the server's
callback handler, and the extension's `state` would be Google's
server-side state instead of the client-side CSRF state → state
mismatch.

The URI Google MUST be whitelisted for is the server's callback
endpoint:

```
https://mcp.mnemonik.xyz/oauth/google/callback
```

This is what `GOOGLE_OAUTH_REDIRECT_URI` env var holds; the server
passes it to Google on the authorize URL.

## Stable ID — two installation modes

1. **Web Store install (production / user-facing).** Google assigns
   and enforces the ID once the extension is published. End-users
   install via `https://chrome.google.com/webstore/detail/<id>` and
   the production ID is `iegoicpcogbnnnajgfdbljfickgfnfoj`.
2. **Unpacked dev load.** Chrome derives the ID from the SHA-256 of
   the manifest's `key` field. To stabilise the dev-build ID across
   developer machines (so the chromiumapp.org redirect URI line is
   the same for every contributor), the `key` field IS committed to
   `manifest.json`. The committed key is an RSA-2048 PUBLIC key only
   (SubjectPublicKeyInfo, 294 bytes) — no private key material is
   ever in the repo. Exposing the public key is safe: it cannot be
   used to sign anything, and Chrome only uses it to derive the
   extension ID.

   **Important:** the committed dev-build key maps to a DIFFERENT
   extension ID than the Web Store production ID. Developers who do
   `Load unpacked` against this repo will get the dev-build ID, which
   is intentional — it lets us OAuth-test against
   `https://mcp.mnemonik.xyz` from the dev-build without going through
   a CWS release. The corresponding `redirect_uris` are registered
   server-side under the `google_oauth_client_id` configuration; see
   `mcp/src/config.rs` and the `MNEMONIK_OAUTH_REDIRECT_URIS` env var.

   PR134-C-07 / PR134-S-01 / BUG2-10: this is a deliberate policy
   change vs. the previous "key is NOT checked in" stance. The prior
   policy required every developer to manually paste a shared key
   into a gitignored manifest.local.json, which contradicted the
   "anyone can clone + run" goal of the dev workflow.

   The private key used to sign CRX bundles for the Web Store remains
   server-side (Google manages it inside the Web Store dashboard once
   the extension is enrolled); only the public key needed for the
   stable-ID derivation lives here.

## Google Cloud OAuth client

- Project: `mnemonik-xyz`
- OAuth client type: **Web application** (not Chrome extension —
  the latter requires a Web Store Item ID for creation, but the
  resulting client is incompatible with `launchWebAuthFlow`).
- Authorized redirect URI: the URL above.
- Client ID is the public token sent on `/oauth/google/start`. The
  client secret is held server-side only and never reaches the
  extension (Decision 5 in `work/chrome-extension/decisions.md`).

## Server env vars

See `.env.example`:

- `GOOGLE_OAUTH_CLIENT_ID` — public; the same string the OAuth
  consent screen surfaces to end users.
- `GOOGLE_OAUTH_CLIENT_SECRET` — held only on the MCP server. Used
  in the `oauth2.googleapis.com/token` exchange (PKCE flow; secret
  is optional in pure RFC 7636 but Google's web-application client
  type still requires it).
- `GOOGLE_OAUTH_REDIRECT_URI` — must equal the URI above byte for
  byte.

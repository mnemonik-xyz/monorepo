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
full redirect URI is:

```
https://iegoicpcogbnnnajgfdbljfickgfnfoj.chromiumapp.org/google
```

This exact URI must be whitelisted under "Authorized redirect URIs"
in the Google Cloud OAuth client (project `mnemonik-xyz`, OAuth
client type "Web application"). The MCP server's
`GOOGLE_OAUTH_REDIRECT_URI` env var must match the same string —
Google's `oauth2.googleapis.com/token` endpoint compares the value
on the token-exchange leg.

## Stable ID — two installation modes

1. **Web Store install (production / user-facing).** Google assigns
   and enforces the ID once the extension is published. End-users
   install via `https://chrome.google.com/webstore/detail/<id>` and
   the ID is automatically `iegoicpcogbnnnajgfdbljfickgfnfoj`.
2. **Unpacked dev load.** Chrome derives the ID from the SHA-256 of
   the manifest's `key` field. To get the same ID under
   `chrome://extensions → Load unpacked`, the developer must add a
   `key` field to `manifest.json` whose public-key bytes hash to the
   same ID. The `key` value is intentionally NOT checked into this
   repo — see `RELEASE.md` for how it is stored.

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

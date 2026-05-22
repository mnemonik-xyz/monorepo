// `mnemonic login` — three paths:
//
//   default (browserless):  loads `~/.mnemonic/identity.json`, signs the
//                           server's PKCE challenge with the local Ed25519
//                           keypair, exchanges for a JWT. No browser, no
//                           loopback server, fully agent-friendly. `JWT.sub`
//                           is `identity.pubkey_base58` by construction —
//                           closes the identity-mismatch class of bugs that
//                           the browser flow can introduce (issue #27).
//   --browser:              legacy PKCE + loopback HTTP server bound to
//                           127.0.0.1:0 (random port). Opens the system
//                           browser, OAuth challenge is signed by the
//                           webapp's localStorage keypair. Retained for
//                           users who have already paired the CLI identity
//                           against the webapp's localStorage key.
//   --token <jwt>:          headless. Decodes header b64 to assert
//                           `alg=HS256`, parses payload, asserts `exp` in
//                           future, persists. No browser, no server.
//
// Decision 5: loopback server binds 127.0.0.1 ONLY (never 0.0.0.0). Single-
// shot, 5-minute timeout. State is validated BEFORE the code is sent to the
// token endpoint (defence-in-depth — SDK validates again before fetch).

import {
  createServer,
  type IncomingMessage,
  type Server,
  type ServerResponse,
} from "node:http";
import type { AddressInfo } from "node:net";

import {
  IdentityRequiresKeystore,
  buildAuthorizeUrl,
  exchangeCodeForToken,
  loginWithIdentity,
  parseJwtPayload,
} from "@mnemonik-xyz/sdk";

import {
  identityExists,
  loadIdentity,
  loadIdentityJson,
  saveToken,
} from "../config.js";
import { AuthError, CliError, fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions, verbose } from "../output.js";

export interface LoginOptions extends OutputOptions {
  token?: string;
  baseUrl?: string;
  /** Opt-in to the legacy browser-mediated OAuth flow. */
  browser?: boolean;
  /** Internal — disable the `open` browser launch (for tests). */
  noOpen?: boolean;
  /** Internal — override loopback timeout for tests (ms). */
  timeoutMs?: number;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";
const CLIENT_ID = "mnemonic-cli";
const DEFAULT_TIMEOUT_MS = 5 * 60 * 1000;

export async function runLogin(opts: LoginOptions): Promise<void> {
  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;

  if (opts.token) {
    await runHeadless(opts.token, opts);
    return;
  }
  if (opts.browser) {
    await runInteractive(baseUrl, opts);
    return;
  }
  await runBrowserless(baseUrl, opts);
}

// ── browserless (default) ──────────────────────────────────────────────────

/**
 * Programmatic OAuth: sign the server's PKCE challenge with the local
 * keypair, exchange for a JWT. No browser, no loopback server.
 *
 * Requires `~/.mnemonic/identity.json` to already exist — if it doesn't,
 * direct the user at the three pairing paths (`init`, `identity import
 * --ticket`, `identity import --file`).
 */
async function runBrowserless(
  baseUrl: string,
  opts: LoginOptions,
): Promise<void> {
  if (!identityExists()) {
    throw new UserError(
      "no local identity — run one of:\n" +
        "  • mnemonic init                              — generate a fresh CLI keypair\n" +
        "  • mnemonic identity import --ticket <uuid>   — pair from the webapp ('Send to CLI')\n" +
        "  • mnemonic identity import --file <path>     — import an existing keypair JSON\n" +
        "  • mnemonic login --browser                   — fall back to the legacy browser flow",
    );
  }
  const kp = await loadIdentity();
  verbose(`base_url=${baseUrl}`, opts);
  verbose(`signing OAuth challenge with local pubkey=${kp.pubkey}`, opts);

  let result: { jwt: string; expiresAt: string; sub: string };
  try {
    result = await loginWithIdentity({
      baseUrl,
      clientId: CLIENT_ID,
      keypair: kp,
    });
  } catch (e) {
    throw fromSdkError(e);
  }
  verbose(`server returned JWT.sub=${result.sub}`, opts);

  saveToken({
    jwt: result.jwt,
    expires_at: result.expiresAt,
    sub: result.sub,
  });

  format(
    { sub: result.sub, expires_at: result.expiresAt, mode: "browserless" },
    opts,
    () => `login OK\nsub: ${result.sub}\nexpires: ${result.expiresAt}`,
  );
}

// ── headless ────────────────────────────────────────────────────────────────

async function runHeadless(jwt: string, opts: LoginOptions): Promise<void> {
  let payload: { sub: string; exp: number };
  try {
    payload = parseJwtPayload(jwt);
  } catch (e) {
    throw fromSdkError(e);
  }

  const expiresAt = new Date(payload.exp * 1000).toISOString();
  saveToken({ jwt, expires_at: expiresAt, sub: payload.sub });
  warnIfMismatch(payload.sub);

  format(
    { sub: payload.sub, expires_at: expiresAt, mode: "headless" },
    opts,
    (_d, _color) =>
      `login OK (headless)\nsub: ${payload.sub}\nexpires: ${expiresAt}`,
  );
}

// ── interactive ─────────────────────────────────────────────────────────────

async function runInteractive(
  baseUrl: string,
  opts: LoginOptions,
): Promise<void> {
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;

  // 1. Bind a one-shot loopback server first so we know the redirect_uri.
  const { server, port } = await listenLoopback();
  const redirectUri = `http://127.0.0.1:${port}/callback`;

  // 2. Build authorize URL (SDK stores `{verifier, state, redirectUri,
  //    sessionId}` keyed by sessionId).
  let authz: { url: string; state: string; sessionId: string };
  try {
    const r = await buildAuthorizeUrl({
      baseUrl,
      clientId: CLIENT_ID,
      redirectUri,
    });
    authz = { url: r.url, state: r.state, sessionId: r.sessionId };
  } catch (e) {
    server.close();
    throw fromSdkError(e);
  }

  hint(`opening browser: ${authz.url}`, opts);
  hint(
    `if no browser opens, paste the URL into one. callback URL: ${redirectUri}`,
    opts,
  );

  // 3. Open the browser (best-effort — failure is non-fatal because users
  //    can paste the URL manually).
  if (!opts.noOpen) {
    try {
      const { default: open } = await import("open");
      await open(authz.url);
    } catch {
      // ignore — `hint` above already told the user how to recover.
    }
  }

  // 4. Await the single callback or timeout.
  let cb: CallbackResult;
  try {
    cb = await awaitCallback(server, timeoutMs);
  } catch (e) {
    if (e instanceof AuthError) throw e;
    throw new AuthError(
      `loopback server error: ${e instanceof Error ? e.message : String(e)}`,
      e,
    );
  } finally {
    server.close();
  }

  // 5. State validation before the code exchange (defence-in-depth — the
  //    SDK validates again, but we don't want to even surface the code to
  //    the token endpoint if state was wrong).
  if (cb.state !== authz.state) {
    throw new AuthError(
      "oauth: state mismatch (possible CSRF) — login aborted",
    );
  }

  // 6. Exchange code → JWT.
  let token: { jwt: string; expiresAt: string };
  try {
    token = await exchangeCodeForToken({
      baseUrl,
      code: cb.code,
      state: cb.state,
      redirectUri,
      sessionId: authz.sessionId,
    });
  } catch (e) {
    throw fromSdkError(e);
  }

  // 7. Decode payload for `sub` + persist.
  let payload: { sub: string; exp: number };
  try {
    payload = parseJwtPayload(token.jwt);
  } catch (e) {
    throw fromSdkError(e);
  }
  saveToken({ jwt: token.jwt, expires_at: token.expiresAt, sub: payload.sub });
  warnIfMismatch(payload.sub);

  format(
    { sub: payload.sub, expires_at: token.expiresAt, mode: "interactive" },
    opts,
    () => `login OK\nsub: ${payload.sub}\nexpires: ${token.expiresAt}`,
  );
}

/**
 * 0.1.5: emit a stderr warning when the just-saved JWT.sub doesn't match the
 * local CLI keypair pubkey. Per Decision 7 of tech-spec, the bug 3 root cause
 * is that the webapp's localStorage keypair (which signs the OAuth challenge)
 * is independent from `~/.mnemonic/identity.json`. We still save the token
 * (Option C — minimum surprise; future commands' pre-flight check will
 * surface the mismatch with the same actionable error). When no local
 * identity exists yet, no warning is emitted (the user will run `init` next
 * which has its own stale-token clear path).
 */
function warnIfMismatch(jwtSub: string): void {
  if (!identityExists()) return;
  let identityPub: string;
  try {
    identityPub = loadIdentityJson().pubkey_base58;
  } catch (e) {
    // Stub-shaped identity.json: the pubkey is still on the `IdentityRequires
    // Keystore` payload. Use that directly — we don't need the secret here.
    if (e instanceof IdentityRequiresKeystore) {
      identityPub = e.pubkey_base58;
    } else if (e instanceof CliError) {
      // Identity file present but unreadable — surface nothing here; the user
      // already has bigger problems and the next command will report them.
      return;
    } else {
      throw e;
    }
  }
  if (identityPub === jwtSub) return;
  process.stderr.write(
    `\nWARNING: logged-in identity sub <${jwtSub}> doesn't match local keypair <${identityPub}>.\n` +
      `         All sign/recall/verify will fail until aligned.\n` +
      `         Fix: mnemonic identity import --ticket <uuid>   (from webapp)\n` +
      `              OR mnemonic init --force   (replaces keypair, then re-login)\n`,
  );
}

interface CallbackResult {
  code: string;
  state: string;
}

/** Bind an HTTP server on `127.0.0.1:0`. Returns the chosen port. */
function listenLoopback(): Promise<{ server: Server; port: number }> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    // 127.0.0.1 ONLY — never 0.0.0.0 (would expose the callback to LAN).
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address() as AddressInfo | null;
      if (!addr || typeof addr === "string") {
        server.close();
        reject(new Error("loopback server returned no address"));
        return;
      }
      resolve({ server, port: addr.port });
    });
  });
}

/**
 * Listen for exactly one `/callback?code=...&state=...` GET. Resolves with
 * the parsed query, rejects on `error=...`, mismatched method/path, or the
 * `timeoutMs` deadline. Always responds with a tiny HTML page so the user's
 * browser tab shows a confirmation rather than hanging.
 */
function awaitCallback(
  server: Server,
  timeoutMs: number,
): Promise<CallbackResult> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const settle = (fn: () => void): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      // Detach the request listener immediately so any stray probe arriving
      // after settle() does not trigger another response. The outer `finally`
      // in runInteractive still calls server.close() to release the port.
      server.removeListener("request", onRequest);
      fn();
    };

    const timer = setTimeout(() => {
      settle(() =>
        reject(
          new AuthError(
            `loopback callback timed out after ${Math.round(timeoutMs / 1000)}s`,
          ),
        ),
      );
    }, timeoutMs);

    const onRequest = (req: IncomingMessage, res: ServerResponse): void => {
      // Defensive: only accept GET /callback*. Reject everything else with
      // 404 so a stray probe doesn't tear down the server.
      if (req.method !== "GET") {
        res.writeHead(405, { "content-type": "text/plain" });
        res.end("method not allowed");
        return;
      }
      const url = new URL(req.url ?? "/", `http://127.0.0.1`);
      if (url.pathname !== "/callback") {
        res.writeHead(404, { "content-type": "text/plain" });
        res.end("not found");
        return;
      }

      const err = url.searchParams.get("error");
      if (err) {
        // Surface the IdP-supplied error code/description verbatim — they're
        // the only diagnostic we have.
        const desc = url.searchParams.get("error_description") ?? "";
        respondHtml(
          res,
          400,
          `<h1>Login failed</h1><p>${escapeHtml(err)}: ${escapeHtml(desc)}</p>`,
        );
        settle(() =>
          reject(new AuthError(`oauth callback error: ${err} ${desc}`)),
        );
        return;
      }

      const code = url.searchParams.get("code");
      const state = url.searchParams.get("state");
      if (!code || !state) {
        respondHtml(
          res,
          400,
          "<h1>Login failed</h1><p>Missing code or state.</p>",
        );
        settle(() =>
          reject(new AuthError("oauth callback missing code/state")),
        );
        return;
      }

      respondHtml(
        res,
        200,
        "<h1>Login complete</h1><p>You can close this tab and return to your terminal.</p>",
      );
      settle(() => resolve({ code, state }));
    };

    server.on("request", onRequest);
    server.once("close", () => {
      // If the server is closed before a callback (e.g. parent error), fail.
      settle(() =>
        reject(new AuthError("loopback server closed before callback")),
      );
    });
  });
}

function respondHtml(res: ServerResponse, status: number, html: string): void {
  res.writeHead(status, { "content-type": "text/html; charset=utf-8" });
  res.end(`<!doctype html><html><body>${html}</body></html>`);
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

import nacl from "tweetnacl";
import bs58 from "bs58";
import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import IdentityPanel from "../components/IdentityPanel";
import InstallButtons from "../components/InstallButtons";
import { readJwt, writeIdentity } from "../lib/storage";

/**
 * Install hub (`/install`). Two columns on desktop, stacked on mobile:
 *   - Install buttons (Cursor / VS Code / Claude Desktop / WindSurf).
 *   - Identity panel (Generate / Import / Export keypair).
 *
 * `?pull=<short_code>` flow (Decision 15):
 *   If a short_code query param is present and the user is logged in,
 *   the page calls the redeem endpoint to adopt the CLI-issued identity
 *   into the webapp's localStorage keypair. This is the in-browser half
 *   of the `mnemonic identity push-to-webapp` CLI command (Task 12/13).
 *
 *   Crypto: We generate an ephemeral X25519 keypair via tweetnacl's
 *   `nacl.box.keyPair()`. The server wraps the 64-byte Ed25519 secret in
 *   a NaCl sealed box (XSalsa20-Poly1305, wire format: nonce[24] || ciphertext)
 *   using `crypto_box::SalsaBox` on the Rust side — bit-compatible with
 *   `nacl.box.open()`. We decrypt locally and write to localStorage.
 *
 *   If the user is NOT logged in when `?pull=` is present, we store the
 *   short_code in sessionStorage and redirect to OAuth. On return the
 *   component picks it up and resumes.
 */

const MCP_BASE =
  (import.meta.env?.VITE_MCP_BASE as string | undefined) ??
  "https://mcp.mnemonik.xyz";

/**
 * Redeem endpoint — placeholder constant so a name-mismatch with the
 * Task 12 server implementation is easy to fix in one place.
 */
export const REDEEM_ENDPOINT = "/api/cli-bootstrap/redeem";

/** sessionStorage key used to persist the short_code across the OAuth redirect. */
const PULL_SESSION_KEY = "mnemonic.pull_short_code";

type PullState =
  | { kind: "idle" }
  | { kind: "working"; message: string }
  | { kind: "success"; message: string }
  | { kind: "error"; message: string };

/**
 * Decode a base64 string to a Uint8Array — handles both standard (+/) and
 * base64url (-_) variants.
 */
function b64ToBytes(b64: string): Uint8Array {
  const std = b64.replace(/-/g, "+").replace(/_/g, "/");
  const padded = std + "=".repeat((4 - (std.length % 4)) % 4);
  const binary = atob(padded);
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

/**
 * Execute the pull-from-CLI redemption flow for the given short_code.
 * Returns `{secret64, pubkeyBase58}` on success or throws on error.
 *
 * Wire contract (Task 12):
 *   POST /api/cli-bootstrap/redeem
 *   Body: { short_code: string, redeemer_eph_pub: string (base64) }
 *   Response: { wrapped_secret: string (base64), eph_pub: string (base64), issuer_pubkey_base58: string }
 *
 * Encryption scheme: NaCl box (XSalsa20-Poly1305 via crypto_box::SalsaBox on
 * the Rust server side). Wire format of wrapped_secret: nonce[24] || ciphertext.
 * Decrypted with tweetnacl's nacl.box.open() — bit-compatible with SalsaBox.
 */
async function redeemShortCode(
  shortCode: string,
  jwt: string,
): Promise<{ secret64: number[]; pubkeyBase58: string }> {
  // 1. Generate ephemeral X25519 keypair (Curve25519) for the NaCl box.
  const eph = nacl.box.keyPair();
  const ephPubB64 = btoa(String.fromCharCode(...eph.publicKey));

  // 2. POST to server with our ephemeral public key.
  const res = await fetch(`${MCP_BASE}${REDEEM_ENDPOINT}`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${jwt}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      short_code: shortCode,
      redeemer_eph_pub: ephPubB64,
    }),
  });

  if (!res.ok) {
    let detail = `HTTP ${res.status}`;
    try {
      const body = await res.json();
      if (body && typeof body.error === "string") detail = body.error;
    } catch {
      // ignore
    }
    throw new Error(detail);
  }

  const body = (await res.json()) as {
    wrapped_secret?: unknown;
    eph_pub?: unknown;
    issuer_pubkey_base58?: unknown;
  };

  if (
    typeof body.wrapped_secret !== "string" ||
    typeof body.eph_pub !== "string" ||
    typeof body.issuer_pubkey_base58 !== "string"
  ) {
    throw new Error("Server returned unexpected shape from redeem endpoint");
  }

  // 3. Decode the server's ephemeral public key and the wrapped secret.
  // Wire format: nonce(24 bytes) || ciphertext+tag
  const serverEphPub = b64ToBytes(body.eph_pub);
  const wrapped = b64ToBytes(body.wrapped_secret);
  if (wrapped.length < nacl.box.nonceLength) {
    throw new Error("wrapped_secret too short");
  }
  const nonce = wrapped.slice(0, nacl.box.nonceLength);
  const ciphertext = wrapped.slice(nacl.box.nonceLength);

  // 4. Decrypt using NaCl box — bit-compatible with Rust's crypto_box::SalsaBox.
  const plaintext = nacl.box.open(
    ciphertext,
    nonce,
    serverEphPub,
    eph.secretKey,
  );
  if (plaintext === null) {
    throw new Error("Decryption failed: authentication tag mismatch");
  }

  if (plaintext.length !== 64) {
    throw new Error(`Expected 64-byte secret, got ${plaintext.length}`);
  }

  // 5. Verify pubkey: derive Ed25519 public key from the last 32 bytes of the
  // secret and compare against the server-supplied issuer_pubkey_base58.
  const derivedPubkey = bs58.encode(plaintext.slice(32));
  if (derivedPubkey !== body.issuer_pubkey_base58) {
    throw new Error(
      "Pubkey mismatch: decrypted secret does not match server-reported pubkey",
    );
  }

  return {
    secret64: Array.from(plaintext),
    pubkeyBase58: body.issuer_pubkey_base58,
  };
}

export default function Install() {
  const [searchParams, setSearchParams] = useSearchParams();
  const [pullState, setPullState] = useState<PullState>({ kind: "idle" });

  // On mount: check for ?pull= query param or a sessionStorage-stashed
  // short_code (left by a previous OAuth redirect).
  useEffect(() => {
    const fromQuery = searchParams.get("pull");
    const fromSession = sessionStorage.getItem(PULL_SESSION_KEY);
    const shortCode = fromQuery ?? fromSession;

    if (!shortCode) return;

    const jwt = readJwt();
    if (!jwt) {
      // Not logged in — stash the short_code and redirect to OAuth.
      if (fromQuery) {
        sessionStorage.setItem(PULL_SESSION_KEY, fromQuery);
        // Strip the query param to avoid re-triggering after OAuth return.
        setSearchParams({}, { replace: true });
      }
      window.location.assign("/oauth/consent");
      return;
    }

    // Logged in — consume the short_code and run the redemption flow.
    sessionStorage.removeItem(PULL_SESSION_KEY);
    setSearchParams({}, { replace: true });
    setPullState({ kind: "working", message: "Adopting CLI identity..." });

    redeemShortCode(shortCode, jwt)
      .then(({ secret64, pubkeyBase58 }) => {
        writeIdentity({ secret: secret64, pubkey_base58: pubkeyBase58 });
        setPullState({
          kind: "success",
          message:
            "CLI identity adopted — webapp keypair updated. Your IDE and CLI now share the same identity.",
        });
      })
      .catch((e: unknown) => {
        const msg =
          e instanceof Error ? e.message : "Unknown error during redemption";
        setPullState({ kind: "error", message: `Adoption failed: ${msg}` });
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <main className="min-h-screen px-4 py-10">
      <div className="mx-auto max-w-4xl space-y-8">
        <header className="space-y-2">
          <Link
            to="/"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            &larr; Back
          </Link>
          <h1 className="text-3xl font-bold tracking-tight text-text-primary">
            Install
          </h1>
          <p className="text-sm text-text-muted">
            Connect Mnemonic to your AI tool, then create or import the Ed25519
            keypair that will sign your memories.
          </p>
        </header>

        {/* Pull-from-CLI status banners */}
        {pullState.kind === "working" && (
          <div
            className="rounded-md border border-accent-primary/30 bg-accent-primary/5 px-4 py-3 text-sm text-accent-primary"
            role="status"
            aria-live="polite"
            data-testid="pull-working"
          >
            {pullState.message}
          </div>
        )}
        {pullState.kind === "success" && (
          <div
            className="rounded-md border border-green-500/30 bg-green-500/10 px-4 py-3 text-sm text-green-400"
            role="status"
            aria-live="polite"
            data-testid="pull-success"
          >
            {pullState.message}
          </div>
        )}
        {pullState.kind === "error" && (
          <p
            className="rounded-md border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-400"
            role="alert"
            data-testid="pull-error"
          >
            {pullState.message}
          </p>
        )}

        <div className="grid gap-8 md:grid-cols-2">
          <InstallButtons />
          <IdentityPanel />
        </div>

        <section
          aria-labelledby="install-instructions"
          className="space-y-4"
          data-testid="install-instructions"
        >
          <h2
            id="install-instructions"
            className="text-lg font-semibold text-text-primary"
          >
            Step-by-step
          </h2>
          <p className="text-sm text-text-muted">
            Generate or import your keypair first — both flows below redirect to
            OAuth, which signs the request with the local Ed25519 secret.
          </p>

          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                Cursor
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Install in Cursor</span>{" "}
                  above.
                </li>
                <li>2. Approve the deeplink prompt in your browser.</li>
                <li>
                  3. Confirm{" "}
                  <span className="font-mono text-text-primary">Mnemonic</span>{" "}
                  in the Cursor MCP install dialog.
                </li>
                <li>4. Complete OAuth in the popup window.</li>
              </ol>
            </article>

            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                VS Code
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Install in VS Code</span>{" "}
                  above.
                </li>
                <li>
                  2. Approve the deeplink prompt; VS Code 1.93+ is required.
                </li>
                <li>
                  3. Accept the{" "}
                  <span className="font-mono text-text-primary">Mnemonic</span>{" "}
                  MCP server in the install dialog.
                </li>
                <li>
                  4. Sign in via OAuth when GitHub Copilot first calls a tool.
                </li>
              </ol>
            </article>

            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                Claude Desktop
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">
                    Add to Claude Desktop
                  </span>{" "}
                  above.
                </li>
                <li>2. Copy the JSON snippet.</li>
                <li>
                  3. Paste into{" "}
                  <span className="font-mono text-text-primary">
                    claude_desktop_config.json
                  </span>
                  .
                </li>
                <li>4. Restart Claude Desktop.</li>
              </ol>
            </article>

            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                WindSurf
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Install in WindSurf</span>{" "}
                  above and copy the JSON snippet.
                </li>
                <li>
                  2. Open{" "}
                  <span className="font-mono text-text-primary">
                    ~/.codeium/windsurf/mcp_config.json
                  </span>{" "}
                  and merge it into{" "}
                  <span className="font-mono text-text-primary">
                    mcpServers
                  </span>
                  .
                </li>
                <li>3. Reload WindSurf so Cascade picks up the new server.</li>
                <li>4. Complete OAuth on the first tool call.</li>
              </ol>
            </article>
          </div>
        </section>
      </div>
    </main>
  );
}

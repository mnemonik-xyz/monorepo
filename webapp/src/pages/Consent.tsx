import { useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { loadWasm } from "../lib/wasm";
import { readIdentity } from "../lib/storage";

/**
 * OAuth consent page (`/oauth/consent`).
 *
 * Lands here when an MCP client (Cursor / VS Code / Claude.ai) hits
 * `GET https://mcp.mnemonik.xyz/oauth/authorize?...&redirect_uri=...&state=...`
 * — the server's bootstrap (`oauth::authorize_init_handler`) builds a
 * canonical-CBOR challenge under that `state`, then 302-redirects the
 * browser here with `?challenge=<base64>&state=<state>`.
 *
 * Flow inside this page:
 *   1. Read challenge + state from URL.
 *   2. Read keypair from localStorage. Missing → guide user to /install.
 *   3. Show a brief preview (we do NOT decode the CBOR — the only thing
 *      the user is approving is "let this MCP client act on my behalf").
 *   4. On Approve → WASM `sign_challenge(keypair, challenge_bytes)` →
 *      POST {state, cose_signed} to `https://mcp.mnemonik.xyz/oauth/authorize`.
 *   5. Server returns {code, state, redirect_uri}. We then 302-bounce
 *      `window.location` to `redirect_uri?code=...&state=...` so the
 *      MCP client's local OAuth callback handler picks the code up.
 *   6. The client exchanges the code at `/oauth/token` for a JWT,
 *      then uses Bearer auth on `/mcp` calls.
 *
 * NOTE: the page does NOT see the redirect_uri up-front (it's bound to
 * the pending entry on the server). The server ships it back inside the
 * AuthorizeResponse so the browser can complete the handoff.
 */

// Configurable for e2e tests + local dev (parallel to Sign.tsx::MCP_BASE).
const MCP_BASE =
  (import.meta.env?.VITE_MCP_BASE as string | undefined) ??
  "https://mcp.mnemonik.xyz";
const MCP_AUTHORIZE_URL = `${MCP_BASE}/oauth/authorize`;

type Phase =
  | { kind: "loading" }
  | { kind: "ready" }
  | { kind: "no-identity" }
  | { kind: "signing" }
  | { kind: "redirecting"; redirectUrl: string }
  | { kind: "error"; message: string };

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export default function Consent() {
  const [params] = useSearchParams();
  const challenge = params.get("challenge") ?? "";
  const state = params.get("state") ?? "";

  const challengeBytes = useMemo(() => {
    try {
      return base64ToBytes(decodeURIComponent(challenge));
    } catch {
      return null;
    }
  }, [challenge]);

  const identity = useMemo(() => readIdentity(), []);

  const [phase, setPhase] = useState<Phase>({ kind: "loading" });

  useEffect(() => {
    if (!challenge || !state) {
      setPhase({
        kind: "error",
        message:
          "Missing `challenge` or `state` query parameter. Restart the OAuth flow from your MCP client.",
      });
      return;
    }
    if (!challengeBytes) {
      setPhase({
        kind: "error",
        message:
          "Challenge is not valid base64. The link may be truncated; restart the OAuth flow from your MCP client.",
      });
      return;
    }
    if (!identity) {
      setPhase({ kind: "no-identity" });
      return;
    }
    setPhase({ kind: "ready" });
  }, [challenge, state, challengeBytes, identity]);

  async function handleApprove() {
    if (!identity || !challengeBytes) return;
    setPhase({ kind: "signing" });
    try {
      const wasm = await loadWasm();
      // sign_challenge returns a 64-byte raw Ed25519 signature, NOT a
      // COSE_Sign1 envelope. The server's /oauth/authorize accepts both
      // (legacy `cose_signed` is a serde alias for `signature`) and now
      // verifies raw Ed25519 against the stored challenge bytes — this
      // closes the "extraneous data in CBOR" failure mode that came from
      // mistakenly treating the 64-byte signature as a COSE envelope.
      const sig = wasm.sign_challenge(identity, challengeBytes);
      const sigB64 = btoa(String.fromCharCode(...sig));
      const resp = await fetch(MCP_AUTHORIZE_URL, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          state,
          signature: sigB64,
          signer_pubkey: identity.pubkey_base58,
        }),
      });
      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(
          `server rejected approval (HTTP ${resp.status}): ${text}`
        );
      }
      const data: {
        code: string;
        state: string;
        redirect_uri: string;
      } = await resp.json();
      const url = new URL(data.redirect_uri);
      url.searchParams.set("code", data.code);
      url.searchParams.set("state", data.state);
      const redirectUrl = url.toString();
      setPhase({ kind: "redirecting", redirectUrl });
      window.location.href = redirectUrl;
    } catch (err) {
      setPhase({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }

  function handleReject() {
    setPhase({
      kind: "error",
      message: "Approval rejected. You can close this tab.",
    });
  }

  return (
    <main
      className="mx-auto max-w-xl px-6 py-16 text-text-primary"
      data-testid="consent-page"
    >
      <h1 className="text-2xl font-semibold">Authorize MCP client</h1>
      <p className="mt-3 text-sm text-text-muted">
        An external AI tool is asking to act on your behalf via the Mnemonic
        protocol. Approving will sign a one-time challenge with your
        localStorage keypair so the client can call{" "}
        <code className="font-mono">/mcp</code> as you.
      </p>

      <div className="mt-6 rounded-md border border-text-muted/30 bg-white/5 px-4 py-3 text-sm">
        <div className="font-mono text-xs text-text-muted">State</div>
        <div className="mt-1 break-all font-mono text-xs">
          {state || "(none)"}
        </div>
        <div className="mt-3 font-mono text-xs text-text-muted">Identity</div>
        <div className="mt-1 break-all font-mono text-xs">
          {identity ? identity.pubkey_base58 : "(no keypair in this browser)"}
        </div>
      </div>

      {phase.kind === "loading" && (
        <p
          className="mt-6 text-sm text-text-muted"
          data-testid="consent-status"
        >
          Loading…
        </p>
      )}

      {phase.kind === "no-identity" && (
        <div className="mt-6 rounded-md border border-error/40 bg-error/10 px-4 py-3 text-sm">
          <p className="font-medium">No keypair found in this browser.</p>
          <p className="mt-2 text-text-muted">
            Visit{" "}
            <Link to="/install" className="text-accent-primary underline">
              /install
            </Link>{" "}
            in this browser first to generate or import a keypair, then retry
            the connector install from your MCP client.
          </p>
        </div>
      )}

      {phase.kind === "ready" && (
        <div className="mt-6 flex gap-3">
          <button
            type="button"
            onClick={handleApprove}
            data-testid="consent-approve"
            className="rounded-md bg-accent-primary px-4 py-2 text-sm font-medium text-background transition-colors hover:bg-accent-primary/90"
          >
            Approve and sign
          </button>
          <button
            type="button"
            onClick={handleReject}
            data-testid="consent-reject"
            className="rounded-md border border-text-muted/30 px-4 py-2 text-sm font-medium text-text-primary transition-colors hover:border-error hover:text-error"
          >
            Reject
          </button>
        </div>
      )}

      {phase.kind === "signing" && (
        <p
          className="mt-6 text-sm text-text-muted"
          data-testid="consent-status"
        >
          Signing challenge in WASM…
        </p>
      )}

      {phase.kind === "redirecting" && (
        <p
          className="mt-6 text-sm text-text-muted"
          data-testid="consent-status"
        >
          Redirecting to your MCP client…{" "}
          <a href={phase.redirectUrl} className="text-accent-primary underline">
            Continue
          </a>
        </p>
      )}

      {phase.kind === "error" && (
        <div
          className="mt-6 rounded-md border border-error/40 bg-error/10 px-4 py-3 text-sm"
          data-testid="consent-error"
        >
          <p className="font-medium">Failed</p>
          <p className="mt-2 break-words font-mono text-xs">{phase.message}</p>
        </div>
      )}
    </main>
  );
}

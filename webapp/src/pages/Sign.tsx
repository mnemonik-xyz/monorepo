import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { Decoder } from "cbor-x";
import ContentPreview from "../components/ContentPreview";
import {
  decodeJwtPayload,
  readIdentity,
  readJwt,
  type KeypairJson,
} from "../lib/storage";
import { loadWasm } from "../lib/wasm";

/**
 * Sign-approval page (`/sign/:correlationId`).
 *
 * Flow:
 *   1. Read JWT from localStorage. If missing, redirect to `/install`.
 *      (Chicken-and-egg note: in production the AI tool's OAuth flow should
 *       have already issued the JWT before redirecting the user here. For
 *       hackathon scope we treat "no JWT" as an unrecoverable error and send
 *       the user back to `/install` so they can complete OAuth — see Decision 4.)
 *   2. GET /api/pending/<id> with Authorization: Bearer <jwt>.
 *   3. Decode the canonical-CBOR body to a readable string preview.
 *      The server attaches `x-mnemonic-content-hash` and an `expires_at`
 *      header; we use those for the integrity check + countdown.
 *   4. On Sign: load identity, call WASM `sign_attestation_bundle`, POST
 *      `/api/sign-callback` with `{correlation_id, cose_signed_bytes (b64),
 *       signer_pubkey}`.
 *   5. On Reject: do not POST — show local "rejected" state. The bundle
 *      ages out server-side at the TTL.
 *
 * MCP backend host: `https://mcp.mnemonik.xyz` per Decision 5. Browser CSP
 * `connect-src` allows this origin.
 */

// Configurable for e2e tests + local dev. Override via Vite env:
// `VITE_MCP_BASE=http://localhost:3000 npm run dev` runs against a
// locally-built mnemonic-mcp release binary.
const MCP_BASE = (import.meta.env?.VITE_MCP_BASE as string | undefined) ?? "";

type Status =
  | { kind: "loading" }
  | { kind: "ready"; bundle: PendingBundle }
  | { kind: "expired"; reason: string }
  | { kind: "signing" }
  | {
      kind: "success";
      attestationId: string | null;
      solanaTx: string | null;
      arweaveTx: string | null;
      solanaExplorerUrl: string | null;
      arweaveUrl: string | null;
    }
  | { kind: "rejected" }
  | { kind: "error"; message: string };

interface PendingBundle {
  /** UTF-8 string preview decoded from the canonical-CBOR `content` field. */
  content: string;
  /** Raw canonical-CBOR bytes returned by the server. */
  cborBytes: Uint8Array;
  /** Hex blake3 hash announced by the server header. */
  contentHash: string;
  /** Unix epoch milliseconds when the server will evict the bundle. */
  expiresAtMs: number;
  /** Embedding bytes — server-supplied for the signing call. */
  embeddingBytes: Uint8Array;
}

/** UUID v4 shape — used to validate the path param before hitting the API. */
const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export default function Sign() {
  const { correlationId } = useParams<{ correlationId: string }>();
  const navigate = useNavigate();

  const [status, setStatus] = useState<Status>({ kind: "loading" });
  const [now, setNow] = useState<number>(() => Date.now());
  const tickRef = useRef<number | null>(null);

  // Pre-Decision-12-fix the page required a JWT in localStorage. That broke
  // the cross-tool flow: the AI tool (VS Code/Cursor/Claude.ai) holds the
  // JWT issued by /oauth/token, but the user's browser DOES NOT — auth
  // contexts are separate. Now /api/* endpoints accept the correlation_id
  // as a capability and rely on COSE signature verification for security
  // (see api.rs::sign_callback_handler doc comment).
  //
  // We still read JWT opportunistically so the DID display works when the
  // user authenticated to the webapp directly (not through an AI tool).
  const jwt = useMemo(() => readJwt(), []);
  const did = useMemo(() => {
    if (!jwt) return null;
    const payload = decodeJwtPayload(jwt);
    if (!payload) return null;
    const sub = typeof payload.sub === "string" ? payload.sub : null;
    return sub ? `did:sol:${sub}` : null;
  }, [jwt]);

  // Redirect to /install ONLY if the user has no keypair AT ALL — without
  // a keypair there is nothing to sign with. With a keypair, proceed even
  // without a JWT (cross-tool case: VS Code holds the JWT, browser holds
  // the keypair).
  useEffect(() => {
    if (
      typeof localStorage !== "undefined" &&
      !localStorage.getItem("mnemonic.identity")
    ) {
      navigate("/install", { replace: true });
    }
  }, [navigate]);

  // Validate correlationId shape before fetching.
  const validCorrelation = correlationId && UUID_RE.test(correlationId);

  // Initial fetch of the pending bundle. JWT is optional — if present we
  // include it (server still accepts), otherwise we use capability-based
  // auth via correlation_id alone.
  useEffect(() => {
    if (!validCorrelation || !correlationId) return;

    let cancelled = false;
    (async () => {
      try {
        const headers: Record<string, string> = { Accept: "application/cbor" };
        if (jwt) headers.Authorization = `Bearer ${jwt}`;
        const res = await fetch(
          `${MCP_BASE}/api/pending/${encodeURIComponent(correlationId)}`,
          { method: "GET", headers },
        );

        if (cancelled) return;

        if (res.status === 410 || res.status === 404) {
          setStatus({
            kind: "expired",
            reason:
              res.status === 410
                ? "Bundle expired or already signed."
                : "Bundle not found. It may have been evicted.",
          });
          return;
        }
        if (!res.ok) {
          setStatus({
            kind: "error",
            message: `Failed to load bundle (HTTP ${res.status}).`,
          });
          return;
        }

        const buf = new Uint8Array(await res.arrayBuffer());
        const contentHash = res.headers.get("x-mnemonic-content-hash") ?? "";
        const expiresAtHeader = res.headers.get("x-mnemonic-expires-at");
        const expiresAtMs = expiresAtHeader
          ? Number.parseInt(expiresAtHeader, 10) * 1000
          : Date.now() + 5 * 60 * 1000;

        const content = decodeContentFromCbor(buf);
        const embeddingBytes = decodeEmbeddingFromCbor(buf);

        if (cancelled) return;
        setStatus({
          kind: "ready",
          bundle: {
            content,
            cborBytes: buf,
            contentHash,
            expiresAtMs,
            embeddingBytes,
          },
        });
      } catch (e) {
        if (cancelled) return;
        setStatus({ kind: "error", message: formatError(e) });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [correlationId, jwt, validCorrelation]);

  // Countdown tick — recompute every second based on wall-clock.
  useEffect(() => {
    tickRef.current = window.setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => {
      if (tickRef.current !== null) {
        window.clearInterval(tickRef.current);
        tickRef.current = null;
      }
    };
  }, []);

  const handleSign = useCallback(async () => {
    if (status.kind !== "ready") return;
    const identity: KeypairJson | null = readIdentity();
    // JWT is optional — capability via correlation_id + keypair-signed COSE
    // is the auth path for cross-tool flow (see api.rs::sign_callback_handler).
    if (!identity || !correlationId) return;

    setStatus({ kind: "signing" });

    try {
      const wasm = await loadWasm();
      // Sign the SERVER-PROVIDED canonical-CBOR bytes verbatim. Using
      // sign_attestation_bundle here produces different bytes (subset of
      // metadata fields) and verify_artifact fails content_integrity on
      // the server. sign_cose_payload wraps the exact bytes in COSE_Sign1.
      const cose = wasm.sign_cose_payload(
        status.bundle.cborBytes,
        identity,
      ) as Uint8Array;

      const cose_signed_bytes_b64 = uint8ToBase64(cose);

      const cbHeaders: Record<string, string> = {
        "Content-Type": "application/json",
      };
      if (jwt) cbHeaders.Authorization = `Bearer ${jwt}`;
      const res = await fetch(`${MCP_BASE}/api/sign-callback`, {
        method: "POST",
        headers: cbHeaders,
        body: JSON.stringify({
          correlation_id: correlationId,
          cose_signed_bytes: cose_signed_bytes_b64,
          signer_pubkey: identity.pubkey_base58,
        }),
      });

      if (res.ok) {
        const body = await res.json().catch(() => null);
        const attestationId =
          body && typeof body.attestation_id === "string"
            ? body.attestation_id
            : null;
        const solanaTx =
          body && typeof body.solana_tx === "string" ? body.solana_tx : null;
        const arweaveTx =
          body && typeof body.arweave_tx === "string" ? body.arweave_tx : null;
        const solanaExplorerUrl =
          body &&
          typeof body.solana_explorer_url === "string" &&
          body.solana_explorer_url.length > 0
            ? body.solana_explorer_url
            : null;
        const arweaveUrl =
          body &&
          typeof body.arweave_url === "string" &&
          body.arweave_url.length > 0
            ? body.arweave_url
            : null;
        setStatus({
          kind: "success",
          attestationId,
          solanaTx,
          arweaveTx,
          solanaExplorerUrl,
          arweaveUrl,
        });
        return;
      }
      if (res.status === 410) {
        setStatus({
          kind: "expired",
          reason: "Bundle was already signed or expired.",
        });
        return;
      }
      let detail = `HTTP ${res.status}`;
      try {
        const body = await res.json();
        if (body && typeof body.error === "string") detail = body.error;
      } catch {
        // ignore parse error
      }
      setStatus({ kind: "error", message: `Sign failed: ${detail}` });
    } catch (e) {
      setStatus({ kind: "error", message: formatError(e) });
    }
  }, [correlationId, jwt, status]);

  const handleReject = () => {
    setStatus({ kind: "rejected" });
  };

  // Compute mm:ss for the countdown when we have an active bundle.
  const expiresAtMs =
    status.kind === "ready"
      ? status.bundle.expiresAtMs
      : status.kind === "signing"
        ? null
        : null;
  const remainingMs = expiresAtMs !== null ? Math.max(0, expiresAtMs - now) : 0;
  const remainingLabel = formatMmSs(remainingMs);

  // Auto-flip to expired state when the timer hits zero.
  useEffect(() => {
    if (status.kind === "ready" && remainingMs === 0) {
      setStatus({
        kind: "expired",
        reason: "Bundle expired. Please retry from the AI tool.",
      });
    }
  }, [remainingMs, status]);

  if (!validCorrelation) {
    return (
      <ErrorShell title="Invalid sign URL">
        The correlation id in the URL is malformed.
      </ErrorShell>
    );
  }

  return (
    <main className="min-h-screen px-4 py-10">
      <div className="mx-auto max-w-2xl space-y-6">
        <header className="space-y-2">
          <Link
            to="/install"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            ← Cancel
          </Link>
          <h1 className="text-2xl font-bold tracking-tight text-text-primary">
            Approve attestation
          </h1>
          {did && (
            <p className="font-mono text-xs text-text-muted">
              Signing as <span className="text-accent-primary">{did}</span>
            </p>
          )}
        </header>

        {status.kind === "loading" && (
          <p className="text-sm text-text-muted" data-testid="sign-loading">
            Loading bundle...
          </p>
        )}

        {status.kind === "ready" && (
          <>
            <ContentPreview
              content={status.bundle.content}
              cborByteLength={status.bundle.cborBytes.byteLength}
            />
            <p
              className="font-mono text-sm text-text-muted"
              data-testid="sign-countdown"
              aria-live="polite"
            >
              Expires in {remainingLabel}
            </p>
            <div className="flex flex-wrap gap-3">
              <button
                type="button"
                onClick={handleSign}
                className="rounded-md bg-accent-primary px-5 py-2.5 text-sm font-semibold text-background transition-opacity hover:opacity-90"
                data-testid="sign-approve"
              >
                Sign with my Mnemonic identity
              </button>
              <button
                type="button"
                onClick={handleReject}
                className="rounded-md border border-text-muted/30 px-5 py-2.5 text-sm text-text-primary transition-colors hover:border-error hover:text-error"
                data-testid="sign-reject"
              >
                Reject
              </button>
            </div>
          </>
        )}

        {status.kind === "signing" && (
          <p className="text-sm text-text-muted" data-testid="sign-signing">
            Signing memory...
          </p>
        )}

        {status.kind === "success" && (
          <div
            className="space-y-3 rounded-md border border-success/30 bg-success/10 p-4 text-sm text-success"
            role="status"
            data-testid="sign-success"
          >
            <div className="font-medium">
              Saved. Return to your AI tool and ask it to recall — or follow the
              on-chain trail below.
            </div>

            {status.attestationId && (
              <div
                className="font-mono text-xs break-all"
                data-testid="sign-success-attestation-id"
              >
                attestation_id: {status.attestationId}
              </div>
            )}

            {status.solanaTx && (
              <div
                className="font-mono text-xs break-all"
                data-testid="sign-success-solana-tx"
              >
                solana_tx: {status.solanaTx}
              </div>
            )}

            {status.arweaveTx && (
              <div
                className="font-mono text-xs break-all"
                data-testid="sign-success-arweave-tx"
              >
                arweave_tx: {status.arweaveTx}
              </div>
            )}

            {(status.solanaExplorerUrl || status.arweaveUrl) && (
              <div className="flex flex-wrap gap-2 pt-2">
                {status.solanaExplorerUrl && (
                  <a
                    href={status.solanaExplorerUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="rounded border border-success/40 bg-success/20 px-3 py-1 text-xs font-medium hover:border-success/70 hover:bg-success/30"
                    data-testid="sign-success-solscan-link"
                  >
                    View on Solscan
                  </a>
                )}
                {status.arweaveUrl && (
                  <a
                    href={status.arweaveUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="rounded border border-success/40 bg-success/20 px-3 py-1 text-xs font-medium hover:border-success/70 hover:bg-success/30"
                    data-testid="sign-success-arweave-link"
                  >
                    View on Arweave
                  </a>
                )}
              </div>
            )}
          </div>
        )}

        {status.kind === "rejected" && (
          <div
            className="rounded-md border border-text-muted/30 bg-white/5 p-4 text-sm text-text-muted"
            role="status"
            data-testid="sign-rejected"
          >
            Rejected. The bundle will be evicted server-side at TTL. Return to
            your AI tool.
          </div>
        )}

        {status.kind === "expired" && (
          <div
            className="rounded-md border border-text-muted/30 bg-white/5 p-4 text-sm text-text-muted"
            role="status"
            data-testid="sign-expired"
          >
            {status.reason}
          </div>
        )}

        {status.kind === "error" && (
          <div
            className="rounded-md border border-error/30 bg-error/10 p-4 text-sm text-error"
            role="alert"
            data-testid="sign-error"
          >
            {status.message}
          </div>
        )}
      </div>
    </main>
  );
}

/**
 * Shared CBOR decoder. The server emits canonical CBOR via
 * `mnemonic_core::codec::canonical::to_canonical_cbor` — keys sorted, no
 * tags. `cbor-x`'s default `Decoder` reads that without configuration.
 */
const cborDecoder = new Decoder();

/**
 * Decode the canonical-CBOR memory artifact into a plain JS object.
 *
 * Schema (MEMORY_V1):
 *   {
 *     artifact_id: string,
 *     type: "memory",
 *     schema_version: 1,
 *     content: string,            // user-visible text
 *     producer: string,           // "did:sol:<base58>"
 *     created_at: string,         // RFC3339
 *     metadata?: { embedding_compressed?: base64 string, ... },
 *     parents?: string[],
 *     tags?: string[],
 *   }
 *
 * Throws on malformed CBOR — callers should treat that as a bundle-load
 * failure (the server should have produced canonical CBOR; if we can't
 * parse it the bundle is corrupt).
 */
function decodeArtifactFromCbor(buf: Uint8Array): Record<string, unknown> {
  // Decoder.decode wants a Buffer in node and a Uint8Array in the browser.
  // The browser path is what we need — pass the underlying bytes directly.
  const decoded = cborDecoder.decode(buf) as unknown;
  if (!decoded || typeof decoded !== "object") {
    throw new Error("canonical CBOR did not decode to an object");
  }
  return decoded as Record<string, unknown>;
}

/**
 * Extract the user-visible `content` field from the decoded artifact.
 *
 * Replaces the previous regex heuristic (T10 #2 / T11 #4): a malicious or
 * compromised server could craft canonical CBOR where a benign-looking
 * string appears earlier in the byte stream than the actual `content`
 * field, and the heuristic would surface the decoy to the user — who
 * would then sign the malicious payload. A real CBOR decoder reads the
 * structured `content` value verbatim.
 */
function decodeContentFromCbor(buf: Uint8Array): string {
  try {
    const artifact = decodeArtifactFromCbor(buf);
    const content = artifact.content;
    if (typeof content === "string") return content;
    return `[bundle: ${buf.byteLength} bytes — content field missing or non-string]`;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return `[bundle: ${buf.byteLength} bytes — decode failed: ${msg}]`;
  }
}

/**
 * Decode and base64-decode the TurboQuant-compressed embedding bytes from
 * `metadata.embedding_compressed`. Returns an empty Uint8Array if the
 * field is missing — the server's sign-callback validator is the source
 * of truth on whether the resulting COSE_Sign1 verifies, so a missing
 * embedding will surface as a 401 there rather than silently passing.
 */
function decodeEmbeddingFromCbor(buf: Uint8Array): Uint8Array {
  try {
    const artifact = decodeArtifactFromCbor(buf);
    const metadata = artifact.metadata;
    if (metadata && typeof metadata === "object") {
      const ec = (metadata as Record<string, unknown>).embedding_compressed;
      if (typeof ec === "string") return base64ToUint8(ec);
      if (ec instanceof Uint8Array) return ec;
    }
    return new Uint8Array(0);
  } catch {
    return new Uint8Array(0);
  }
}

/** base64 (standard alphabet, padded) → Uint8Array. Browser-safe. */
function base64ToUint8(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function formatMmSs(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const mm = Math.floor(totalSec / 60)
    .toString()
    .padStart(2, "0");
  const ss = (totalSec % 60).toString().padStart(2, "0");
  return `${mm}:${ss}`;
}

function uint8ToBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]!);
  return btoa(s);
}

function formatError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return "unknown error";
  }
}

// Test-only re-exports — internal helpers that vitest exercises directly.
// Prefixed with `__test__` to discourage accidental production use.
export const __test__decodeContentFromCbor = decodeContentFromCbor;
export const __test__decodeEmbeddingFromCbor = decodeEmbeddingFromCbor;

function ErrorShell({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center px-4">
      <div className="max-w-md space-y-3 text-center">
        <h1 className="text-xl font-semibold text-text-primary">{title}</h1>
        <p className="text-sm text-text-muted">{children}</p>
        <Link
          to="/install"
          className="inline-block rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary"
        >
          Back to install
        </Link>
      </div>
    </main>
  );
}

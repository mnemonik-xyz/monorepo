import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
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

const MCP_BASE = "https://mcp.mnemonik.xyz";

type Status =
  | { kind: "loading" }
  | { kind: "ready"; bundle: PendingBundle }
  | { kind: "expired"; reason: string }
  | { kind: "signing" }
  | { kind: "success"; attestationId: string | null }
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

  const jwt = useMemo(() => readJwt(), []);
  const did = useMemo(() => {
    if (!jwt) return null;
    const payload = decodeJwtPayload(jwt);
    if (!payload) return null;
    const sub = typeof payload.sub === "string" ? payload.sub : null;
    return sub ? `did:sol:${sub}` : null;
  }, [jwt]);

  // Redirect to /install if there is no JWT in localStorage.
  useEffect(() => {
    if (!jwt) {
      navigate("/install", { replace: true });
    }
  }, [jwt, navigate]);

  // Validate correlationId shape before fetching.
  const validCorrelation = correlationId && UUID_RE.test(correlationId);

  // Initial fetch of the pending bundle.
  useEffect(() => {
    if (!jwt || !validCorrelation || !correlationId) return;

    let cancelled = false;
    (async () => {
      try {
        const res = await fetch(
          `${MCP_BASE}/api/pending/${encodeURIComponent(correlationId)}`,
          {
            method: "GET",
            headers: {
              Authorization: `Bearer ${jwt}`,
              Accept: "application/cbor",
            },
          }
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
    if (!identity || !jwt || !correlationId) return;

    setStatus({ kind: "signing" });

    try {
      const wasm = await loadWasm();
      const cose = wasm.sign_attestation_bundle(
        status.bundle.content,
        status.bundle.embeddingBytes,
        status.bundle.contentHash,
        identity.pubkey_base58,
        identity
      ) as Uint8Array;

      const cose_signed_bytes_b64 = uint8ToBase64(cose);

      const res = await fetch(`${MCP_BASE}/api/sign-callback`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${jwt}`,
          "Content-Type": "application/json",
        },
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
        setStatus({ kind: "success", attestationId });
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
          <p className="text-sm text-text-muted">Loading bundle...</p>
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
          <p className="text-sm text-text-muted">Signing memory...</p>
        )}

        {status.kind === "success" && (
          <div
            className="rounded-md border border-success/30 bg-success/10 p-4 text-sm text-success"
            role="status"
          >
            Saved to onchain memory. Return to your AI tool and ask to recall.
            {status.attestationId && (
              <div className="mt-2 font-mono text-xs">
                attestation_id: {status.attestationId}
              </div>
            )}
          </div>
        )}

        {status.kind === "rejected" && (
          <div
            className="rounded-md border border-text-muted/30 bg-white/5 p-4 text-sm text-text-muted"
            role="status"
          >
            Rejected. The bundle will be evicted server-side at TTL. Return to
            your AI tool.
          </div>
        )}

        {status.kind === "expired" && (
          <div
            className="rounded-md border border-text-muted/30 bg-white/5 p-4 text-sm text-text-muted"
            role="status"
          >
            {status.reason}
          </div>
        )}

        {status.kind === "error" && (
          <div
            className="rounded-md border border-error/30 bg-error/10 p-4 text-sm text-error"
            role="alert"
          >
            {status.message}
          </div>
        )}
      </div>
    </main>
  );
}

/**
 * Best-effort decode of the user-visible `content` field from canonical CBOR.
 *
 * Canonical CBOR with the MEMORY_V1 schema embeds the content as a UTF-8 text
 * string. Rather than pulling in a full CBOR decoder for a preview, this scans
 * for the text-string major type (0x60-0x7B) immediately following the key
 * `"content"` (text string). Best-effort: if the heuristic fails, we fall back
 * to a hex preview so the user still sees the bundle bytes.
 */
function decodeContentFromCbor(buf: Uint8Array): string {
  try {
    const text = new TextDecoder("utf-8", { fatal: false }).decode(buf);
    const marker = "content";
    const idx = text.indexOf(marker);
    if (idx >= 0) {
      // Heuristic: take the longest printable run after the marker.
      const after = text.slice(idx + marker.length);
      const run = after.match(/[\x20-\x7E\s]{8,}/);
      if (run) return run[0].trim();
    }
    return `[bundle: ${buf.byteLength} bytes — preview unavailable]`;
  } catch {
    return `[bundle: ${buf.byteLength} bytes]`;
  }
}

/**
 * Stub: extract the embedding bytes from the canonical CBOR. Today the
 * server-stored bundle includes `embedding_compressed` as a base64 string;
 * decoding it precisely requires a full CBOR parser. For Phase 1 we pass an
 * empty Uint8Array — the WASM `sign_attestation_bundle` accepts any bytes and
 * the server validates the resulting COSE_Sign1 against the originally-stored
 * canonical CBOR, so the embedding bytes the browser supplies do not change
 * the final signature outcome (the server is the source of truth on the
 * canonicalized bundle). Documented in the WASM module: see
 * `core/src/wasm/mod.rs::sign_attestation_bundle` doc comment.
 */
function decodeEmbeddingFromCbor(_buf: Uint8Array): Uint8Array {
  return new Uint8Array(0);
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

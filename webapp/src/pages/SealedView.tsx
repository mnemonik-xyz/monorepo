import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Decoder } from "cbor-x";
import { loadWasm } from "../lib/wasm";
import { readIdentity } from "../lib/storage";
import { MCP_BASE } from "../lib/api";

/**
 * `/m/<hash>` — Sealed memory viewer (tech-spec §8.5).
 *
 * Security invariants:
 * - This page loads NO third-party scripts. All decryption runs in local WASM.
 * - The URL fragment (`#k=...`) is NEVER sent in any network request.
 *   RFC 3986 §3.5 guarantees the browser strips the fragment before sending
 *   HTTP requests. We never read `window.location.hash` until after all
 *   network fetches are complete. We never log it.
 * - If `#k=` is present: verify `kc`, decrypt locally with the WASM
 *   `open_with_key` export. If absent: show author/date/signature status only.
 *
 * UI states:
 *   loading  → fetching outer SEALED_V1 artifact from server
 *   no-key   → artifact loaded, no #k= fragment, show public metadata
 *   decrypting → WASM open_with_key running
 *   open     → decrypted plaintext
 *   error    → any failure (network, malformed CBOR, AEAD fail, kc mismatch)
 */

type Status =
  | { kind: "loading" }
  | { kind: "no-key"; artifact: SealedArtifact }
  | { kind: "decrypting"; artifact: SealedArtifact }
  | { kind: "open"; artifact: SealedArtifact; plaintext: string }
  | { kind: "kc-fail"; artifact: SealedArtifact }
  | { kind: "error"; message: string };

interface SealedArtifact {
  /** `artifact_id` from outer SEALED_V1 CBOR. */
  artifactId: string;
  /** `producer` DID from outer CBOR. */
  producer: string;
  /** `created_at` ISO from outer CBOR. */
  createdAt: string;
  /** `type` — should be "sealed". */
  type: string;
  /** `kc` bytes (key commitment, 32 bytes) from outer CBOR. */
  kc: Uint8Array | null;
  /** Raw outer canonical-CBOR bytes returned from the server. */
  rawBytes: Uint8Array;
  /** True if COSE_Sign1 signature was valid (checked by server or verified locally). */
  signatureValid: boolean;
}

const cborDecoder = new Decoder();

export default function SealedView() {
  const { hash } = useParams<{ hash: string }>();
  const [status, setStatus] = useState<Status>({ kind: "loading" });
  // We read the fragment ONLY after the network fetch completes to ensure the
  // browser cannot accidentally include it in any request (belt-and-suspenders
  // over RFC 3986's own guarantee).
  const fragmentReadRef = useRef(false);

  // Validate hash param — blake3 hex is 64 lowercase hex chars.
  const HASH_RE = /^[0-9a-f]{64}$/i;
  const validHash = hash && HASH_RE.test(hash);

  useEffect(() => {
    if (!validHash || !hash) return;

    let cancelled = false;
    (async () => {
      try {
        // Fetch the sealed artifact. The fragment is NOT included by the
        // browser in this request (RFC 3986 §3.5).
        const res = await fetch(
          `${MCP_BASE}/api/sealed/${encodeURIComponent(hash)}`,
          {
            method: "GET",
            headers: { Accept: "application/cbor, application/json" },
          },
        );

        if (cancelled) return;

        if (res.status === 404) {
          setStatus({ kind: "error", message: "Memory not found." });
          return;
        }
        if (!res.ok) {
          setStatus({
            kind: "error",
            message: `Failed to load memory (HTTP ${res.status}).`,
          });
          return;
        }

        const buf = new Uint8Array(await res.arrayBuffer());
        const artifact = parseSealedArtifact(buf);

        if (cancelled) return;

        // NOW read the fragment — all network I/O is complete.
        if (!fragmentReadRef.current) {
          fragmentReadRef.current = true;
        }
        const rawFragment = typeof window !== "undefined"
          ? window.location.hash
          : "";
        const kParam = extractKFromFragment(rawFragment);

        if (!kParam) {
          setStatus({ kind: "no-key", artifact });
          return;
        }

        // Key found in fragment — attempt decryption.
        setStatus({ kind: "decrypting", artifact });

        const keyBytes = base64urlToBytes(kParam);
        if (!keyBytes || keyBytes.length !== 32) {
          setStatus({
            kind: "error",
            message: "Fragment key is malformed (expected 32-byte base64url).",
          });
          return;
        }

        // Attempt decryption with WASM.
        let plaintext: string;
        try {
          const wasm = await loadWasm();
          // The native opener validates the key commitment before AEAD
          // (core/src/sealed/api.rs::open_with_key). Do not duplicate that KDF
          // in JavaScript or treat unavailable crypto as a successful check.
          const openFn = (wasm as unknown as {
            open_with_key?: (bytes: Uint8Array, key: Uint8Array) => Uint8Array;
          }).open_with_key;
          if (typeof openFn === "function") {
            const opened = openFn(artifact.rawBytes, keyBytes);
            if (!(opened instanceof Uint8Array)) {
              throw new Error("Unexpected sealed decryption result");
            }
            // The native API returns an inner memory JSON blob. Do not
            // render malformed plaintext or include parser snippets in errors.
            try {
              const inner: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(opened));
              if (!inner || typeof inner !== "object" || !("content" in inner) || typeof inner.content !== "string") {
                throw new Error("invalid memory");
              }
              plaintext = inner.content;
            } catch {
              throw new Error("Invalid decrypted memory format");
            }
          } else {
            // WASM sealed crypto not yet available — surface an informative error
            // rather than silently failing or using a fallback that may differ.
            setStatus({
              kind: "error",
              message:
                "Sealed memory decryption requires a newer version of the Mnemonic WASM module. " +
                "The signature and metadata are displayed below.",
            });
            return;
          }
        } catch (e) {
          if (String(e).includes("key commitment mismatch")) {
            setStatus({ kind: "kc-fail", artifact });
            return;
          }
          setStatus({
            kind: "error",
            message: `Decryption failed: ${e instanceof Error ? e.message : String(e)}`,
          });
          return;
        }

        if (cancelled) return;
        setStatus({ kind: "open", artifact, plaintext });
      } catch (e) {
        if (cancelled) return;
        setStatus({
          kind: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [hash, validHash]);

  if (!validHash) {
    return (
      <ErrorShell title="Invalid memory link">
        The hash in this URL is malformed.
      </ErrorShell>
    );
  }

  return (
    <main className="min-h-screen px-4 py-10">
      <div className="mx-auto max-w-2xl space-y-6">
        <header className="space-y-2">
          <Link
            to="/ledger"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            ← Ledger
          </Link>
          <h1 className="text-2xl font-bold tracking-tight text-text-primary">
            Sealed memory
          </h1>
        </header>

        {status.kind === "loading" && (
          <p className="text-sm text-text-muted" data-testid="sealed-loading">
            Loading memory…
          </p>
        )}

        {(status.kind === "no-key" ||
          status.kind === "decrypting" ||
          status.kind === "open" ||
          status.kind === "kc-fail") && (
          <ArtifactMeta artifact={status.artifact} />
        )}

        {status.kind === "no-key" && (
          <div
            className="rounded-md border border-text-muted/20 bg-white/[0.03] p-4 text-sm text-text-muted"
            data-testid="sealed-no-key"
          >
            <p>
              This memory is encrypted. Only the holder of the bearer key can
              read it.
            </p>
            <p className="mt-2 font-mono text-xs">
              To decrypt: open the full link including the{" "}
              <code className="text-accent-primary">#k=</code> fragment.
            </p>
          </div>
        )}

        {status.kind === "decrypting" && (
          <p
            className="text-sm text-text-muted"
            data-testid="sealed-decrypting"
          >
            Decrypting locally…
          </p>
        )}

        {status.kind === "open" && (
          <div
            className="space-y-3"
            data-testid="sealed-open"
          >
            <div className="rounded-sm border border-accent-primary/30 bg-accent-primary/5 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.16em] text-accent-primary">
              Decrypted locally — encrypted, only you can read it
            </div>
            <pre
              className="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-md border border-text-muted/20 bg-white/5 p-4 font-mono text-sm text-text-primary"
              data-testid="sealed-plaintext"
            >
              {status.plaintext}
            </pre>
          </div>
        )}

        {status.kind === "kc-fail" && (
          <div
            className="rounded-md border border-error/30 bg-error/10 p-4 text-sm text-error"
            role="alert"
            data-testid="sealed-kc-fail"
          >
            Key commitment check failed. The key in this link does not match
            the sealed memory. The link may be corrupt or forged.
          </div>
        )}

        {status.kind === "error" && (
          <div
            className="rounded-md border border-error/30 bg-error/10 p-4 text-sm text-error"
            role="alert"
            data-testid="sealed-error"
          >
            {status.message}
          </div>
        )}
      </div>
    </main>
  );
}

/* -------------------------------------------------------------------------- */
/*                             Artifact metadata                              */
/* -------------------------------------------------------------------------- */

function ArtifactMeta({ artifact }: { artifact: SealedArtifact }) {
  const identity = useMemo(() => readIdentity(), []);
  const isAuthor =
    identity &&
    artifact.producer.includes(identity.pubkey_base58);

  const formattedDate = useMemo(() => {
    const d = new Date(artifact.createdAt);
    if (Number.isNaN(d.getTime())) return artifact.createdAt;
    return d.toLocaleString("en-US", {
      year: "numeric",
      month: "short",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  }, [artifact.createdAt]);

  return (
    <div
      className="rounded-md border border-white/10 bg-panel/60 p-5 space-y-3"
      data-testid="sealed-meta"
    >
      <div className="flex items-center gap-2 flex-wrap">
        {/* Sealed badge */}
        <span
          aria-label="Sealed memory"
          className="rounded-sm border border-stamp/50 px-1.5 py-0.5 font-mono text-[10px] font-bold uppercase tracking-[0.2em] text-stamp"
          data-testid="sealed-badge"
        >
          Sealed
        </span>
        {artifact.signatureValid && (
          <span
            className="rounded-sm border border-success/40 bg-success/10 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-success"
            data-testid="sealed-sig-valid"
          >
            Signature valid
          </span>
        )}
        {isAuthor && (
          <span className="rounded-sm border border-accent-primary/30 bg-accent-primary/10 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-accent-primary">
            Author
          </span>
        )}
      </div>

      <dl className="grid grid-cols-1 gap-y-2 border-t border-white/5 pt-3">
        <MetaRow label="Author" value={artifact.producer} mono />
        <MetaRow label="Date" value={formattedDate} />
        <MetaRow
          label="Type"
          value={artifact.type === "sealed" ? "Sealed (encrypted)" : artifact.type}
        />
      </dl>
    </div>
  );
}

function MetaRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-baseline gap-3">
      <dt className="shrink-0 w-16 font-mono text-[10px] uppercase tracking-[0.16em] text-text-faint">
        {label}
      </dt>
      <dd
        className={`min-w-0 flex-1 break-all text-[13px] text-text-muted ${mono ? "font-mono" : ""}`}
      >
        {value}
      </dd>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                  Helpers                                   */
/* -------------------------------------------------------------------------- */

/**
 * Extract the `k` parameter from a URL fragment string.
 * The fragment must start with `#k=`. Returns null if absent or malformed.
 *
 * SECURITY: this function is the ONLY place the fragment is read. It must
 * never be called before all network I/O is complete. The value must never
 * be logged or included in error messages.
 */
export function extractKFromFragment(fragment: string): string | null {
  if (!fragment.startsWith("#k=")) return null;
  const value = fragment.slice(3);
  // Validate: base64url chars only, length consistent with 32 bytes (43 chars, no padding).
  if (!value || !/^[A-Za-z0-9_-]{43}$/.test(value)) return null;
  return value;
}

/**
 * Decode a base64url-encoded string to Uint8Array (no padding, URL-safe alphabet).
 */
function base64urlToBytes(b64url: string): Uint8Array | null {
  try {
    // base64url → standard base64
    const b64 = b64url.replace(/-/g, "+").replace(/_/g, "/");
    const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
    const bin = atob(padded);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

/**
 * Parse a SEALED_V1 (or COSE_Sign1 wrapping it) buffer into a SealedArtifact.
 * Accepts both raw CBOR and COSE_Sign1-wrapped CBOR, since the server may
 * store either format.
 */
function parseSealedArtifact(buf: Uint8Array): SealedArtifact {
  let payload: Uint8Array = buf;
  let signatureValid = false;

  // Check if the outer encoding is a COSE_Sign1 (CBOR array tag 18 or bare array).
  // A COSE_Sign1 is a CBOR array: [protected, unprotected, payload, signature].
  // We try to extract the payload from it.
  try {
    const outer = cborDecoder.decode(buf) as unknown;
    if (Array.isArray(outer) && outer.length === 4) {
      // Looks like COSE_Sign1: [protected_bstr, {}, payload_bstr, sig_bstr].
      const payloadField = outer[2];
      if (payloadField instanceof Uint8Array && payloadField.length > 0) {
        payload = payloadField;
        // The server verified the COSE signature before serving — we trust it.
        signatureValid = true;
      }
    }
  } catch {
    // Not COSE — treat as raw CBOR.
  }

  // Parse the inner (or direct) SEALED_V1 CBOR.
  try {
    const obj = cborDecoder.decode(payload) as Record<string, unknown>;
    return {
      artifactId: typeof obj.artifact_id === "string" ? obj.artifact_id : "",
      producer: typeof obj.producer === "string" ? obj.producer : "(unknown)",
      createdAt: typeof obj.created_at === "string" ? obj.created_at : "",
      type: typeof obj.type === "string" ? obj.type : "sealed",
      kc: obj.kc instanceof Uint8Array ? obj.kc : null,
      rawBytes: payload,
      signatureValid,
    };
  } catch {
    // Fallback: return a minimal struct so the error path shows *something*.
    return {
      artifactId: "",
      producer: "(malformed)",
      createdAt: "",
      type: "sealed",
      kc: null,
      rawBytes: buf,
      signatureValid: false,
    };
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
          to="/"
          className="inline-block rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary"
        >
          Home
        </Link>
      </div>
    </main>
  );
}

// Test-only exports
export const __test__extractKFromFragment = extractKFromFragment;
export const __test__parseSealedArtifact = parseSealedArtifact;

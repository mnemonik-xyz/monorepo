import { useCallback, useState } from "react";
import { readIdentity } from "../lib/storage";
import { loadWasm } from "../lib/wasm";
import { MCP_BASE } from "../lib/api";

/**
 * ShareDialog — grant access to a sealed memory (tech-spec §8.4, §8.5).
 *
 * Three grant modes:
 *   1. DID / AgentCard / X25519 key  → G1 server grant (`POST /api/grants`)
 *   2. Bearer link (`#k=...`)        → copies link to clipboard, no server call
 *   3. (anonymous/Arweave G2 — planned; out of scope for task 8)
 *
 * Security invariants:
 * - The server NEVER receives K or the plaintext. The browser unwraps K from
 *   the author's own wrap, then re-wraps it to the recipient (tech-spec §8.4).
 * - No third-party scripts are loaded.
 * - The no-revocation warning (tech-spec §5.5) is shown and must be confirmed
 *   before granting.
 *
 * The component is a modal-style panel. Pass `onClose` to dismiss it.
 */

type GrantMode = "did" | "link";
type Phase =
  | { kind: "choose" }
  | { kind: "warn"; mode: GrantMode; recipient: string }
  | { kind: "granting" }
  | { kind: "done"; result: GrantResult }
  | { kind: "error"; message: string };

interface GrantResult {
  mode: GrantMode;
  /** For DID mode: the grant id from the server. For link mode: the URL. */
  value: string;
}

export interface ShareDialogProps {
  /** blake3 hex hash of the sealed memory. */
  memoryHash: string;
  /** Raw outer SEALED_V1 CBOR bytes — needed to unwrap K from the author wrap. */
  sealedBytes: Uint8Array;
  onClose: () => void;
}

export default function ShareDialog({
  memoryHash,
  sealedBytes,
  onClose,
}: ShareDialogProps) {
  const [mode, setMode] = useState<GrantMode>("did");
  const [recipient, setRecipient] = useState("");
  const [phase, setPhase] = useState<Phase>({ kind: "choose" });
  const [linkCopied, setLinkCopied] = useState(false);

  const handleProceed = useCallback(() => {
    const trimmed = recipient.trim();
    if (mode === "did" && !trimmed) return;
    // Show the no-revocation warning before confirming.
    setPhase({ kind: "warn", mode, recipient: trimmed });
  }, [mode, recipient]);

  const handleConfirmGrant = useCallback(async () => {
    const identity = readIdentity();
    if (!identity) {
      setPhase({
        kind: "error",
        message: "No keypair found in this browser. Visit /install first.",
      });
      return;
    }

    const currentPhase = phase;
    if (currentPhase.kind !== "warn") return;
    const { mode: grantMode, recipient: grantRecipient } = currentPhase;

    setPhase({ kind: "granting" });

    try {
      const wasm = await loadWasm();

      // Unwrap K from the sealed bundle using the author's X25519 key.
      // WASM export `unwrap_content_key(sealed_cbor, keypair_json)` → Uint8Array(32).
      const wasmRecord = wasm as Record<string, unknown>;
      if (typeof wasmRecord.unwrap_content_key !== "function") {
        // WASM sealed crypto not yet available (tasks 1-5 not merged).
        setPhase({
          kind: "error",
          message:
            "Sealed memory grants require a newer version of the Mnemonic WASM module. " +
            "This feature will be available once the sealed crypto tasks are merged.",
        });
        return;
      }

      const unwrapFn = wasmRecord.unwrap_content_key as (
        bytes: Uint8Array,
        keypair: unknown,
      ) => Uint8Array;
      const contentKey = unwrapFn(sealedBytes, identity);

      if (grantMode === "link") {
        // Build bearer link: fragment is not sent to any server.
        // base64url-encode the 32-byte key.
        const b64url = bytesToBase64url(contentKey);
        const url = `${window.location.origin}/m/${memoryHash}#k=${b64url}`;
        setPhase({ kind: "done", result: { mode: "link", value: url } });
        return;
      }

      // DID / X25519 key grant — re-wrap K to recipient and POST to server.
      if (typeof wasmRecord.wrap_content_key_to_recipient !== "function") {
        setPhase({
          kind: "error",
          message: "Grant wrapping not available in this WASM build.",
        });
        return;
      }
      const wrapFn = wasmRecord.wrap_content_key_to_recipient as (
        key: Uint8Array,
        recipient: string,
        memory_hash: string,
        keypair: unknown,
      ) => Uint8Array;
      const grantCbor = wrapFn(contentKey, grantRecipient, memoryHash, identity);

      // POST the signed GRANT_V1 to the server. K is never sent — only the wrap.
      const res = await fetch(`${MCP_BASE}/api/grants`, {
        method: "POST",
        headers: { "Content-Type": "application/cbor" },
        body: grantCbor,
      });

      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`Server rejected grant (HTTP ${res.status}): ${text}`);
      }

      const body = (await res.json().catch(() => ({}))) as Record<
        string,
        unknown
      >;
      const grantId =
        typeof body.grant_id === "string" ? body.grant_id : "ok";
      setPhase({ kind: "done", result: { mode: "did", value: grantId } });
    } catch (e) {
      setPhase({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [phase, memoryHash, sealedBytes]);

  const handleCopyLink = useCallback(async () => {
    if (phase.kind !== "done" || phase.result.mode !== "link") return;
    try {
      await navigator.clipboard.writeText(phase.result.value);
      setLinkCopied(true);
      setTimeout(() => setLinkCopied(false), 2000);
    } catch {
      // clipboard unavailable
    }
  }, [phase]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Share sealed memory"
      className="fixed inset-0 z-50 flex items-center justify-center bg-ink/70 px-4"
      data-testid="share-dialog"
    >
      <div className="w-full max-w-lg rounded-md border border-white/15 bg-panel shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
          <h2 className="text-base font-semibold text-text-primary">
            Share sealed memory
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close share dialog"
            className="rounded p-1 text-text-muted transition-colors hover:text-text-primary"
          >
            ✕
          </button>
        </div>

        <div className="px-5 py-4 space-y-4">
          {phase.kind === "choose" && (
            <>
              {/* Mode picker */}
              <div className="flex gap-2" role="group" aria-label="Grant mode">
                <button
                  type="button"
                  aria-pressed={mode === "did"}
                  onClick={() => setMode("did")}
                  className={`flex-1 rounded-md border px-3 py-2 text-sm transition-colors ${
                    mode === "did"
                      ? "border-accent-primary/60 bg-accent-primary/10 text-accent-primary"
                      : "border-white/10 text-text-muted hover:border-white/20"
                  }`}
                  data-testid="share-mode-did"
                >
                  DID / Key
                </button>
                <button
                  type="button"
                  aria-pressed={mode === "link"}
                  onClick={() => setMode("link")}
                  className={`flex-1 rounded-md border px-3 py-2 text-sm transition-colors ${
                    mode === "link"
                      ? "border-accent-primary/60 bg-accent-primary/10 text-accent-primary"
                      : "border-white/10 text-text-muted hover:border-white/20"
                  }`}
                  data-testid="share-mode-link"
                >
                  Bearer link
                </button>
              </div>

              {mode === "did" && (
                <div className="space-y-2">
                  <label
                    htmlFor="share-recipient"
                    className="block font-mono text-[11px] uppercase tracking-[0.14em] text-text-muted"
                  >
                    Recipient (DID, AgentCard URL, or X25519 pubkey)
                  </label>
                  <input
                    id="share-recipient"
                    type="text"
                    value={recipient}
                    onChange={(e) => setRecipient(e.target.value)}
                    placeholder="did:sol:… or AgentCard URL"
                    className="w-full rounded-md border border-white/10 bg-white/[0.03] px-3 py-2 font-mono text-sm text-text-primary placeholder:text-text-faint focus-visible:border-accent-primary focus-visible:outline-none"
                    data-testid="share-recipient-input"
                  />
                </div>
              )}

              {mode === "link" && (
                <div className="rounded-md border border-text-muted/20 bg-white/[0.03] p-3 text-sm text-text-muted">
                  <p>
                    A bearer link puts the decryption key in the URL fragment.
                    Anyone with this link can read this memory — permanently.
                  </p>
                </div>
              )}

              <button
                type="button"
                onClick={handleProceed}
                disabled={mode === "did" && !recipient.trim()}
                className="w-full rounded-md bg-accent-primary px-4 py-2.5 text-sm font-semibold text-background transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
                data-testid="share-proceed"
              >
                Continue
              </button>
            </>
          )}

          {/* No-revocation warning — tech-spec §5.5 */}
          {phase.kind === "warn" && (
            <div className="space-y-4" data-testid="share-warn">
              <div className="rounded-md border border-error/40 bg-error/10 p-4 text-sm">
                <p className="font-semibold text-error">
                  Warning: grants cannot be revoked
                </p>
                <ul className="mt-2 list-disc pl-4 space-y-1 text-text-muted">
                  <li>
                    Once the recipient has the key, they keep read access forever.
                    You cannot revoke it.
                  </li>
                  <li>
                    You can stop serving a grant from this server, but the
                    recipient may already have saved the key locally.
                  </li>
                  <li>
                    If this memory is anchored on Arweave, a grant anchored
                    there is also permanent.
                  </li>
                  {phase.mode === "link" && (
                    <li>
                      A bearer link is permanent read access for anyone who
                      receives it. Treat it like a password.
                    </li>
                  )}
                </ul>
              </div>

              {phase.mode === "did" && phase.recipient && (
                <p className="text-sm text-text-muted">
                  Granting to:{" "}
                  <code className="font-mono text-xs break-all text-text-primary">
                    {phase.recipient}
                  </code>
                </p>
              )}

              <div className="flex gap-3">
                <button
                  type="button"
                  onClick={handleConfirmGrant}
                  className="rounded-md bg-accent-primary px-4 py-2 text-sm font-semibold text-background transition-opacity hover:opacity-90"
                  data-testid="share-confirm"
                >
                  I understand — grant access
                </button>
                <button
                  type="button"
                  onClick={() => setPhase({ kind: "choose" })}
                  className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-error hover:text-error"
                  data-testid="share-cancel-warn"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}

          {phase.kind === "granting" && (
            <p className="text-sm text-text-muted" data-testid="share-granting">
              Preparing grant in WASM…
            </p>
          )}

          {phase.kind === "done" && (
            <div
              className="space-y-3 rounded-md border border-success/30 bg-success/10 p-4 text-sm text-success"
              role="status"
              data-testid="share-done"
            >
              {phase.result.mode === "link" ? (
                <>
                  <p className="font-medium">Bearer link ready.</p>
                  <p className="text-text-muted text-xs">
                    This link gives permanent read access. Keep it secret.
                  </p>
                  <div className="flex gap-2 items-center">
                    <code className="min-w-0 flex-1 break-all rounded-sm bg-black/30 px-2 py-1 font-mono text-[11px] text-text-muted">
                      {phase.result.value}
                    </code>
                    <button
                      type="button"
                      onClick={handleCopyLink}
                      className="shrink-0 rounded-sm border border-white/10 px-2 py-1 font-mono text-[10px] uppercase tracking-[0.14em] text-text-muted transition-colors hover:border-accent-primary hover:text-accent-primary"
                      data-testid="share-copy-link"
                    >
                      {linkCopied ? "Copied" : "Copy"}
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p className="font-medium">Grant recorded.</p>
                  <p className="font-mono text-xs break-all text-success/80">
                    grant_id: {phase.result.value}
                  </p>
                </>
              )}
            </div>
          )}

          {phase.kind === "error" && (
            <div
              className="rounded-md border border-error/30 bg-error/10 p-4 text-sm text-error"
              role="alert"
              data-testid="share-error"
            >
              {phase.message}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Encode Uint8Array to base64url (no padding). */
function bytesToBase64url(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]!);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
}

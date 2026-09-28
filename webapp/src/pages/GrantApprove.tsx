import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { readIdentity } from "../lib/storage";
import { loadWasm } from "../lib/wasm";
import { MCP_BASE } from "../lib/api";

/**
 * Grant approve page (`/grant/approve`).
 *
 * Handles the `approve_url` returned by `mnemonic_share` (tech-spec §8.4).
 *
 * Flow:
 *   1. The MCP server calls `mnemonic_share {memory_hash, reader}`.
 *   2. Server returns `{status: "awaiting_signature", approve_url}`.
 *   3. The AI tool redirects the user's browser here with:
 *      ?memory_hash=<hash>&reader=<did_or_key>&grant_token=<token>
 *   4. This page fetches the sealed memory bytes from `/api/sealed/<hash>`.
 *   5. In the browser: unwrap K from the author's own wrap, re-wrap to reader.
 *   6. POST the signed GRANT_V1 CBOR to `/api/grants`.
 *   7. The server never receives K — only the HPKE-wrapped K for the reader.
 *
 * Security: same as ShareDialog (§8.4). The no-revocation warning (§5.5) is
 * shown before the user confirms.
 */

type Phase =
  | { kind: "loading" }
  | { kind: "ready"; memoryHash: string; reader: string; grantToken: string }
  | { kind: "no-identity" }
  | { kind: "warn"; memoryHash: string; reader: string; grantToken: string }
  | { kind: "granting" }
  | { kind: "done" }
  | { kind: "error"; message: string };

export default function GrantApprove() {
  const [params] = useSearchParams();
  const memoryHash = params.get("memory_hash") ?? "";
  const reader = params.get("reader") ?? "";
  const grantToken = params.get("grant_token") ?? "";

  const identity = useMemo(() => readIdentity(), []);
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });

  useEffect(() => {
    if (!memoryHash || !reader) {
      setPhase({
        kind: "error",
        message:
          "Missing `memory_hash` or `reader` query parameter. Restart the share flow from your MCP client.",
      });
      return;
    }
    if (!identity) {
      setPhase({ kind: "no-identity" });
      return;
    }
    setPhase({ kind: "ready", memoryHash, reader, grantToken });
  }, [memoryHash, reader, grantToken, identity]);

  const handleProceed = useCallback(() => {
    if (phase.kind !== "ready") return;
    // Show no-revocation warning before confirming.
    // Spread phase BEFORE kind so that kind: "warn" takes precedence.
    setPhase({ ...phase, kind: "warn" });
  }, [phase]);

  const handleConfirm = useCallback(async () => {
    if (phase.kind !== "warn" || !identity) return;
    const { memoryHash: hash, reader: recipient, grantToken: token } = phase;

    setPhase({ kind: "granting" });

    try {
      // 1. Fetch the sealed memory bytes from the server.
      const res = await fetch(
        `${MCP_BASE}/api/sealed/${encodeURIComponent(hash)}`,
        {
          method: "GET",
          headers: { Accept: "application/cbor" },
        },
      );
      if (!res.ok) {
        throw new Error(`Failed to fetch sealed memory (HTTP ${res.status}).`);
      }
      const sealedBytes = new Uint8Array(await res.arrayBuffer());

      // 2. Load WASM and check for sealed crypto exports.
      const wasm = await loadWasm();
      const wasmRecord = wasm as Record<string, unknown>;

      if (
        typeof wasmRecord.unwrap_content_key !== "function" ||
        typeof wasmRecord.wrap_content_key_to_recipient !== "function"
      ) {
        throw new Error(
          "Sealed memory grants require a newer version of the Mnemonic WASM module. " +
            "This feature will be available once the sealed crypto tasks are merged.",
        );
      }

      // 3. Unwrap K from the author's own wrap.
      const unwrapFn = wasmRecord.unwrap_content_key as (
        bytes: Uint8Array,
        keypair: unknown,
      ) => Uint8Array;
      const contentKey = unwrapFn(sealedBytes, identity);

      // 4. Re-wrap K to the recipient.
      const wrapFn = wasmRecord.wrap_content_key_to_recipient as (
        key: Uint8Array,
        recipient: string,
        memory_hash: string,
        keypair: unknown,
      ) => Uint8Array;
      const grantCbor = wrapFn(contentKey, recipient, hash, identity);

      // 5. POST the signed GRANT_V1 to the server. K never leaves the browser.
      const grantHeaders: Record<string, string> = {
        "Content-Type": "application/cbor",
      };
      if (token) grantHeaders["X-Grant-Token"] = token;

      const grantRes = await fetch(`${MCP_BASE}/api/grants`, {
        method: "POST",
        headers: grantHeaders,
        body: grantCbor,
      });

      if (!grantRes.ok) {
        const text = await grantRes.text().catch(() => "");
        throw new Error(
          `Server rejected grant (HTTP ${grantRes.status}): ${text}`,
        );
      }

      setPhase({ kind: "done" });
    } catch (e) {
      setPhase({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [phase, identity]);

  const handleReject = useCallback(() => {
    setPhase({
      kind: "error",
      message: "Grant rejected. Return to your AI tool.",
    });
  }, []);

  return (
    <main
      className="mx-auto max-w-xl px-6 py-16 text-text-primary"
      data-testid="grant-approve-page"
    >
      <header className="space-y-2 mb-6">
        <Link
          to="/install"
          className="text-sm text-text-muted transition-colors hover:text-text-primary"
        >
          ← Cancel
        </Link>
        <h1 className="text-2xl font-semibold">Grant memory access</h1>
        <p className="text-sm text-text-muted">
          Your AI tool is requesting that you grant another agent access to a
          sealed memory. The key never leaves your browser.
        </p>
      </header>

      {/* Memory details panel */}
      {memoryHash && reader && (
        <div className="rounded-md border border-text-muted/30 bg-white/5 px-4 py-3 text-sm mb-6">
          <div className="font-mono text-xs text-text-muted">Memory hash</div>
          <div className="mt-1 break-all font-mono text-xs text-text-primary">
            {memoryHash || "(none)"}
          </div>
          <div className="mt-3 font-mono text-xs text-text-muted">
            Grant to
          </div>
          <div className="mt-1 break-all font-mono text-xs text-text-primary">
            {reader || "(none)"}
          </div>
          {identity && (
            <>
              <div className="mt-3 font-mono text-xs text-text-muted">
                Signing as
              </div>
              <div className="mt-1 break-all font-mono text-xs text-text-primary">
                {identity.pubkey_base58}
              </div>
            </>
          )}
        </div>
      )}

      {phase.kind === "loading" && (
        <p className="text-sm text-text-muted" data-testid="grant-status">
          Loading…
        </p>
      )}

      {phase.kind === "no-identity" && (
        <div className="rounded-md border border-error/40 bg-error/10 px-4 py-3 text-sm">
          <p className="font-medium">No keypair found in this browser.</p>
          <p className="mt-2 text-text-muted">
            Visit{" "}
            <Link to="/install" className="text-accent-primary underline">
              /install
            </Link>{" "}
            to generate or import a keypair, then retry the share from your MCP
            client.
          </p>
        </div>
      )}

      {phase.kind === "ready" && (
        <div className="flex gap-3">
          <button
            type="button"
            onClick={handleProceed}
            className="rounded-md bg-accent-primary px-4 py-2 text-sm font-medium text-background transition-colors hover:bg-accent-primary/90"
            data-testid="grant-proceed"
          >
            Grant access
          </button>
          <button
            type="button"
            onClick={handleReject}
            className="rounded-md border border-text-muted/30 px-4 py-2 text-sm font-medium text-text-primary transition-colors hover:border-error hover:text-error"
            data-testid="grant-reject"
          >
            Reject
          </button>
        </div>
      )}

      {/* No-revocation warning — tech-spec §5.5 */}
      {phase.kind === "warn" && (
        <div className="space-y-4" data-testid="grant-warn">
          <div className="rounded-md border border-error/40 bg-error/10 p-4 text-sm">
            <p className="font-semibold text-error">
              Warning: grants cannot be revoked
            </p>
            <ul className="mt-2 list-disc pl-4 space-y-1 text-text-muted">
              <li>
                Once granted, the recipient keeps read access to this memory
                forever. You cannot revoke it.
              </li>
              <li>
                You can stop serving this grant from the server, but the
                recipient may have already saved the key locally.
              </li>
              <li>
                A new version of the memory has a new key; the old key will not
                open it.
              </li>
            </ul>
          </div>
          <div className="flex gap-3">
            <button
              type="button"
              onClick={handleConfirm}
              className="rounded-md bg-accent-primary px-4 py-2 text-sm font-medium text-background transition-colors hover:bg-accent-primary/90"
              data-testid="grant-confirm"
            >
              I understand — grant access
            </button>
            <button
              type="button"
              onClick={() => setPhase({ kind: "ready", memoryHash, reader, grantToken })}
              className="rounded-md border border-text-muted/30 px-4 py-2 text-sm font-medium text-text-primary transition-colors hover:border-error hover:text-error"
              data-testid="grant-cancel-warn"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {phase.kind === "granting" && (
        <p className="text-sm text-text-muted" data-testid="grant-status">
          Preparing grant in WASM…
        </p>
      )}

      {phase.kind === "done" && (
        <div
          className="rounded-md border border-success/30 bg-success/10 p-4 text-sm text-success"
          role="status"
          data-testid="grant-done"
        >
          <p className="font-medium">Grant recorded successfully.</p>
          <p className="mt-1 text-text-muted">
            The recipient can now access this memory. Return to your AI tool.
          </p>
        </div>
      )}

      {phase.kind === "error" && (
        <div
          className="rounded-md border border-error/40 bg-error/10 px-4 py-3 text-sm"
          role="alert"
          data-testid="grant-error"
        >
          <p className="font-medium">Failed</p>
          <p className="mt-2 break-words font-mono text-xs">
            {phase.message}
          </p>
        </div>
      )}
    </main>
  );
}

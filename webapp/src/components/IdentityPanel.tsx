import { useEffect, useMemo, useRef, useState, useCallback } from "react";
import {
  clearIdentity,
  readIdentity,
  readJwt,
  writeIdentity,
  type KeypairJson,
} from "../lib/storage";
import { loadWasm } from "../lib/wasm";

/**
 * MCP backend host — mirrors the resolution used in `Sign.tsx` and
 * `Consent.tsx`. The CLI bootstrap endpoints (`/api/cli-bootstrap/issue`
 * and `/api/cli-bootstrap/redeem/:ticket`) live on the MCP server, not the
 * webapp origin, so we always go through this base URL.
 */
const MCP_BASE =
  (import.meta.env?.VITE_MCP_BASE as string | undefined) ??
  "https://mcp.mnemonik.xyz";

/** Bootstrap-ticket TTL fallback when the server omits `expires_at`. */
const BOOTSTRAP_TTL_MS = 10 * 60 * 1000;

/**
 * "Send to CLI" UI state machine.
 *
 *   `idle`    — button is shown, no ticket outstanding
 *   `issuing` — POST in flight; button disabled, no ticket yet
 *   `issued`  — ticket UUID returned by server; show paste-command + countdown
 *   `error`   — issue failed; show inline error message
 */
type CliBootstrapState =
  | { kind: "idle" }
  | { kind: "issuing" }
  | { kind: "issued"; ticketId: string; expiresAtMs: number }
  | { kind: "error"; message: string };

/**
 * "Send to Extension" UI state machine — mirrors `CliBootstrapState` but
 * tracks a separate ticket so a user can issue both a CLI and an extension
 * ticket without one clobbering the other.
 *
 * The extension polls `GET /api/extension-bootstrap/redeem/:ticket` after
 * the user pastes the ticket UUID into its onboarding screen (T20 popup
 * onboarding); on success the extension receives the same flat 64-byte
 * keypair the CLI flow returns and writes it into `chrome.storage.local`.
 *
 * For MVP the webapp surface is the same one-line paste-command pattern
 * as Send to CLI — simplest viable approach. A future deep-link
 * (`chrome-extension://<id>/popup/index.html#bootstrap=<ticket>`) is
 * backlog and would require the Chrome Web Store extension id, which
 * isn't known until first publication.
 */
type ExtBootstrapState =
  | { kind: "idle" }
  | { kind: "issuing" }
  | { kind: "issued"; ticketId: string; expiresAtMs: number }
  | { kind: "error"; message: string };

/**
 * Identity panel — renders the active DID/pubkey and exposes Generate / Import
 * / Export actions. Backed entirely by the WASM module from `webapp/src/wasm/`.
 *
 * The DID format is `did:sol:<base58_pubkey>` per Decision 4.
 *
 * Storage caveat: the keypair is stored as plain JSON in localStorage today. The
 * Risks table (XSS row) calls for AES-GCM encryption with a passphrase-derived
 * key as a hardening step. Phase 1 ships the unencrypted form gated by the CSP
 * meta tag in `index.html`; the encryption upgrade is tracked as a follow-up.
 */
export default function IdentityPanel() {
  const [identity, setIdentity] = useState<KeypairJson | null>(() =>
    readIdentity(),
  );
  const [error, setError] = useState<string | null>(null);
  const [isWorking, setIsWorking] = useState(false);
  const importInputRef = useRef<HTMLInputElement>(null);

  // "Send to CLI" state — separate from the panel's main `error` channel so
  // a failed bootstrap-ticket issue doesn't clobber a previously-shown
  // generate/import/export error and vice-versa.
  // Drift-warning modal state — shown when the user clicks "Generate new"
  // while an identity already exists.
  const [showGenerateModal, setShowGenerateModal] = useState(false);
  const cancelButtonRef = useRef<HTMLButtonElement>(null);

  const [cliState, setCliState] = useState<CliBootstrapState>({ kind: "idle" });
  // "Send to Extension" — distinct ticket lifecycle so the user can issue
  // CLI and extension tickets without conflict. The extension ticket TTL,
  // rate-limit window, and one-shot semantics mirror the CLI flow on the
  // server (see T15 extension-bootstrap endpoint).
  const [extState, setExtState] = useState<ExtBootstrapState>({ kind: "idle" });
  const [copied, setCopied] = useState(false);
  const [extCopied, setExtCopied] = useState(false);
  const [now, setNow] = useState<number>(() => Date.now());

  useEffect(() => {
    // Pre-warm the WASM module so the first user action feels instant.
    loadWasm().catch((e) => {
      setError(`WASM module failed to load: ${formatError(e)}`);
    });
  }, []);

  /**
   * Actually execute keypair generation — shared by both the first-time
   * path (no identity) and the "Generate anyway" destructive path (via modal).
   */
  const executeGenerate = useCallback(async () => {
    setError(null);
    setIsWorking(true);
    try {
      const wasm = await loadWasm();
      const value = wasm.generate_keypair() as KeypairJson;
      writeIdentity(value);
      setIdentity(value);
    } catch (e) {
      setError(`Generate failed: ${formatError(e)}`);
    } finally {
      setIsWorking(false);
    }
  }, []);

  /**
   * "Generate" button handler. If an identity already exists, open the
   * drift-warning modal so the user can back up / send to CLI before
   * replacing. For first-time generation (no existing identity), proceed
   * directly without confirmation.
   */
  const handleGenerate = () => {
    if (identity) {
      setShowGenerateModal(true);
    } else {
      void executeGenerate();
    }
  };

  /** Focus the Cancel button when the modal opens (accessibility). */
  useEffect(() => {
    if (showGenerateModal) {
      // rAF gives the DOM time to mount the modal before we try to focus.
      const id = window.requestAnimationFrame(() => {
        cancelButtonRef.current?.focus();
      });
      return () => window.cancelAnimationFrame(id);
    }
  }, [showGenerateModal]);

  const handleImportClick = () => {
    importInputRef.current?.click();
  };

  const handleImportFile = async (
    e: React.ChangeEvent<HTMLInputElement>,
  ): Promise<void> => {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow re-picking same file
    if (!file) return;

    setError(null);
    setIsWorking(true);
    try {
      const text = await file.text();
      const wasm = await loadWasm();
      const imported = wasm.import_keypair_json(text) as KeypairJson;
      writeIdentity(imported);
      setIdentity(imported);
    } catch (err) {
      setError(`Import failed: ${formatError(err)}`);
    } finally {
      setIsWorking(false);
    }
  };

  const handleExport = async () => {
    if (!identity) return;
    setError(null);
    setIsWorking(true);
    try {
      const wasm = await loadWasm();
      const json = wasm.export_keypair_json(identity);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      const truncated = identity.pubkey_base58.slice(0, 12);
      a.download = `mnemonic-keypair-${truncated}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (err) {
      setError(`Export failed: ${formatError(err)}`);
    } finally {
      setIsWorking(false);
    }
  };

  const handleClear = () => {
    if (
      !window.confirm(
        "Clear local keypair? You will lose access to memories signed by this identity unless you have a backup.",
      )
    ) {
      return;
    }
    clearIdentity();
    setIdentity(null);
  };

  /**
   * "Send to CLI" — POST the localStorage keypair JSON to
   * `/api/cli-bootstrap/issue` (Bearer JWT required). On success the server
   * returns `{ticket_id}`; we render the resulting one-line `mnemonic
   * identity import --ticket <uuid>` command for the user to paste in their
   * terminal. On 401 we redirect to `/oauth/consent` to re-auth; on 429 we
   * surface the per-user cap message; other errors render inline.
   *
   * Wire-shape note: localStorage holds the rich object form
   * `{secret: number[64], pubkey_base58: string}` (see `lib/storage.ts`),
   * but the T6 redeem handler parses the stored `keypair_json` as a bare
   * 64-byte JSON array — `serde_json::from_str::<Vec<u8>>(&keypair_json)`
   * in `mcp/src/api.rs::bootstrap_redeem_handler`. We therefore extract
   * `parsed.secret` and post that flat array (stringified) so the redeem
   * round-trip succeeds end-to-end. Posting the object form verbatim would
   * 500 on redeem with "stored keypair_json is not a JSON byte array".
   */
  const handleSendToCli = async () => {
    setCopied(false);
    // Identity must exist — the button is disabled when it doesn't, but we
    // re-check in case the user clears localStorage in another tab.
    const stored = localStorage.getItem("mnemonic.identity");
    if (!stored) {
      setCliState({
        kind: "error",
        message: "No identity to send. Generate or import one first.",
      });
      return;
    }
    // Parse the stored identity and extract the bare 64-byte secret. The
    // server expects a flat JSON byte array (Solana on-disk convention), so
    // we re-stringify just `parsed.secret` rather than the wrapping object.
    let secretWire: string;
    try {
      const parsed = JSON.parse(stored) as unknown;
      if (
        !parsed ||
        typeof parsed !== "object" ||
        !("secret" in parsed) ||
        !Array.isArray((parsed as { secret: unknown }).secret)
      ) {
        throw new Error("stored identity has no `secret` array");
      }
      const secret = (parsed as { secret: unknown[] }).secret;
      if (secret.length !== 64 || !secret.every((n) => typeof n === "number")) {
        throw new Error(
          `stored secret must be exactly 64 numbers, got ${secret.length}`,
        );
      }
      secretWire = JSON.stringify(secret);
    } catch (e) {
      setCliState({
        kind: "error",
        message: `Local identity is malformed: ${formatError(
          e,
        )}. Re-generate or re-import a keypair.`,
      });
      return;
    }
    const jwt = readJwt();
    if (!jwt) {
      // Without a JWT there is no way to authenticate the issue request.
      // Send the user through OAuth consent; they'll return with a token.
      window.location.assign("/oauth/consent");
      return;
    }

    setCliState({ kind: "issuing" });
    try {
      const res = await fetch(`${MCP_BASE}/api/cli-bootstrap/issue`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${jwt}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ keypair_json: secretWire }),
      });

      if (res.status === 401) {
        // Token expired or rejected — re-auth.
        window.location.assign("/oauth/consent");
        return;
      }
      if (res.status === 429) {
        setCliState({
          kind: "error",
          message:
            "You have 3 active CLI tickets. Wait for one to expire (10 min) or revoke later.",
        });
        return;
      }
      if (!res.ok) {
        let detail = `HTTP ${res.status}`;
        try {
          const body = await res.json();
          if (body && typeof body.error === "string") detail = body.error;
        } catch {
          // ignore parse error — keep generic detail
        }
        setCliState({
          kind: "error",
          message: `Could not issue ticket: ${detail}`,
        });
        return;
      }

      const body = (await res.json()) as {
        ticket_id?: unknown;
        expires_at?: unknown;
      };
      if (typeof body.ticket_id !== "string" || body.ticket_id.length === 0) {
        setCliState({
          kind: "error",
          message: "Could not issue ticket: server returned no ticket_id.",
        });
        return;
      }
      // Server may include an absolute `expires_at` (unix seconds). If
      // absent, fall back to "now + 10 minutes" per Decision 7.
      const expiresAtMs =
        typeof body.expires_at === "number"
          ? body.expires_at * 1000
          : Date.now() + BOOTSTRAP_TTL_MS;
      setCliState({
        kind: "issued",
        ticketId: body.ticket_id,
        expiresAtMs,
      });
    } catch (e) {
      setCliState({
        kind: "error",
        message: `Could not issue ticket: ${formatError(e)}`,
      });
    }
  };

  /**
   * Copy the paste-command to the clipboard. Uses the modern Clipboard API;
   * older browsers (or test environments without `navigator.clipboard`)
   * surface the failure inline rather than crashing the panel.
   */
  const handleCopyTicket = async () => {
    if (cliState.kind !== "issued") return;
    const command = `mnemonic identity import --ticket ${cliState.ticketId}`;
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      // Reset the "Copied" label after a short delay so a second copy
      // attempt re-flashes the confirmation.
      window.setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      setCliState({
        kind: "error",
        message: `Copy failed: ${formatError(e)}`,
      });
    }
  };

  /**
   * "Send to Extension" — same wire shape as Send to CLI, different
   * server route (`/api/extension-bootstrap/issue`) and different paste
   * target (Mnemonik popup → Onboarding → "Restore from webapp").
   *
   * We issue a one-shot ticket; the user pastes the UUID into the popup;
   * the popup polls `GET /api/extension-bootstrap/redeem/<ticket>` and
   * writes the resulting 64-byte secret into `chrome.storage.local`.
   * Simplest viable approach (per decisions.md T20 entry); a future
   * `chrome-extension://<id>/...` deep link requires the published
   * extension id and is backlog.
   */
  const handleSendToExtension = async () => {
    setExtCopied(false);
    const stored = localStorage.getItem("mnemonic.identity");
    if (!stored) {
      setExtState({
        kind: "error",
        message: "No identity to send. Generate or import one first.",
      });
      return;
    }
    // Re-use the same flat 64-byte secret extraction logic as Send to CLI
    // so the server-side redeem handler can stay payload-agnostic.
    let secretWire: string;
    try {
      const parsed = JSON.parse(stored) as unknown;
      if (
        !parsed ||
        typeof parsed !== "object" ||
        !("secret" in parsed) ||
        !Array.isArray((parsed as { secret: unknown }).secret)
      ) {
        throw new Error("stored identity has no `secret` array");
      }
      const secret = (parsed as { secret: unknown[] }).secret;
      if (secret.length !== 64 || !secret.every((n) => typeof n === "number")) {
        throw new Error(
          `stored secret must be exactly 64 numbers, got ${secret.length}`,
        );
      }
      secretWire = JSON.stringify(secret);
    } catch (e) {
      setExtState({
        kind: "error",
        message: `Local identity is malformed: ${formatError(
          e,
        )}. Re-generate or re-import a keypair.`,
      });
      return;
    }
    const jwt = readJwt();
    if (!jwt) {
      window.location.assign("/oauth/consent");
      return;
    }

    setExtState({ kind: "issuing" });
    try {
      const res = await fetch(`${MCP_BASE}/api/extension-bootstrap/issue`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${jwt}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ keypair_json: secretWire }),
      });

      if (res.status === 401) {
        window.location.assign("/oauth/consent");
        return;
      }
      if (res.status === 429) {
        setExtState({
          kind: "error",
          message:
            "You have 3 active extension tickets. Wait for one to expire (10 min).",
        });
        return;
      }
      if (!res.ok) {
        let detail = `HTTP ${res.status}`;
        try {
          const body = await res.json();
          if (body && typeof body.error === "string") detail = body.error;
        } catch {
          // ignore parse error — keep generic detail
        }
        setExtState({
          kind: "error",
          message: `Could not issue ticket: ${detail}`,
        });
        return;
      }

      const body = (await res.json()) as {
        ticket_id?: unknown;
        expires_at?: unknown;
      };
      if (typeof body.ticket_id !== "string" || body.ticket_id.length === 0) {
        setExtState({
          kind: "error",
          message: "Could not issue ticket: server returned no ticket_id.",
        });
        return;
      }
      const expiresAtMs =
        typeof body.expires_at === "number"
          ? body.expires_at * 1000
          : Date.now() + BOOTSTRAP_TTL_MS;
      setExtState({
        kind: "issued",
        ticketId: body.ticket_id,
        expiresAtMs,
      });
    } catch (e) {
      setExtState({
        kind: "error",
        message: `Could not issue ticket: ${formatError(e)}`,
      });
    }
  };

  const handleCopyExtTicket = async () => {
    if (extState.kind !== "issued") return;
    try {
      await navigator.clipboard.writeText(extState.ticketId);
      setExtCopied(true);
      window.setTimeout(() => setExtCopied(false), 2000);
    } catch (e) {
      setExtState({
        kind: "error",
        message: `Copy failed: ${formatError(e)}`,
      });
    }
  };

  /**
   * "Send to CLI" from inside the drift-warning modal — issues a ticket
   * for the CURRENT keypair so the CLI can adopt it before it disappears.
   * Closes the modal and reuses the existing handleSendToCli logic.
   */
  const handleModalSendToCli = () => {
    setShowGenerateModal(false);
    void handleSendToCli();
  };

  /**
   * "Download backup JSON" from inside the drift-warning modal.
   * Exports the current keypair in legacy shape
   * `{secret: number[64], pubkey_base58: string}` as a downloadable file.
   */
  const handleModalDownloadBackup = () => {
    if (!identity) return;
    const date = new Date().toISOString().slice(0, 10); // YYYY-MM-DD
    const filename = `mnemonic-identity-backup-${date}.json`;
    const payload = JSON.stringify(
      { secret: identity.secret, pubkey_base58: identity.pubkey_base58 },
      null,
      2,
    );
    const blob = new Blob([payload], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  // Tick the countdown every second only while a ticket is outstanding.
  // Outside the `issued` state the interval is a waste of cycles.
  useEffect(() => {
    if (cliState.kind !== "issued" && extState.kind !== "issued") return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [cliState.kind, extState.kind]);

  const ticketRemainingMs = useMemo(() => {
    if (cliState.kind !== "issued") return 0;
    return Math.max(0, cliState.expiresAtMs - now);
  }, [cliState, now]);
  const ticketRemainingLabel = formatMmSs(ticketRemainingMs);
  // Auto-expire the displayed ticket when the timer hits zero. The server
  // already drops the entry at TTL; this just stops misleading the user.
  useEffect(() => {
    if (cliState.kind === "issued" && ticketRemainingMs === 0) {
      setCliState({
        kind: "error",
        message: "Ticket expired. Click Send to CLI to issue a new one.",
      });
    }
  }, [cliState, ticketRemainingMs]);

  // Same machinery for the extension ticket — separate so the two timers
  // don't share an "expired" message channel.
  const extTicketRemainingMs = useMemo(() => {
    if (extState.kind !== "issued") return 0;
    return Math.max(0, extState.expiresAtMs - now);
  }, [extState, now]);
  const extTicketRemainingLabel = formatMmSs(extTicketRemainingMs);
  useEffect(() => {
    if (extState.kind === "issued" && extTicketRemainingMs === 0) {
      setExtState({
        kind: "error",
        message: "Ticket expired. Click Send to Extension to issue a new one.",
      });
    }
  }, [extState, extTicketRemainingMs]);

  return (
    <section className="space-y-4" aria-label="Agent identity">
      <h2 className="text-lg font-semibold text-text-primary">
        Agent identity
      </h2>

      {identity ? (
        <div
          className="space-y-2 rounded-md border border-text-muted/20 bg-white/5 p-4"
          data-testid="identity-display"
        >
          <div className="flex items-center justify-between gap-4">
            <span className="text-xs uppercase tracking-wide text-text-muted">
              DID
            </span>
            <code
              className="truncate font-mono text-sm text-accent-primary"
              data-testid="identity-did"
              aria-label="Agent decentralized identifier"
            >
              did:sol:{identity.pubkey_base58}
            </code>
          </div>
          <div className="flex items-center justify-between gap-4">
            <span className="text-xs uppercase tracking-wide text-text-muted">
              Pubkey
            </span>
            <code
              className="truncate font-mono text-xs text-text-primary"
              data-testid="identity-pubkey"
              aria-label="Ed25519 public key (base58)"
            >
              {identity.pubkey_base58}
            </code>
          </div>
        </div>
      ) : (
        <p className="rounded-md border border-text-muted/20 bg-white/5 p-4 text-sm text-text-muted">
          No identity yet. Generate a new keypair or import an existing backup.
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={handleGenerate}
          disabled={isWorking}
          className="rounded-md bg-accent-primary px-4 py-2 text-sm font-medium text-background transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-generate"
        >
          {identity ? "Generate new" : "Generate"}
        </button>
        <button
          type="button"
          onClick={handleImportClick}
          disabled={isWorking}
          className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-import"
        >
          Import keypair
        </button>
        <button
          type="button"
          onClick={handleExport}
          disabled={isWorking || !identity}
          className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-export"
        >
          Export backup
        </button>
        {identity && (
          <button
            type="button"
            onClick={handleClear}
            disabled={isWorking}
            className="rounded-md border border-error/30 px-4 py-2 text-sm text-error transition-colors hover:border-error disabled:cursor-not-allowed disabled:opacity-40"
            data-testid="identity-clear"
          >
            Clear local store
          </button>
        )}
        <button
          type="button"
          onClick={handleSendToCli}
          disabled={isWorking || !identity || cliState.kind === "issuing"}
          className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-send-to-cli"
        >
          {cliState.kind === "issuing" ? "Issuing ticket..." : "Send to CLI"}
        </button>
        <button
          type="button"
          onClick={handleSendToExtension}
          disabled={isWorking || !identity || extState.kind === "issuing"}
          className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-send-to-extension"
        >
          {extState.kind === "issuing"
            ? "Issuing ticket..."
            : "Send to Extension"}
        </button>
      </div>

      {extState.kind === "issued" && (
        <div
          className="space-y-2 rounded-md border border-accent-primary/30 bg-accent-primary/5 p-4"
          data-testid="identity-ext-ticket"
        >
          <p className="text-xs uppercase tracking-wide text-text-muted">
            Paste this ticket into the Mnemonik extension popup within{" "}
            {extTicketRemainingLabel}
          </p>
          <div className="flex items-center gap-2">
            <code
              className="flex-1 overflow-x-auto rounded bg-black/30 px-3 py-2 font-mono text-xs text-accent-primary"
              data-testid="identity-ext-ticket-id"
              aria-label="Extension bootstrap ticket ID"
            >
              {extState.ticketId}
            </code>
            <button
              type="button"
              onClick={handleCopyExtTicket}
              className="rounded-md border border-text-muted/30 px-3 py-2 text-xs text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary"
              data-testid="identity-ext-copy"
            >
              {extCopied ? "Copied" : "Copy"}
            </button>
          </div>
          <p
            className="font-mono text-xs text-text-muted"
            data-testid="identity-ext-countdown"
            aria-live="polite"
          >
            Expires in {extTicketRemainingLabel}
          </p>
        </div>
      )}

      {extState.kind === "error" && (
        <p
          className="rounded-md border border-error/30 bg-error/10 p-3 text-sm text-error"
          role="alert"
          data-testid="identity-ext-error"
        >
          {extState.message}
        </p>
      )}

      {cliState.kind === "issued" && (
        <div
          className="space-y-2 rounded-md border border-accent-primary/30 bg-accent-primary/5 p-4"
          data-testid="identity-cli-ticket"
        >
          <p className="text-xs uppercase tracking-wide text-text-muted">
            Run this in your terminal within {ticketRemainingLabel}
          </p>
          <div className="flex items-center gap-2">
            <code
              className="flex-1 overflow-x-auto rounded bg-black/30 px-3 py-2 font-mono text-xs text-accent-primary"
              data-testid="identity-cli-command"
              aria-label="CLI bootstrap command"
            >
              mnemonic identity import --ticket {cliState.ticketId}
            </code>
            <button
              type="button"
              onClick={handleCopyTicket}
              className="rounded-md border border-text-muted/30 px-3 py-2 text-xs text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary"
              data-testid="identity-cli-copy"
            >
              {copied ? "Copied" : "Copy"}
            </button>
          </div>
          <p
            className="font-mono text-xs text-text-muted"
            data-testid="identity-cli-countdown"
            aria-live="polite"
          >
            Expires in {ticketRemainingLabel}
          </p>
        </div>
      )}

      {cliState.kind === "error" && (
        <p
          className="rounded-md border border-error/30 bg-error/10 p-3 text-sm text-error"
          role="alert"
          data-testid="identity-cli-error"
        >
          {cliState.message}
        </p>
      )}

      <input
        ref={importInputRef}
        type="file"
        accept="application/json,.json"
        onChange={handleImportFile}
        className="hidden"
        aria-hidden="true"
      />

      {error && (
        <p
          className="rounded-md border border-error/30 bg-error/10 p-3 text-sm text-error"
          role="alert"
        >
          {error}
        </p>
      )}

      {showGenerateModal && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="replace-title"
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
        >
          <div className="w-full max-w-sm space-y-4 rounded-lg border border-text-muted/20 bg-background p-6 shadow-xl">
            <h3
              id="replace-title"
              className="text-base font-semibold text-text-primary"
            >
              Replace keypair?
            </h3>
            <p className="text-sm text-red-400">
              This will replace your current keypair. JWTs your IDEs and CLI
              hold will stop working. Before continuing, you can back up or
              transfer the existing identity:
            </p>
            <div className="flex flex-col gap-2">
              <button
                ref={cancelButtonRef}
                type="button"
                onClick={() => setShowGenerateModal(false)}
                className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowGenerateModal(false);
                  handleModalSendToCli();
                }}
                className="rounded-md border border-accent-primary/50 px-4 py-2 text-sm text-accent-primary transition-colors hover:border-accent-primary"
              >
                Send to CLI first
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowGenerateModal(false);
                  handleModalDownloadBackup();
                }}
                className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary"
              >
                Download backup JSON
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowGenerateModal(false);
                  void executeGenerate();
                }}
                className="rounded-md border border-red-500/50 px-4 py-2 text-sm text-red-400 transition-colors hover:border-red-500"
              >
                Generate anyway
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
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

/** mm:ss countdown — shared shape with `Sign.tsx::formatMmSs`. */
function formatMmSs(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const mm = Math.floor(totalSec / 60)
    .toString()
    .padStart(2, "0");
  const ss = (totalSec % 60).toString().padStart(2, "0");
  return `${mm}:${ss}`;
}

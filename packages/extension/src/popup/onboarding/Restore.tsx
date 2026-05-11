// Restore — second-device onboarding (T17).
//
// Triggered when `lookupExisting` returns
// `{existingPubkey, escrowPresent: true}`. The user types their recovery
// passphrase; the component fetches the encrypted blob from the server,
// AES-GCM-decrypts it locally, and persists the recovered keypair to
// `chrome.storage.local`.
//
// Failure handling:
//   - Wrong passphrase → `WrongPassphraseError` (local AES-GCM tag check;
//     no network call). We bump a local attempt counter; after 5 failures
//     we disable the input for 24h and persist `restore_blocked_until` to
//     `chrome.storage.local` so a popup re-open does not reset the
//     budget. The server enforces fetch-rate independently.
//   - 429 from server → `EscrowRateLimitError` carries `retry_after_seconds`;
//     we surface a countdown ("retry after N min").
//   - 404 → `EscrowNotFoundError`. We surface a static message — the user
//     can reset onboarding from the App shell.
//
// "Forgot passphrase?" opens a modal explaining the two recovery paths
// (start fresh OR import a keypair from another device); both are
// disabled in T17 — manual import is a follow-up.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type JSX,
} from "react";
import {
  EscrowNotFoundError,
  EscrowRateLimitError,
  WrongPassphraseError,
  fetchEscrow,
  unwrapSecret,
} from "../../auth/key-escrow.js";

/** Persistent block budget (T17 task spec § Restore UX). After 5 wrong
 *  passphrase attempts the input is disabled for 24h. The countdown
 *  survives popup re-opens via `chrome.storage.local.restore_blocked_until`. */
export const MAX_WRONG_ATTEMPTS = 5;
export const BLOCK_WINDOW_MS = 24 * 60 * 60 * 1000;

/** chrome.storage.local key for the wall-clock-ms unblock timestamp. */
export const RESTORE_BLOCKED_KEY = "restore_blocked_until";

/** chrome.storage.local keys for the persisted keypair after restore.
 *  Mirror the layout `popup/runtime-impl.ts::loadIdentity` reads —
 *  `identity = {pubkey_base58}` (public metadata) and `identity_secret`
 *  (64-byte Solana keypair as a number[]; structured-clone-safe). Using
 *  a separate key for the secret matches the rest of the popup runtime
 *  and avoids breaking `signMemory`. */
export const IDENTITY_STORAGE_KEY = "identity";
export const IDENTITY_SECRET_STORAGE_KEY = "identity_secret";

/** Truncate a base58 pubkey for the welcome-back headline ("H8x...c4v"). */
export function truncatePubkey(pub: string, head = 6, tail = 4): string {
  if (pub.length <= head + tail + 3) return pub;
  return `${pub.slice(0, head)}...${pub.slice(-tail)}`;
}

export interface RestoreProps {
  /** Server-issued Bearer JWT. */
  jwt: string;
  /** The Ed25519 pubkey already linked to this Google account, surfaced
   *  by `lookupExisting`. Rendered in the welcome headline AND used as
   *  the persisted `keypair.public` after a successful unwrap. */
  existingPubkey: string;
  /** Fired once the keypair is persisted. The host re-bootstraps. */
  onComplete: () => void;
  /** Optional MCP server origin (tests inject a stub). */
  serverOrigin?: string;
  /** Optional fetch implementation (tests inject a stub). */
  fetchImpl?: typeof fetch;
  /** Optional clock injection (tests advance time without timers). */
  now?: () => number;
  /**
   * Optional storage adapter. Defaults to `chrome.storage.local`. Tests
   * inject an in-memory implementation so the 24h block + keypair
   * persistence are observable without a `chrome.*` polyfill.
   */
  storage?: RestoreStorage;
}

/** Minimal storage surface — covers the two reads + two writes we need.
 *  Production wraps `chrome.storage.local`; tests pass a Map-backed stub. */
export interface RestoreStorage {
  get(keys: string[]): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
}

type Step =
  | { kind: "form" }
  | { kind: "fetching" }
  | { kind: "decrypting" }
  | { kind: "persisting" }
  | { kind: "done" }
  | { kind: "error"; message: string };

export function Restore(props: RestoreProps): JSX.Element {
  const {
    jwt,
    existingPubkey,
    onComplete,
    serverOrigin,
    fetchImpl,
    now = () => Date.now(),
    storage = chromeStorageAdapter(),
  } = props;

  const [pp, setPp] = useState("");
  const [attempts, setAttempts] = useState(0);
  const [blockedUntil, setBlockedUntil] = useState<number | null>(null);
  const [rateLimitedUntil, setRateLimitedUntil] = useState<number | null>(null);
  const [step, setStep] = useState<Step>({ kind: "form" });
  const [forgotOpen, setForgotOpen] = useState(false);
  // `nowTick` triggers re-renders so the countdown is live. We rebind on
  // a 1s interval while a block / rate-limit window is active, then
  // tear down to keep the popup idle when nothing is counting down.
  const [, setNowTick] = useState(0);
  const tickRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Restore the persisted block on mount so a popup re-open during the
  // 24h window does NOT reset the counter.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const stored = await storage.get([RESTORE_BLOCKED_KEY]);
        const raw = stored[RESTORE_BLOCKED_KEY];
        if (typeof raw === "number" && raw > now()) {
          if (!cancelled) setBlockedUntil(raw);
        }
      } catch {
        // Best-effort — a missing storage permission shouldn't crash the
        // popup. The user can still attempt restore; the in-memory
        // counter still applies for the lifetime of the popup.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [storage, now]);

  // Drive the live countdown only while a window is active.
  useEffect(() => {
    const blocked = blockedUntil !== null && blockedUntil > now();
    const limited = rateLimitedUntil !== null && rateLimitedUntil > now();
    if (!blocked && !limited) {
      if (tickRef.current !== null) {
        clearInterval(tickRef.current);
        tickRef.current = null;
      }
      return;
    }
    if (tickRef.current === null) {
      tickRef.current = setInterval(() => {
        setNowTick((t) => t + 1);
      }, 1000);
    }
    return () => {
      if (tickRef.current !== null) {
        clearInterval(tickRef.current);
        tickRef.current = null;
      }
    };
  }, [blockedUntil, rateLimitedUntil, now]);

  const isBlocked = blockedUntil !== null && blockedUntil > now();
  const isRateLimited = rateLimitedUntil !== null && rateLimitedUntil > now();
  const submitting =
    step.kind === "fetching" ||
    step.kind === "decrypting" ||
    step.kind === "persisting";
  const inputDisabled = isBlocked || isRateLimited || submitting;

  const onSubmit = useCallback(
    async (e: FormEvent): Promise<void> => {
      e.preventDefault();
      if (inputDisabled || pp.length === 0) return;
      // Snapshot + clear React state. The local `passphrase` binding
      // goes out of scope once this callback returns.
      const passphrase = pp;
      setPp("");
      setStep({ kind: "fetching" });

      let blob;
      try {
        const opts: { serverOrigin?: string; fetchImpl?: typeof fetch } = {};
        if (serverOrigin !== undefined) opts.serverOrigin = serverOrigin;
        if (fetchImpl !== undefined) opts.fetchImpl = fetchImpl;
        blob = await fetchEscrow(jwt, opts);
      } catch (err) {
        if (err instanceof EscrowRateLimitError) {
          const until = now() + err.retry_after_seconds * 1000;
          setRateLimitedUntil(until);
          setStep({
            kind: "error",
            message: `Server limited fetches. Retry after ${formatCountdown(
              err.retry_after_seconds * 1000,
            )}.`,
          });
          return;
        }
        if (err instanceof EscrowNotFoundError) {
          setStep({
            kind: "error",
            message:
              "No encrypted blob is stored on the server for this account. Re-enrol or import your keypair from another device.",
          });
          return;
        }
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Could not fetch encrypted blob: ${err.message}`
              : "Could not fetch encrypted blob.",
        });
        return;
      }

      // Local AES-GCM unwrap. Wrong passphrase surfaces here WITHOUT a
      // network round-trip — that's the rate-limit-budget protection
      // invariant `wrong_passphrase_fails_locally` binds.
      setStep({ kind: "decrypting" });
      let secret: Uint8Array;
      try {
        secret = await unwrapSecret(blob, passphrase);
      } catch (err) {
        if (err instanceof WrongPassphraseError) {
          const next = attempts + 1;
          setAttempts(next);
          if (next >= MAX_WRONG_ATTEMPTS) {
            const until = now() + BLOCK_WINDOW_MS;
            setBlockedUntil(until);
            try {
              await storage.set({ [RESTORE_BLOCKED_KEY]: until });
            } catch {
              // Best-effort; the in-memory state still blocks for this
              // popup lifetime.
            }
            setStep({
              kind: "error",
              message: `Too many wrong attempts. Restore locked for 24 hours.`,
            });
          } else {
            setStep({
              kind: "error",
              message: `Wrong passphrase. ${String(
                MAX_WRONG_ATTEMPTS - next,
              )} attempt${MAX_WRONG_ATTEMPTS - next === 1 ? "" : "s"} remaining.`,
            });
          }
          return;
        }
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Decrypt failed: ${err.message}`
              : "Decrypt failed.",
        });
        return;
      }

      // Persist the recovered keypair. We hand the secret to storage
      // immediately so its in-memory window is minimised. Layout
      // matches `popup/runtime-impl.ts::loadIdentity`: a `identity`
      // metadata object plus a `identity_secret` number[] — production
      // `signMemory` reads BOTH.
      setStep({ kind: "persisting" });
      try {
        // Convert to plain number[] for chrome.storage's structured
        // clone — `Uint8Array` round-trips lossily in some MV3 builds.
        const secretArray = Array.from(secret);
        await storage.set({
          [IDENTITY_STORAGE_KEY]: { pubkey_base58: existingPubkey },
          [IDENTITY_SECRET_STORAGE_KEY]: secretArray,
        });
        // Best-effort wipe of the recovered bytes. JS gives no
        // guarantee — see the THREAT MODEL note in `key-escrow.ts`.
        secret.fill(0);
      } catch (err) {
        secret.fill(0);
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Persist failed: ${err.message}`
              : "Persist failed.",
        });
        return;
      }

      setStep({ kind: "done" });
      onComplete();
    },
    [
      attempts,
      existingPubkey,
      fetchImpl,
      inputDisabled,
      jwt,
      now,
      onComplete,
      pp,
      serverOrigin,
      storage,
    ],
  );

  return (
    <main className="bg-background text-text-primary p-4 flex flex-col gap-3 min-h-full">
      <h1 className="text-sm text-accent-primary font-semibold tracking-wide">
        Welcome back
      </h1>
      <p className="text-xs text-text-muted">
        Identity{" "}
        <span
          className="font-mono text-accent-primary"
          title={existingPubkey}
          data-testid="welcome-pubkey"
        >
          {truncatePubkey(existingPubkey)}
        </span>{" "}
        detected on this Google account. Enter your recovery passphrase to
        restore your keypair on this device.
      </p>

      <form onSubmit={(e) => void onSubmit(e)} className="flex flex-col gap-2">
        <label
          htmlFor="restore-pp"
          className="text-[11px] text-text-muted font-mono"
        >
          Recovery passphrase
        </label>
        <input
          id="restore-pp"
          data-testid="restore-pp-input"
          type="password"
          autoComplete="current-password"
          value={pp}
          onChange={(e) => setPp(e.target.value)}
          disabled={inputDisabled}
          aria-disabled={inputDisabled}
          className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono disabled:opacity-50"
        />
        <div className="flex items-center gap-2">
          <button
            type="submit"
            disabled={inputDisabled || pp.length === 0}
            className="bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
          >
            {step.kind === "fetching"
              ? "Fetching…"
              : step.kind === "decrypting"
                ? "Decrypting…"
                : step.kind === "persisting"
                  ? "Saving…"
                  : "Restore"}
          </button>
          <button
            type="button"
            onClick={() => setForgotOpen(true)}
            className="text-[11px] text-text-muted hover:text-text-primary underline"
          >
            Forgot passphrase?
          </button>
        </div>

        {isBlocked && blockedUntil !== null ? (
          <p
            className="text-[11px] text-error"
            role="alert"
            data-testid="restore-block-countdown"
          >
            Restore locked. Try again in {formatCountdown(blockedUntil - now())}
            .
          </p>
        ) : null}
        {!isBlocked && isRateLimited && rateLimitedUntil !== null ? (
          <p
            className="text-[11px] text-warning"
            role="alert"
            data-testid="restore-rate-limit-countdown"
          >
            Server limited fetches. Try again in{" "}
            {formatCountdown(rateLimitedUntil - now())}.
          </p>
        ) : null}
        {!isBlocked && step.kind === "error" ? (
          <p
            className="text-[11px] text-error"
            role="alert"
            data-testid="restore-error"
          >
            {step.message}
          </p>
        ) : null}
      </form>

      {forgotOpen ? (
        <ForgotPassphraseModal onClose={() => setForgotOpen(false)} />
      ) : null}
    </main>
  );
}

// ── Forgot-passphrase modal ─────────────────────────────────────────────────

function ForgotPassphraseModal({
  onClose,
}: {
  onClose: () => void;
}): JSX.Element {
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="forgot-pp-title"
      className="fixed inset-0 bg-black/70 flex items-center justify-center p-4"
    >
      <div className="bg-background border border-white/10 rounded p-4 max-w-sm flex flex-col gap-3">
        <h2
          id="forgot-pp-title"
          className="text-sm font-mono uppercase tracking-wide text-accent-primary"
        >
          Forgot recovery passphrase
        </h2>
        <p className="text-xs text-text-muted">
          We can&apos;t recover it for you — the server only stores the
          encrypted blob. You have two options:
        </p>
        <ul className="text-xs text-text-muted list-disc pl-5 flex flex-col gap-1">
          <li>
            Start fresh. You&apos;ll keep read-only access to your old
            cloud-history but cannot sign new attestations under the old key.
          </li>
          <li>Import an existing keypair exported from another device.</li>
        </ul>
        <div className="flex flex-col gap-2">
          <button
            type="button"
            disabled
            title="Manual import lands in a follow-up release."
            className="bg-white/5 text-text-muted border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
          >
            Start fresh (coming soon)
          </button>
          <button
            type="button"
            disabled
            title="Manual import lands in a follow-up release."
            className="bg-white/5 text-text-muted border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
          >
            Import keypair from another device (coming soon)
          </button>
          <button
            type="button"
            onClick={onClose}
            className="self-end bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Format a millisecond duration as `HH:MM:SS` for the block / rate-limit
 *  countdown. Defensive against negative values (returns "0s"). */
export function formatCountdown(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0s";
  const totalSec = Math.ceil(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) {
    return `${String(h)}h ${String(m).padStart(2, "0")}m`;
  }
  if (m > 0) {
    return `${String(m)}m ${String(s).padStart(2, "0")}s`;
  }
  return `${String(s)}s`;
}

/** Default storage adapter — production passes nothing and we wrap
 *  `chrome.storage.local`. Tests inject a stub via the `storage` prop. */
function chromeStorageAdapter(): RestoreStorage {
  return {
    async get(keys: string[]): Promise<Record<string, unknown>> {
      try {
        const out = (await chrome.storage.local.get(keys)) as Record<
          string,
          unknown
        >;
        return out ?? {};
      } catch {
        return {};
      }
    },
    async set(items: Record<string, unknown>): Promise<void> {
      try {
        await chrome.storage.local.set(items);
      } catch {
        // Best-effort — surface through the form rather than crashing.
      }
    },
  };
}

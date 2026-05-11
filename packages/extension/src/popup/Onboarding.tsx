// First-run onboarding flow (T16). Branches per the user-spec / task spec
// after the Google sign-in resolves to a `LookupResult`:
//
//   - existing_pubkey === null         → "Set recovery passphrase" + T17
//                                        wrap-and-upload stub.
//   - existing_pubkey + escrow_present → "Welcome back" + passphrase prompt
//                                        + T17 fetch-and-restore stub.
//   - existing_pubkey + no escrow      → "Existing identity but no escrow
//                                        on server" + manual import (T17
//                                        follow-up).
//
// All key-escrow side effects are stubs in this file (`keyEscrow.*Stub`)
// — T17 will replace them with the real Argon2id+AES-GCM pipeline. The
// UI wiring + the server `/oauth/google/link` call are owned by T14 (the
// link endpoint already ships) and T16 (this file).

import { useState, type JSX } from "react";
import { getRuntime } from "./runtime.js";
import type {
  GoogleSignInResult,
  LookupResult,
  Session,
} from "../auth/types.js";
import { jwtExpiresAtMs } from "../auth/session.js";

/**
 * Top-level onboarding state machine. The popup mounts this when
 * `runtime.session.get()` returned `null` on first paint.
 */
type OnboardingStep =
  | { kind: "intro" }
  | { kind: "signing_in" }
  | { kind: "set_passphrase"; signIn: GoogleSignInResult; lookup: LookupResult }
  | { kind: "welcome_back"; signIn: GoogleSignInResult; lookup: LookupResult }
  | {
      kind: "no_escrow_edge";
      signIn: GoogleSignInResult;
      lookup: LookupResult;
    }
  | { kind: "wrapping"; signIn: GoogleSignInResult }
  | { kind: "restoring"; signIn: GoogleSignInResult }
  | { kind: "done" }
  | { kind: "error"; message: string };

export interface OnboardingProps {
  /** Called once `done` is reached so the App can re-bootstrap. */
  onComplete: () => void;
}

export function Onboarding({ onComplete }: OnboardingProps): JSX.Element {
  const [step, setStep] = useState<OnboardingStep>({ kind: "intro" });
  const [passphrase, setPassphrase] = useState("");

  // ── Intro → sign-in → lookup ─────────────────────────────────────────────
  const handleSignIn = async (): Promise<void> => {
    setStep({ kind: "signing_in" });
    try {
      const r = getRuntime();
      const signIn = await r.auth.signIn();
      const lookup = await r.auth.lookupExisting(signIn.jwt);
      // Persist a minimal session — the popup re-reads it on next open.
      const session: Session = {
        jwt: signIn.jwt,
        googleSub: signIn.googleSub,
        profile: signIn.profile,
        jwtExpiresAt:
          jwtExpiresAtMs(signIn.jwt) ??
          // 1h fallback when the JWT lacks an `exp` claim — the server
          // contract guarantees it, but defence-in-depth keeps the
          // session usable.
          Date.now() + 60 * 60 * 1000,
        signedInAt: Date.now(),
      };
      await r.session.set(session);

      if (lookup.existingPubkey === null) {
        setStep({ kind: "set_passphrase", signIn, lookup });
      } else if (lookup.escrowPresent) {
        setStep({ kind: "welcome_back", signIn, lookup });
      } else {
        setStep({ kind: "no_escrow_edge", signIn, lookup });
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : "sign-in failed";
      setStep({ kind: "error", message });
    }
  };

  // ── Branch 1: no existing pubkey → set passphrase → wrap + upload ──────
  const handleSetPassphrase = async (
    signIn: GoogleSignInResult,
  ): Promise<void> => {
    if (passphrase.length < 12) {
      setStep({
        kind: "error",
        message: "passphrase must be at least 12 characters",
      });
      return;
    }
    setStep({ kind: "wrapping", signIn });
    try {
      // T17 stub: real implementation will Argon2id-derive a key,
      // AES-GCM-256 wrap the freshly-generated Ed25519 secret, and POST
      // `PUT /api/key-escrow` followed by `POST /oauth/google/link` with
      // the possession proof. Here we only call the documented stub —
      // T17 replaces it.
      await keyEscrow.wrapAndUploadStub({ passphrase, jwt: signIn.jwt });
      setStep({ kind: "done" });
      onComplete();
    } catch (e) {
      const message = e instanceof Error ? e.message : "wrap-and-upload failed";
      setStep({ kind: "error", message });
    }
  };

  // ── Branch 2: existing pubkey + escrow → fetch + restore ───────────────
  const handleWelcomeBack = async (
    signIn: GoogleSignInResult,
  ): Promise<void> => {
    setStep({ kind: "restoring", signIn });
    try {
      await keyEscrow.fetchAndRestoreStub({ passphrase, jwt: signIn.jwt });
      setStep({ kind: "done" });
      onComplete();
    } catch (e) {
      const message = e instanceof Error ? e.message : "restore failed";
      setStep({ kind: "error", message });
    }
  };

  // ── Render ──────────────────────────────────────────────────────────────

  if (step.kind === "intro") {
    return (
      <Frame title="Welcome to Mnemonik">
        <p className="text-xs text-text-muted">
          Capture AI-chat context as verifiable memory. Sign in with Google to
          enable cloud sync and cross-device restore. Local-only mode is
          available from Settings after onboarding.
        </p>
        <button
          type="button"
          onClick={() => void handleSignIn()}
          className="bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Sign in with Google
        </button>
      </Frame>
    );
  }

  if (step.kind === "signing_in") {
    return (
      <Frame title="Signing in">
        <p className="text-xs text-text-muted">
          Opening Google consent screen…
        </p>
      </Frame>
    );
  }

  if (step.kind === "set_passphrase") {
    return (
      <Frame title="Set recovery passphrase">
        <p className="text-xs text-text-muted">
          Your passphrase encrypts your identity key. We cannot recover it for
          you — store it in a password manager. Minimum 12 characters.
        </p>
        <label htmlFor="passphrase" className="sr-only">
          Recovery passphrase
        </label>
        <input
          id="passphrase"
          type="password"
          autoComplete="new-password"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          className="bg-black/30 border border-white/10 rounded px-2 py-1 text-xs font-mono"
          placeholder="Recovery passphrase"
        />
        <button
          type="button"
          onClick={() => void handleSetPassphrase(step.signIn)}
          className="bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Encrypt &amp; upload
        </button>
      </Frame>
    );
  }

  if (step.kind === "welcome_back") {
    return (
      <Frame title="Welcome back">
        <p className="text-xs text-text-muted">
          Identity{" "}
          <span className="font-mono text-accent-primary">
            {step.lookup.existingPubkey?.slice(0, 8)}…
          </span>{" "}
          detected. Enter your recovery passphrase to restore your keypair on
          this device.
        </p>
        <label htmlFor="passphrase" className="sr-only">
          Recovery passphrase
        </label>
        <input
          id="passphrase"
          type="password"
          autoComplete="current-password"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          className="bg-black/30 border border-white/10 rounded px-2 py-1 text-xs font-mono"
          placeholder="Recovery passphrase"
        />
        <button
          type="button"
          onClick={() => void handleWelcomeBack(step.signIn)}
          className="bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Restore identity
        </button>
      </Frame>
    );
  }

  if (step.kind === "no_escrow_edge") {
    return (
      <Frame title="No escrow on server">
        <p className="text-xs text-text-muted">
          Your Google account is linked to identity{" "}
          <span className="font-mono text-accent-primary">
            {step.lookup.existingPubkey?.slice(0, 8)}…
          </span>
          , but we have no encrypted key blob for it. Import the keypair from
          another device (CLI / webapp export) to continue. Manual import lands
          in T17.
        </p>
        <button
          type="button"
          onClick={() => onComplete()}
          className="bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Continue in local mode
        </button>
      </Frame>
    );
  }

  if (step.kind === "wrapping") {
    return (
      <Frame title="Encrypting identity">
        <p className="text-xs text-text-muted">
          Deriving key, encrypting secret, uploading blob…
        </p>
      </Frame>
    );
  }

  if (step.kind === "restoring") {
    return (
      <Frame title="Restoring identity">
        <p className="text-xs text-text-muted">
          Fetching encrypted key, deriving Argon2id key, decrypting secret…
        </p>
      </Frame>
    );
  }

  if (step.kind === "error") {
    return (
      <Frame title="Sign-in failed">
        <p className="text-xs text-red-400" role="alert">
          {step.message}
        </p>
        <button
          type="button"
          onClick={() => setStep({ kind: "intro" })}
          className="bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Try again
        </button>
      </Frame>
    );
  }

  return (
    <Frame title="Done">
      <p className="text-xs text-text-muted">Loading popup…</p>
    </Frame>
  );
}

function Frame({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}): JSX.Element {
  return (
    <main className="bg-background text-text-primary p-4 flex flex-col gap-3 min-h-full">
      <h1 className="text-sm text-accent-primary font-semibold tracking-wide">
        {title}
      </h1>
      {children}
    </main>
  );
}

// ── T17 stubs ──────────────────────────────────────────────────────────────

/**
 * Key-escrow stubs. T17 (`packages/extension/src/auth/key-escrow.ts`)
 * replaces these with the real implementation:
 *
 *   - `wrapAndUploadStub` will Argon2id-derive a 256-bit key from the
 *     passphrase, AES-GCM-256 wrap the freshly-generated Ed25519 secret,
 *     `PUT /api/key-escrow` with the ciphertext + KDF params, then
 *     `POST /oauth/google/link` with a possession-proof signature over
 *     the server-issued challenge.
 *
 *   - `fetchAndRestoreStub` will `GET /api/key-escrow` for the
 *     ciphertext + KDF params, derive the same key, AES-GCM-decrypt,
 *     and persist the recovered keypair to `chrome.storage.local`.
 *
 * Both are intentionally rejected here so the UI surfaces a clear
 * "T17 follow-up" error in dev until the real implementation lands.
 */
const keyEscrow = {
  async wrapAndUploadStub(_args: {
    passphrase: string;
    jwt: string;
  }): Promise<void> {
    throw new Error(
      "key-escrow wrap-and-upload not implemented yet (T17 follow-up)",
    );
  },
  async fetchAndRestoreStub(_args: {
    passphrase: string;
    jwt: string;
  }): Promise<void> {
    throw new Error(
      "key-escrow fetch-and-restore not implemented yet (T17 follow-up)",
    );
  },
};

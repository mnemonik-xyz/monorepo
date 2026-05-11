// First-run onboarding flow (T16 + T17). Branches per the user-spec /
// task spec after the Google sign-in resolves to a `LookupResult`:
//
//   - existing_pubkey === null         → "Set recovery passphrase" → T17
//                                        wrap + upload + link.
//   - existing_pubkey + escrow_present → <Restore> (T17) — passphrase
//                                        prompt + fetchEscrow + unwrap +
//                                        keypair persist, with 5-attempt
//                                        local block + 429 surfacing.
//   - existing_pubkey + no escrow      → "Existing identity but no escrow
//                                        on server" + manual import (T17
//                                        follow-up — backlog).
//
// T17 replaces the prior stub `keyEscrow.{wrapAndUpload,fetchAndRestore}`
// calls with the real `auth/key-escrow.ts` client. The first-time-Cloud
// branch ("set passphrase") still uses the inline form here rather than
// the dedicated <SetPassphrase> component because keypair generation +
// the /oauth/google/link possession proof live in a different wave (T18
// will fold both into a single onboarding flow when the WASM signer
// loader is wired to the popup).

import { useState, type JSX } from "react";
import { getRuntime } from "./runtime.js";
import type {
  GoogleSignInResult,
  LookupResult,
  Session,
} from "../auth/types.js";
import { jwtExpiresAtMs } from "../auth/session.js";
import { Restore } from "./onboarding/Restore.js";
import { uploadEscrow, wrapSecret } from "../auth/key-escrow.js";

/** Fallback session lifetime when neither the server response nor the JWT
 *  carries a usable expiry. Mirrors the server's default `expires_in`. */
const FALLBACK_SESSION_MS = 60 * 60 * 1000;

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
  | { kind: "done" }
  | { kind: "error"; message: string };

export interface OnboardingProps {
  /** Called once `done` is reached so the App can re-bootstrap. */
  onComplete: () => void;
}

/** Minimum passphrase length per user-spec § Scenario 1. zxcvbn strength
 *  scoring lands in T17 alongside the real key-escrow client; T16 keeps
 *  the simpler length-only rule so both branches agree. */
const MIN_PASSPHRASE_LEN = 12;

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
      // Source of truth for token lifetime is the server's `expires_in`
      // (already wall-clock-converted into `signIn.jwtExpiresAt`); the
      // JWT `exp` claim is the next fallback so a hand-crafted test JWT
      // without `expires_in` still parses; finally a 1h default keeps
      // the session usable when both are missing (defence-in-depth).
      const session: Session = {
        jwt: signIn.jwt,
        googleSub: signIn.googleSub,
        profile: signIn.profile,
        jwtExpiresAt:
          signIn.jwtExpiresAt > 0
            ? signIn.jwtExpiresAt
            : (jwtExpiresAtMs(signIn.jwt) ?? Date.now() + FALLBACK_SESSION_MS),
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
    if (passphrase.length < MIN_PASSPHRASE_LEN) {
      setStep({
        kind: "error",
        message: `passphrase must be at least ${String(MIN_PASSPHRASE_LEN)} characters`,
      });
      // Clear before the user retries — see security-auditor SEC-MIN-3.
      setPassphrase("");
      return;
    }
    // Snapshot the passphrase locally so we can hand it to the T17 stub
    // and immediately wipe the React state slot. The local `pp` is the
    // only remaining reference; it goes out of scope as soon as the
    // function returns. This minimises the window in which the
    // passphrase is reachable from React DevTools / heap snapshots.
    const pp = passphrase;
    setPassphrase("");
    setStep({ kind: "wrapping", signIn });
    try {
      // T17: load the freshly-minted (or pre-existing local) keypair
      // from `chrome.storage.local`, wrap the secret under the user's
      // passphrase, and PUT to `/api/key-escrow`. The `/oauth/google/link`
      // possession-proof step requires Ed25519 signing via the WASM
      // bridge — the popup's WASM signer is wired by the Capture path
      // (`runtime-impl.ts::signMemory`), not here. Until the bridge is
      // exposed to onboarding (deferred to a follow-up wave), the link
      // call ships in <SetPassphrase> when the host owns the signer.
      const keypair = await loadLocalKeypair();
      if (!keypair) {
        throw new Error(
          "no local keypair available to encrypt — generate one before enabling Cloud sync",
        );
      }
      const blob = await wrapSecret(keypair.secret, pp, keypair.pubkey_base58);
      await uploadEscrow(signIn.jwt, blob);
      // Best-effort wipe of the secret bytes we held briefly.
      keypair.secret.fill(0);
      setStep({ kind: "done" });
      onComplete();
    } catch (e) {
      const message = e instanceof Error ? e.message : "wrap-and-upload failed";
      setStep({ kind: "error", message });
    }
  };

  // ── Branch 2: existing pubkey + escrow → render <Restore> ──────────────
  // T17 owns the entire flow (fetch → unwrap → persist + 5-attempt
  // local block + 429 surfacing). The component fires `onComplete` once
  // the keypair is persisted, then we re-bootstrap the popup.

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
    // Defensive: `welcome_back` is only entered when `existingPubkey` is
    // non-null, but TypeScript narrowing on the LookupResult cannot prove
    // that across the state-machine boundary, so we guard at the render
    // site. Falling back to the error frame is preferable to handing
    // <Restore> an empty pubkey string.
    if (!step.lookup.existingPubkey) {
      return (
        <Frame title="Restore unavailable">
          <p className="text-xs text-red-400" role="alert">
            Server returned an empty pubkey for this account. Please retry.
          </p>
        </Frame>
      );
    }
    return (
      <Restore
        jwt={step.signIn.jwt}
        existingPubkey={step.lookup.existingPubkey}
        onComplete={() => {
          setStep({ kind: "done" });
          onComplete();
        }}
      />
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

// ── chrome.storage helpers ─────────────────────────────────────────────────

/** Load the popup-realm keypair from `chrome.storage.local`. Mirrors
 *  `runtime-impl.ts::loadIdentity` exactly so the secret format stays
 *  in lockstep (64-byte Solana keypair = seed||pubkey). Returns `null`
 *  when no keypair has been minted yet (the set-passphrase branch then
 *  surfaces a typed error rather than uploading garbage).
 *
 *  WASM-side keypair generation is owned by T05 (`generate_keypair`
 *  export). When that lands the onboarding flow can mint a fresh
 *  identity in-place; today we require the popup to have been seeded
 *  (e.g. by a CLI export or the existing local-tier identity).
 */
async function loadLocalKeypair(): Promise<{
  pubkey_base58: string;
  secret: Uint8Array;
} | null> {
  let stored: {
    identity?: { pubkey_base58?: string } | null;
    identity_secret?: number[] | null;
  } = {};
  try {
    stored = (await chrome.storage.local.get([
      "identity",
      "identity_secret",
    ])) as typeof stored;
  } catch {
    return null;
  }
  const pub = stored.identity?.pubkey_base58;
  const sec = stored.identity_secret;
  if (!pub || !Array.isArray(sec) || sec.length === 0) return null;
  return { pubkey_base58: pub, secret: Uint8Array.from(sec) };
}

// First-run onboarding flow (T16 + T17). Branches per the user-spec /
// task spec after the Google sign-in resolves to a `LookupResult`:
//
//   - existing_pubkey === null         → <SetPassphrase> (T17) —
//                                        zxcvbn-meter passphrase form +
//                                        wrap + upload + /oauth/google/
//                                        link possession proof.
//   - existing_pubkey + escrow_present → <Restore> (T17) — passphrase
//                                        prompt + fetchEscrow + unwrap +
//                                        keypair persist, with 5-attempt
//                                        local block + 429 surfacing.
//   - existing_pubkey + no escrow      → "Existing identity but no escrow
//                                        on server" + manual import
//                                        (follow-up — backlog).
//
// Round-2: both branches route through dedicated components so the
// inline state machine here owns only the sign-in / lookup wiring. The
// zxcvbn meter + the `/oauth/google/link` possession proof live in
// <SetPassphrase>; the 5-attempt local block lives in <Restore>.

import { useEffect, useRef, useState, type JSX } from "react";
import { getRuntime } from "./runtime.js";
import type {
  GoogleSignInResult,
  LookupResult,
  Session,
} from "../auth/types.js";
import { Restore } from "./onboarding/Restore.js";
import { SetPassphrase } from "./onboarding/SetPassphrase.js";

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
  | {
      kind: "set_passphrase";
      signIn: GoogleSignInResult;
      lookup: LookupResult;
      keypair: { pubkey_base58: string; secret: Uint8Array };
      linkChallenge: string;
    }
  | { kind: "welcome_back"; signIn: GoogleSignInResult; lookup: LookupResult }
  | {
      kind: "no_escrow_edge";
      signIn: GoogleSignInResult;
      lookup: LookupResult;
    }
  | { kind: "done" }
  | { kind: "error"; message: string };

export interface OnboardingProps {
  /** Called once `done` is reached so the App can re-bootstrap. */
  onComplete: () => void;
}

export function Onboarding({ onComplete }: OnboardingProps): JSX.Element {
  const [step, setStep] = useState<OnboardingStep>({ kind: "intro" });

  // T17-S-03 belt-and-braces: track any keypair we materialised for the
  // `set_passphrase` branch so it is wiped on unmount even if the user
  // closes the popup mid-flow (the secret bytes would otherwise stay
  // live on the heap until GC). The successful path also wipes inside
  // `onComplete`; on the error path the user can retry inside
  // `<SetPassphrase>` so we keep the bytes around until they leave the
  // step. Once `step.kind` changes away from `set_passphrase` (error
  // navigation, `done`, or popup close → React unmount), we wipe.
  const heldKeypairRef = useRef<Uint8Array | null>(null);
  useEffect(() => {
    if (step.kind === "set_passphrase") {
      heldKeypairRef.current = step.keypair.secret;
    } else if (heldKeypairRef.current !== null) {
      try {
        heldKeypairRef.current.fill(0);
      } catch {
        // Already wiped / non-writable; harmless.
      }
      heldKeypairRef.current = null;
    }
    return () => {
      // Component unmount → wipe whatever we last held.
      if (heldKeypairRef.current !== null) {
        try {
          heldKeypairRef.current.fill(0);
        } catch {
          // Already wiped.
        }
        heldKeypairRef.current = null;
      }
    };
  }, [step]);

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
            : (r.session.jwtExpiresAtMs(signIn.jwt) ??
              Date.now() + FALLBACK_SESSION_MS),
        signedInAt: Date.now(),
      };
      await r.session.set(session);

      if (lookup.existingPubkey === null) {
        // Server returns `link_challenge` only on this branch. Without
        // it the possession-proof step inside <SetPassphrase> cannot
        // run, so we surface a typed error rather than silently
        // skipping the link (the original blocker).
        if (!lookup.linkChallenge) {
          setStep({
            kind: "error",
            message:
              "server did not issue a link_challenge for the new-identity branch",
          });
          return;
        }
        const keypair = await loadLocalKeypair();
        if (!keypair) {
          setStep({
            kind: "error",
            message:
              "no local keypair available to encrypt — generate one before enabling Cloud sync",
          });
          return;
        }
        setStep({
          kind: "set_passphrase",
          signIn,
          lookup,
          keypair,
          linkChallenge: lookup.linkChallenge,
        });
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

  // ── Branch 1: no existing pubkey → <SetPassphrase> (wrap + upload + link)
  // The <SetPassphrase> component owns: zxcvbn ≥3 gate, confirm-match,
  // wrapSecret + uploadEscrow + POST /oauth/google/link sequence, and
  // each step's error rendering. Onboarding only supplies the signer
  // and listens for `onComplete`.

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
      <SetPassphrase
        jwt={step.signIn.jwt}
        keypair={step.keypair}
        linkChallenge={step.linkChallenge}
        signChallenge={makeSignChallenge(step.keypair)}
        onComplete={() => {
          // Best-effort wipe of the seed bytes once the wrap → upload
          // → link sequence has finished. The wipe is hygiene; JS gives
          // no guarantee — see the THREAT MODEL block in
          // `auth/key-escrow.ts`.
          try {
            step.keypair.secret.fill(0);
          } catch {
            // Already wiped or non-writable; either is harmless.
          }
          setStep({ kind: "done" });
          onComplete();
        }}
      />
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
          in a follow-up release.
        </p>
        <button
          type="button"
          onClick={() => {
            // PR134-BUG2-04: flip the storage tier to `local` BEFORE
            // signalling `onComplete`. Without this, the App would
            // re-bootstrap with `storage_tier === "cloud"` and the
            // user would land in the broken "no identity, no session"
            // state. Writing `local` first means the App's
            // post-bootstrap branch sees `tier === "local"` and skips
            // the Cloud-tier identity gate.
            const cr = (globalThis as { chrome?: typeof chrome }).chrome;
            void Promise.resolve()
              .then(async () => {
                try {
                  await cr?.storage?.local?.set({ storage_tier: "local" });
                } catch {
                  // Best-effort — the App's local-tier path is
                  // already the default, so a write failure simply
                  // leaves the user in the previous broken state
                  // (same as before this fix).
                }
              })
              .finally(() => onComplete());
          }}
          className="bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
        >
          Continue in local mode
        </button>
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

/** Bind the WASM Ed25519 signer to the freshly-loaded keypair so the
 *  `<SetPassphrase>` component can sign the server-issued link
 *  challenge without importing the WASM bridge itself. The bridge is
 *  dynamic-imported so the popup's first-paint bundle stays free of
 *  the ~200KB `mnemonic_core.wasm` glue. */
function makeSignChallenge(keypair: {
  pubkey_base58: string;
  secret: Uint8Array;
}): (nonce: Uint8Array) => Promise<Uint8Array> {
  return async (nonce: Uint8Array): Promise<Uint8Array> => {
    const { signChallenge } = await import("../runtime/sign/cose.js");
    return signChallenge(
      {
        secret: Array.from(keypair.secret),
        pubkey_base58: keypair.pubkey_base58,
      },
      nonce,
    );
  };
}

// SetPassphrase — first-time Cloud onboarding component (T17).
//
// Triggered after `signIn → lookupExisting` returns
// `{existingPubkey: null, linkChallenge: <nonce>}`. The user picks a
// recovery passphrase that becomes the wrap key for their freshly-
// minted Ed25519 secret. After successful wrap + upload + link:
//
//   1. `wrapSecret(keypair.secret, passphrase, keypair.pubkey_base58)`
//   2. `uploadEscrow(jwt, blob)`
//   3. POST /oauth/google/link with the possession-proof signature
//      over the server-issued `link_challenge`.
//
// The link step's possession-proof requires Ed25519 signing — wired
// here behind the `signChallenge` callback the host supplies (the
// onboarding container owns the WASM bridge so this component stays
// pure-presentational + bound to the T17 surface).
//
// UX rules per user-spec / task spec:
//   - Two inputs (passphrase + confirm).
//   - zxcvbn strength meter; submit disabled until score >= 3 and
//     length >= 12 (`isPassphraseAcceptable` from the meter module).
//   - Explainer copy: "Write this in your password manager. We can't
//     recover it for you."
//   - Submit button shows progress ("Encrypting…" → "Linking…").
//   - On unrecoverable error, surface a typed message and stay on the
//     form so the user can retry; passphrase fields are cleared on
//     every submit attempt so a typo isn't auto-resubmitted.

import {
  Suspense,
  lazy,
  useCallback,
  useRef,
  useState,
  type FormEvent,
  type JSX,
} from "react";
import { type EscrowBlob } from "../../auth/key-escrow.js";
import {
  MIN_PASSPHRASE_LENGTH,
  isPassphraseAcceptable,
} from "../../options/components/PassphraseStrength.js";
import { getRuntime } from "../runtime.js";

/** Inline base64 encoder. Mirrors `auth/key-escrow.ts::bytesToBase64`
 *  but kept local so this component doesn't reach into the key-escrow
 *  module's static graph (audit M3 / AUD-C-08). */
function setPpBytesToBase64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]!);
  return globalThis.btoa(bin);
}

// Lazy: the zxcvbn dictionaries are ~50KB gzip. Defer the cost until
// the user actually opens the SetPassphrase step.
const PassphraseStrength = lazy(
  () => import("../../options/components/PassphraseStrength.js"),
);

/** Identity material handed to the component. Sealed: SetPassphrase
 *  reads `secret` ONCE inside the wrap call and forwards `pubkey_base58`
 *  to both the wrap (binds the blob to the pubkey) and the link POST. */
export interface OnboardingKeypair {
  /** Raw Ed25519 secret-key bytes. The component does NOT inspect the
   *  payload — it forwards to `wrapSecret`. */
  secret: Uint8Array;
  /** Base58-encoded Ed25519 public key. Server (T15) verifies the
   *  PUT-body `pubkey_base58` against `google_identity_links`. */
  pubkey_base58: string;
}

/** Server-issued possession-proof nonce (base64). Surfaced from the
 *  prior `lookupExisting` response on the no-existing-pubkey branch. */
export type LinkChallenge = string;

/** Ed25519-signs the raw bytes of `nonce` using the host-managed key
 *  material. Returns the 64-byte signature. The host owns the WASM
 *  bridge so this surface stays decoupled from the loader. */
export type SignChallenge = (nonce: Uint8Array) => Promise<Uint8Array>;

/** Narrow seam SetPassphrase consumes — a subset of
 *  `PopupRuntime.keyEscrow`. Round-2 fix (facade): production reaches
 *  `auth/key-escrow.ts` ONLY through `popup/runtime.ts`; tests inject a
 *  stub via this prop so the wrap / upload halves can be observed
 *  without WebCrypto. */
export interface SetPassphraseKeyEscrow {
  wrap(
    secret: Uint8Array,
    passphrase: string,
    pubkeyBase58: string,
  ): Promise<EscrowBlob>;
  upload(jwt: string, blob: EscrowBlob): Promise<void>;
}

export interface SetPassphraseProps {
  /** User's cached `aud=mcp` Google-session JWT. The popup runtime
   *  internally runs the T15 extension-bootstrap handshake before
   *  any `/api/key-escrow` call (audit B1 / AUD-S-01), and forwards
   *  this same JWT verbatim to `/oauth/google/link`, which uses the
   *  standard `verify_jwt` validator (aud=mcp). */
  jwt: string;
  /** Freshly-minted (or pre-existing local) Ed25519 keypair. */
  keypair: OnboardingKeypair;
  /** Base64-encoded link challenge from `lookupExisting`. */
  linkChallenge: LinkChallenge;
  /** Host-supplied Ed25519 signer. */
  signChallenge: SignChallenge;
  /** Optional override of the MCP server origin (tests inject a stub). */
  serverOrigin?: string;
  /** Optional fetch implementation (tests inject a stub). Used for the
   *  `/oauth/google/link` POST only; key-escrow wrap+upload go through
   *  `keyEscrow`. */
  fetchImpl?: typeof fetch;
  /** Optional key-escrow facade override (tests inject a stub). Defaults
   *  to the popup runtime's `keyEscrow.{wrap,upload}` pair. */
  keyEscrow?: SetPassphraseKeyEscrow;
  /** Called once the whole wrap → upload → link sequence succeeds. */
  onComplete: () => void;
}

/** Internal step shape. Kept narrow so the renderer is a flat switch. */
type Step =
  | { kind: "form" }
  | { kind: "wrapping" }
  | { kind: "linking" }
  | { kind: "done" }
  | { kind: "error"; message: string };

export function SetPassphrase(props: SetPassphraseProps): JSX.Element {
  const {
    jwt,
    keypair,
    linkChallenge,
    signChallenge,
    serverOrigin,
    fetchImpl,
    keyEscrow,
    onComplete,
  } = props;
  const [pp1, setPp1] = useState("");
  const [pp2, setPp2] = useState("");
  const [step, setStep] = useState<Step>({ kind: "form" });

  // Resolve the key-escrow facade. Tests pass an explicit stub; the
  // popup wires the real runtime here lazily so we do not import
  // `auth/key-escrow.ts` from this module's static graph (the popup's
  // first-paint bundle stays under the 50KB size-limit budget). The
  // ref pin makes the reference stable across renders so `useCallback`
  // deps don't churn (cousin of T17-C-03 for Restore.tsx).
  const keRef = useRef<SetPassphraseKeyEscrow | null>(null);
  if (keRef.current === null) {
    keRef.current = keyEscrow ?? defaultSetPassphraseKeyEscrow();
  }
  const ke: SetPassphraseKeyEscrow = keRef.current;

  const acceptable = isPassphraseAcceptable(pp1) && pp1 === pp2;
  const submitting = step.kind === "wrapping" || step.kind === "linking";
  const submitDisabled = !acceptable || submitting;

  const onSubmit = useCallback(
    async (e: FormEvent): Promise<void> => {
      e.preventDefault();
      if (!acceptable) return;
      // Snapshot passphrase locally and immediately wipe React state so
      // the rendered <input> no longer reflects the secret while the
      // async wrap runs. The local `pp` binding goes out of scope when
      // this function returns.
      const pp = pp1;
      setPp1("");
      setPp2("");
      setStep({ kind: "wrapping" });

      let blob: EscrowBlob;
      try {
        blob = await ke.wrap(keypair.secret, pp, keypair.pubkey_base58);
      } catch (err) {
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Encrypt failed: ${err.message}`
              : "Encrypt failed",
        });
        return;
      }
      try {
        await ke.upload(jwt, blob);
      } catch (err) {
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Upload failed: ${err.message}`
              : "Upload failed",
        });
        return;
      }

      // Link step: sign the raw bytes of `linkChallenge` and POST.
      setStep({ kind: "linking" });
      let nonceBytes: Uint8Array;
      try {
        const bin = globalThis.atob(linkChallenge);
        nonceBytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) nonceBytes[i] = bin.charCodeAt(i);
      } catch {
        setStep({
          kind: "error",
          message: "Link failed: server-issued challenge is not valid base64",
        });
        return;
      }
      let signature: Uint8Array;
      try {
        signature = await signChallenge(nonceBytes);
      } catch (err) {
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Link failed: signing challenge — ${err.message}`
              : "Link failed: signing challenge",
        });
        return;
      }
      try {
        const linkBody = {
          pubkey_base58: keypair.pubkey_base58,
          possession_proof_base64: setPpBytesToBase64(signature),
          challenge: linkChallenge,
        };
        if (fetchImpl !== undefined || serverOrigin !== undefined) {
          // Legacy test seam (kept for back-compat: existing component
          // tests inject `{ fetchImpl, serverOrigin }` and observe the
          // raw `/oauth/google/link` POST). Production callers omit
          // both and we route through the runtime facade (M3).
          const { linkGoogleAccount } = await import("../../auth/link.js");
          const linkOpts: { serverOrigin?: string; fetchImpl?: typeof fetch } =
            {};
          if (serverOrigin !== undefined) linkOpts.serverOrigin = serverOrigin;
          if (fetchImpl !== undefined) linkOpts.fetchImpl = fetchImpl;
          await linkGoogleAccount(jwt, linkBody, linkOpts);
        } else {
          await getRuntime().auth.linkGoogle(jwt, linkBody);
        }
      } catch (err) {
        setStep({
          kind: "error",
          message:
            err instanceof Error
              ? `Link failed: ${err.message}`
              : "Link failed",
        });
        return;
      }
      setStep({ kind: "done" });
      onComplete();
    },
    [
      acceptable,
      pp1,
      keypair,
      jwt,
      ke,
      linkChallenge,
      signChallenge,
      serverOrigin,
      fetchImpl,
      onComplete,
    ],
  );

  return (
    <main className="bg-background text-text-primary p-4 flex flex-col gap-3 min-h-full">
      <h1 className="text-sm text-accent-primary font-semibold tracking-wide">
        Set recovery passphrase
      </h1>
      <p className="text-xs text-text-muted">
        Write this in your password manager. We can&apos;t recover it for you.
        Your passphrase encrypts your identity key — without it, you cannot
        restore your memories on a new device. Minimum {MIN_PASSPHRASE_LENGTH}{" "}
        characters and a Strong strength rating.
      </p>
      <form onSubmit={(e) => void onSubmit(e)} className="flex flex-col gap-2">
        <label
          htmlFor="set-pp"
          className="text-[11px] text-text-muted font-mono"
        >
          Recovery passphrase
        </label>
        <input
          id="set-pp"
          type="password"
          autoComplete="new-password"
          value={pp1}
          onChange={(e) => setPp1(e.target.value)}
          disabled={submitting}
          className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono"
        />
        <Suspense
          fallback={
            <div className="text-[10px] text-text-muted font-mono">
              Loading strength meter…
            </div>
          }
        >
          <PassphraseStrength value={pp1} />
        </Suspense>
        <label
          htmlFor="set-pp-confirm"
          className="text-[11px] text-text-muted font-mono"
        >
          Confirm passphrase
        </label>
        <input
          id="set-pp-confirm"
          type="password"
          autoComplete="new-password"
          value={pp2}
          onChange={(e) => setPp2(e.target.value)}
          disabled={submitting}
          className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono"
        />
        {pp2.length > 0 && pp1 !== pp2 ? (
          <p className="text-[11px] text-error" role="alert">
            Confirmation does not match.
          </p>
        ) : null}
        <button
          type="submit"
          disabled={submitDisabled}
          className="self-start bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
        >
          {step.kind === "wrapping"
            ? "Encrypting…"
            : step.kind === "linking"
              ? "Linking…"
              : "Encrypt & upload"}
        </button>
        {step.kind === "error" ? (
          <p className="text-[11px] text-error" role="alert">
            {step.message}
          </p>
        ) : null}
      </form>
    </main>
  );
}

// The legacy inline `linkGoogle` implementation moved to
// `src/auth/link.ts::linkGoogleAccount` (audit M3 / AUD-C-08). Tests
// that still pass `{ fetchImpl, serverOrigin }` go through the same
// module via a dynamic import in `onSubmit` above.

/** Default key-escrow facade — production passes nothing and we route
 *  through the popup runtime's `keyEscrow.{wrap,upload}` pair. Tests
 *  inject a stub via the `keyEscrow` prop so the wrap / upload halves
 *  are observable without WebCrypto. */
function defaultSetPassphraseKeyEscrow(): SetPassphraseKeyEscrow {
  return {
    async wrap(secret, passphrase, pubkeyBase58) {
      return getRuntime().keyEscrow.wrap(secret, passphrase, pubkeyBase58);
    },
    async upload(jwt, blob) {
      return getRuntime().keyEscrow.upload(jwt, blob);
    },
  };
}

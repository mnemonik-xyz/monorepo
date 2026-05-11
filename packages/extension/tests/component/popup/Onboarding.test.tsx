// Onboarding-orchestrator integration tests (T21). Binds the two TDD
// anchors from the task spec:
//
//   1. fresh_user_branch_wraps_and_uploads — `lookupExisting` returns
//      `{existingPubkey: null, linkChallenge: <nonce>}`; the orchestrator
//      mounts <SetPassphrase>; a strong passphrase drives the
//      wrap → upload → link sequence; we assert each runtime seam was
//      called exactly once and the link POST carries `aud=extension`-
//      ready JWT (passed verbatim because the runtime facade owns the
//      bootstrap handshake — see audit B1 / AUD-S-01).
//
//   2. welcome_back_branch_persists_identity — `lookupExisting` returns
//      `{existingPubkey, escrowPresent: true}`; the orchestrator mounts
//      <Restore>; the user submits the correct passphrase; we assert
//      `keyEscrow.fetch` was called exactly ONCE (per-attempt re-fetch
//      regression-guard from T17-C-02 / T17-S-01 / AUD-S-01) and the
//      decrypted keypair lands in `chrome.storage.local`.
//
// Branch 3 (existing pubkey + no escrow) renders the "manual import"
// edge-case panel with the "Continue in local mode" button.
//
// Branch-reset case: after a typed error in the sign-in step, the user
// can click "Try again" and return to the intro panel — no leaked
// state from the prior branch.
//
// Runtime injection mirrors the Capture / Restore / SetPassphrase test
// pattern: `setRuntime(stub)` in `beforeEach`, `setRuntime(null)` in
// `afterEach`. The WASM signer is stubbed via `__setWasmForTesting`
// (Onboarding's internal `makeSignChallenge` dynamic-imports
// `runtime/sign/cose.ts`).

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Onboarding } from "../../../src/popup/Onboarding.js";
import { setRuntime, type PopupRuntime } from "../../../src/popup/runtime.js";
import {
  IDENTITY_STORAGE_KEY,
  IDENTITY_SECRET_STORAGE_KEY,
} from "../../../src/popup/onboarding/Restore.js";
import { __setWasmForTesting } from "../../../src/runtime/sign/cose.js";
import type {
  GoogleSignInResult,
  LookupResult,
  Session,
} from "../../../src/auth/types.js";
import type { EscrowBlob } from "../../../src/auth/key-escrow.js";

// ── Fixtures ────────────────────────────────────────────────────────────────

const STRONG_PASSPHRASE = "correct horse battery staple 0xFA1";
const EXT_JWT = "ext.header.payload.signature";
const EXISTING_PUBKEY = "PubExistingABC123";
const PRESEEDED_PUBKEY = "PubLocalSeed987";

function makeSignInResult(): GoogleSignInResult {
  return {
    jwt: EXT_JWT,
    googleSub: "google-sub-12345",
    profile: {
      sub: "google-sub-12345",
      email: "user@example.com",
      email_verified: true,
      name: "Test User",
      picture: "",
    },
    jwtExpiresAt: Date.now() + 60 * 60 * 1000,
  };
}

function makeSecret(): Uint8Array {
  const secret = new Uint8Array(64);
  for (let i = 0; i < 64; i++) secret[i] = (i * 7 + 1) & 0xff;
  return secret;
}

function makeBlob(pubkey: string): EscrowBlob {
  return {
    ciphertext_b64: "AAEC",
    nonce_b64: "AAECAwQFBgcICQoL",
    kdf: "argon2id",
    kdf_params: JSON.stringify({ salt_b64: "AAAA", m: 8, t: 1, p: 1 }),
    pubkey_base58: pubkey,
  };
}

/** Build a chrome.storage.local-backed Map stub. Tests pre-seed the
 *  identity keypair for branch 1 (so `loadLocalKeypair` returns
 *  non-null) and observe writes from Restore's persistence step in
 *  branch 2. */
function installChromeStub(initial: Record<string, unknown> = {}): {
  store: Map<string, unknown>;
} {
  const store = new Map<string, unknown>(Object.entries(initial));
  const get = async (
    keys: string | string[],
  ): Promise<Record<string, unknown>> => {
    const list = typeof keys === "string" ? [keys] : keys;
    const out: Record<string, unknown> = {};
    for (const k of list) {
      if (store.has(k)) out[k] = store.get(k);
    }
    return out;
  };
  const set = async (items: Record<string, unknown>): Promise<void> => {
    for (const [k, v] of Object.entries(items)) store.set(k, v);
  };
  (globalThis as { chrome?: unknown }).chrome = {
    tabs: {
      query: async () => [],
      sendMessage: async () => undefined,
    },
    storage: {
      local: { get, set },
      sync: { get: async () => ({}), set: async () => undefined },
    },
    runtime: {
      sendMessage: async () => undefined,
      openOptionsPage: async () => undefined,
    },
  };
  return { store };
}

/** Stub WASM signer used by Onboarding's internal `makeSignChallenge`
 *  (the dynamic-imported `runtime/sign/cose.ts` calls
 *  `wasm.sign_challenge`). Returns a deterministic 64-byte signature so
 *  branch-1 tests can assert the link POST's `possession_proof_base64`
 *  is non-empty without exercising the real Ed25519 path. */
function installWasmSignerStub(): void {
  __setWasmForTesting({
    default: () => Promise.resolve(undefined),
    generate_keypair: () => ({}),
    sign_challenge: (_kp: unknown, _bytes: Uint8Array) => {
      const sig = new Uint8Array(64);
      for (let i = 0; i < 64; i++) sig[i] = (i + 11) & 0xff;
      return sig;
    },
    sign_cose_payload: () => new Uint8Array([0x84]),
    import_keypair_json: () => ({}),
    export_keypair_json: () => "{}",
    compress_embedding: () => new Uint8Array(0),
    decompress_embedding: () => new Float32Array(0),
    to_canonical_cbor_bytes: () => new Uint8Array(0),
    blake3_hash: () => new Uint8Array(32),
    blake3_hash_hex: () => "00".repeat(32),
  } as unknown as Parameters<typeof __setWasmForTesting>[0]);
}

/** Minimal runtime stub. Per-test `overrides` carry the auth /
 *  keyEscrow seams the assertion targets. */
function makeRuntime(overrides: {
  auth?: Partial<PopupRuntime["auth"]>;
  session?: Partial<PopupRuntime["session"]>;
  keyEscrow?: Partial<PopupRuntime["keyEscrow"]>;
}): PopupRuntime {
  const throwingAuth: PopupRuntime["auth"] = {
    signIn: async () => {
      throw new Error("auth.signIn stub: not configured");
    },
    lookupExisting: async () => {
      throw new Error("auth.lookupExisting stub: not configured");
    },
    ensureExtensionJwt: async () => EXT_JWT,
    linkGoogle: async () => {
      throw new Error("auth.linkGoogle stub: not configured");
    },
  };
  const session: PopupRuntime["session"] = {
    get: async () => null,
    set: async () => undefined,
    clear: async () => undefined,
    jwtExpiresAtMs: () => null,
  };
  const throwingEscrow: PopupRuntime["keyEscrow"] = {
    wrap: async () => {
      throw new Error("keyEscrow.wrap stub: not configured");
    },
    unwrap: async () => {
      throw new Error("keyEscrow.unwrap stub: not configured");
    },
    upload: async () => {
      throw new Error("keyEscrow.upload stub: not configured");
    },
    fetch: async () => {
      throw new Error("keyEscrow.fetch stub: not configured");
    },
    delete: async () => undefined,
    rotate: async () => undefined,
    hasBlob: async () => false,
  };
  return {
    loadIdentity: async () => null,
    loadStorageTier: async () => "local" as const,
    getActiveTabAdapter: async () => null,
    getActiveTabSelection: async () => "",
    getActiveTabConversation: async () => null,
    signMemory: async () => {
      throw new Error("signMemory stub: not configured");
    },
    signRemote: async () => undefined,
    recall: async () => [],
    verify: async () => ({ status: "not_found" }),
    auth: { ...throwingAuth, ...overrides.auth },
    session: { ...session, ...overrides.session },
    cloudSync: {
      signRemote: async () => null,
      recallRemote: async () => null,
      verifyRemote: async () => null,
    },
    keyEscrow: { ...throwingEscrow, ...overrides.keyEscrow },
  };
}

// ── Setup / teardown ────────────────────────────────────────────────────────

beforeEach(() => {
  setRuntime(null);
  installWasmSignerStub();
});

afterEach(() => {
  setRuntime(null);
  __setWasmForTesting(null);
});

// ── Branch 1 — fresh user (TDD anchor: fresh_user_branch_wraps_and_uploads)

describe("Onboarding — fresh user (no existingPubkey)", () => {
  it("mounts SetPassphrase and runs wrap → upload → link exactly once each", async () => {
    // Pre-seed identity so loadLocalKeypair() succeeds — the orchestrator
    // requires a local keypair before mounting <SetPassphrase>.
    const secretBytes = makeSecret();
    installChromeStub({
      [IDENTITY_STORAGE_KEY]: { pubkey_base58: PRESEEDED_PUBKEY },
      [IDENTITY_SECRET_STORAGE_KEY]: Array.from(secretBytes),
    });

    const signIn = vi
      .fn<[], Promise<GoogleSignInResult>>()
      .mockResolvedValue(makeSignInResult());
    const linkChallenge = btoa("link-challenge-nonce-XYZ");
    const lookupExisting = vi
      .fn<[string], Promise<LookupResult>>()
      .mockResolvedValue({
        existingPubkey: null,
        escrowPresent: false,
        linkChallenge,
      });
    const sessionSet = vi.fn<[Session], Promise<void>>().mockResolvedValue();
    const linkGoogle = vi
      .fn<
        [
          string,
          {
            pubkey_base58: string;
            possession_proof_base64: string;
            challenge: string;
          },
        ],
        Promise<void>
      >()
      .mockResolvedValue();
    // wrap snapshots the secret bytes synchronously — Onboarding's
    // `onComplete` callback wipes `step.keypair.secret.fill(0)` once
    // the full sequence resolves, and `vi.fn().mock.calls` holds the
    // SAME Uint8Array reference, so a deferred assertion would see all
    // zeros. The snapshot freezes the inputs at call time.
    const wrappedSecretSnapshots: number[][] = [];
    const wrap = vi
      .fn<[Uint8Array, string, string], Promise<EscrowBlob>>()
      .mockImplementation(async (secret, _pp, _pub) => {
        wrappedSecretSnapshots.push(Array.from(secret));
        return makeBlob(PRESEEDED_PUBKEY);
      });
    const upload = vi
      .fn<[string, EscrowBlob], Promise<void>>()
      .mockResolvedValue();

    setRuntime(
      makeRuntime({
        auth: { signIn, lookupExisting, linkGoogle },
        session: { set: sessionSet },
        keyEscrow: { wrap, upload },
      }),
    );

    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    // Drive the intro → signing_in → set_passphrase transition.
    fireEvent.click(
      await screen.findByRole("button", { name: /Sign in with Google/i }),
    );

    // SetPassphrase mounted: the "Set recovery passphrase" heading,
    // the two password inputs, and the zxcvbn meter region all appear.
    const pp1 = (await screen.findByLabelText(
      /Recovery passphrase/i,
    )) as HTMLInputElement;
    const pp2 = (await screen.findByLabelText(
      /Confirm passphrase/i,
    )) as HTMLInputElement;
    expect(
      screen.getByRole("heading", { name: /Set recovery passphrase/i }),
    ).toBeInTheDocument();

    // Type a strong passphrase that meets the zxcvbn ≥ 3 AND length ≥
    // 12 gate. The matching confirm enables the submit button.
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: STRONG_PASSPHRASE } });
    const submit = (await screen.findByRole("button", {
      name: /Encrypt & upload/i,
    })) as HTMLButtonElement;
    await waitFor(() => {
      expect(submit.disabled).toBe(false);
    });

    fireEvent.click(submit);

    // The whole sequence must finish before onComplete fires.
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });

    // 1. wrap was called once with the loaded keypair's secret + the
    //    user's passphrase + the loaded pubkey. We assert against the
    //    snapshot captured inside the mock impl — the live Uint8Array
    //    reference in `wrap.mock.calls[0]` is zeroed by Onboarding's
    //    `onComplete` callback once the sequence resolves.
    expect(wrap).toHaveBeenCalledTimes(1);
    expect(wrappedSecretSnapshots).toHaveLength(1);
    expect(wrappedSecretSnapshots[0]).toEqual(Array.from(secretBytes));
    const wrapArgs = wrap.mock.calls[0]!;
    expect(wrapArgs[1]).toBe(STRONG_PASSPHRASE);
    expect(wrapArgs[2]).toBe(PRESEEDED_PUBKEY);

    // 2. upload was called once with the resulting blob and the
    //    `aud=mcp` JWT (the runtime facade owns the extension-JWT
    //    bootstrap, audit B1 / AUD-S-01 — so we pass EXT_JWT through
    //    verbatim and the upload-seam mock observes that same value).
    expect(upload).toHaveBeenCalledTimes(1);
    expect(upload.mock.calls[0]![0]).toBe(EXT_JWT);
    expect(upload.mock.calls[0]![1].pubkey_base58).toBe(PRESEEDED_PUBKEY);

    // 3. linkGoogle was called once with the link-challenge nonce, the
    //    base64-encoded possession-proof, and the same EXT_JWT.
    expect(linkGoogle).toHaveBeenCalledTimes(1);
    expect(linkGoogle.mock.calls[0]![0]).toBe(EXT_JWT);
    const linkBody = linkGoogle.mock.calls[0]![1];
    expect(linkBody.pubkey_base58).toBe(PRESEEDED_PUBKEY);
    expect(linkBody.challenge).toBe(linkChallenge);
    expect(typeof linkBody.possession_proof_base64).toBe("string");
    expect(linkBody.possession_proof_base64.length).toBeGreaterThan(0);

    // 4. Call ORDER: wrap before upload before linkGoogle. We compare
    //    invocationCallOrder so this binding survives even when the
    //    component implementation re-orders local state mutations.
    expect(wrap.mock.invocationCallOrder[0]!).toBeLessThan(
      upload.mock.invocationCallOrder[0]!,
    );
    expect(upload.mock.invocationCallOrder[0]!).toBeLessThan(
      linkGoogle.mock.invocationCallOrder[0]!,
    );

    // 5. The session was persisted exactly once (the sign-in handler
    //    bookkeeping that mints the popup's `session.v1` blob).
    expect(sessionSet).toHaveBeenCalledTimes(1);
    expect(sessionSet.mock.calls[0]![0].jwt).toBe(EXT_JWT);
  });

  it("surfaces a typed error and stays signed-out when no linkChallenge is issued", async () => {
    installChromeStub({
      [IDENTITY_STORAGE_KEY]: { pubkey_base58: PRESEEDED_PUBKEY },
      [IDENTITY_SECRET_STORAGE_KEY]: Array.from(makeSecret()),
    });
    setRuntime(
      makeRuntime({
        auth: {
          signIn: async () => makeSignInResult(),
          // Server bug: returns existingPubkey=null but no challenge.
          lookupExisting: async () => ({
            existingPubkey: null,
            escrowPresent: false,
            linkChallenge: null,
          }),
        },
        session: { set: async () => undefined },
      }),
    );

    render(<Onboarding onComplete={() => undefined} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /Sign in with Google/i }),
    );

    // Surface the typed error — the orchestrator must NOT silently fall
    // through to <SetPassphrase>.
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/link_challenge/i);
    // SetPassphrase must NOT have mounted.
    expect(
      screen.queryByRole("heading", { name: /Set recovery passphrase/i }),
    ).not.toBeInTheDocument();
  });
});

// ── Branch 2 — welcome back (TDD anchor: welcome_back_branch_persists_identity)

describe("Onboarding — welcome back (existingPubkey + escrowPresent)", () => {
  it("mounts Restore and persists the recovered identity (single fetch)", async () => {
    const { store } = installChromeStub();
    const recoveredSecret = makeSecret();

    const signIn = vi
      .fn<[], Promise<GoogleSignInResult>>()
      .mockResolvedValue(makeSignInResult());
    const lookupExisting = vi
      .fn<[string], Promise<LookupResult>>()
      .mockResolvedValue({
        existingPubkey: EXISTING_PUBKEY,
        escrowPresent: true,
        linkChallenge: null,
      });
    const fetchEscrow = vi
      .fn<[string], Promise<EscrowBlob>>()
      .mockResolvedValue(makeBlob(EXISTING_PUBKEY));
    // unwrap returns a FRESH copy so Restore's post-persist
    // `secret.fill(0)` wipe leaves our test fixture intact for the
    // post-condition assertion.
    const unwrapSecret = vi
      .fn<[EscrowBlob, string], Promise<Uint8Array>>()
      .mockImplementation(async () => new Uint8Array(recoveredSecret));

    setRuntime(
      makeRuntime({
        auth: { signIn, lookupExisting },
        session: { set: async () => undefined },
        keyEscrow: { fetch: fetchEscrow, unwrap: unwrapSecret },
      }),
    );

    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    fireEvent.click(
      await screen.findByRole("button", { name: /Sign in with Google/i }),
    );

    // Restore mounted: the "Welcome back" headline tells us which
    // branch is active (the only branch in the state machine that
    // renders this exact title).
    expect(
      await screen.findByRole("heading", { name: /Welcome back/i }),
    ).toBeInTheDocument();

    const input = (await screen.findByTestId(
      "restore-pp-input",
    )) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "user-passphrase-correct" } });
    fireEvent.click(screen.getByRole("button", { name: /^Restore/ }));

    // onComplete fires only after the keypair is persisted.
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });

    // AUD-S-01 / T17-C-02 regression guard: per-attempt re-fetch must
    // NOT have happened — exactly one server fetch for the whole flow.
    expect(fetchEscrow).toHaveBeenCalledTimes(1);
    expect(fetchEscrow.mock.calls[0]![0]).toBe(EXT_JWT);
    // Unwrap was called once with the cached blob.
    expect(unwrapSecret).toHaveBeenCalledTimes(1);
    expect(unwrapSecret.mock.calls[0]![1]).toBe("user-passphrase-correct");

    // chrome.storage.local.set received both the `identity` metadata
    // and the `identity_secret` array (mirrors the layout
    // `runtime-impl.ts::loadIdentity` reads).
    const persistedIdentity = store.get(IDENTITY_STORAGE_KEY) as
      | { pubkey_base58: string }
      | undefined;
    const persistedSecret = store.get(IDENTITY_SECRET_STORAGE_KEY) as
      | number[]
      | undefined;
    expect(persistedIdentity).toBeDefined();
    expect(persistedIdentity?.pubkey_base58).toBe(EXISTING_PUBKEY);
    expect(persistedSecret).toBeDefined();
    expect(persistedSecret!.length).toBe(recoveredSecret.length);
    expect(Uint8Array.from(persistedSecret!)).toEqual(recoveredSecret);
  });
});

// ── Branch 3 — existing identity but no escrow ─────────────────────────────

describe("Onboarding — existing identity, no escrow", () => {
  it("renders the manual-import edge panel and fires onComplete on local-mode", async () => {
    installChromeStub();
    setRuntime(
      makeRuntime({
        auth: {
          signIn: async () => makeSignInResult(),
          lookupExisting: async () => ({
            existingPubkey: EXISTING_PUBKEY,
            escrowPresent: false,
            linkChallenge: null,
          }),
        },
        session: { set: async () => undefined },
      }),
    );

    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    fireEvent.click(
      await screen.findByRole("button", { name: /Sign in with Google/i }),
    );

    // The edge-case frame uses the "No escrow on server" headline.
    expect(
      await screen.findByRole("heading", { name: /No escrow on server/i }),
    ).toBeInTheDocument();

    // The truncated pubkey appears in the explainer copy.
    expect(screen.getByText(/PubExist…/i)).toBeInTheDocument();

    // "Continue in local mode" is the only actionable button and it
    // fires onComplete so the popup re-bootstraps in Local tier.
    const localBtn = screen.getByRole("button", {
      name: /Continue in local mode/i,
    });
    fireEvent.click(localBtn);
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });
  });

  it("writes storage_tier=local before firing onComplete (BUG2-04)", async () => {
    // Regression: without the explicit tier flip, App re-bootstraps
    // in cloud mode with no identity and surfaces "identity not
    // configured" on the next sign. The button MUST write
    // `storage_tier: 'local'` to chrome.storage.local first.
    const { store } = installChromeStub({ storage_tier: "cloud" });
    setRuntime(
      makeRuntime({
        auth: {
          signIn: async () => makeSignInResult(),
          lookupExisting: async () => ({
            existingPubkey: EXISTING_PUBKEY,
            escrowPresent: false,
            linkChallenge: null,
          }),
        },
        session: { set: async () => undefined },
      }),
    );

    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    fireEvent.click(
      await screen.findByRole("button", { name: /Sign in with Google/i }),
    );
    await screen.findByRole("heading", { name: /No escrow on server/i });

    const localBtn = screen.getByRole("button", {
      name: /Continue in local mode/i,
    });
    fireEvent.click(localBtn);
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });
    expect(store.get("storage_tier")).toBe("local");
  });
});

// ── Branch reset — error in sign-in then retry ─────────────────────────────

describe("Onboarding — branch reset after error", () => {
  it("'Try again' returns to the intro panel without leaking the prior step", async () => {
    installChromeStub();

    // signIn rejects on the first call, succeeds on the second.
    let attempts = 0;
    const signIn = vi.fn<[], Promise<GoogleSignInResult>>(async () => {
      attempts++;
      if (attempts === 1) throw new Error("popup closed");
      return makeSignInResult();
    });
    setRuntime(
      makeRuntime({
        auth: {
          signIn,
          lookupExisting: async () => ({
            existingPubkey: EXISTING_PUBKEY,
            escrowPresent: false,
            linkChallenge: null,
          }),
        },
        session: { set: async () => undefined },
      }),
    );

    render(<Onboarding onComplete={() => undefined} />);

    fireEvent.click(
      await screen.findByRole("button", { name: /Sign in with Google/i }),
    );

    // Error frame surfaces.
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/popup closed/);
    expect(
      screen.getByRole("heading", { name: /Sign-in failed/i }),
    ).toBeInTheDocument();

    // Click "Try again" → orchestrator returns to the intro panel.
    fireEvent.click(screen.getByRole("button", { name: /Try again/i }));

    expect(
      await screen.findByRole("heading", { name: /Welcome to Mnemonik/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Sign in with Google/i }),
    ).toBeInTheDocument();

    // Retry the sign-in: now succeeds and lands on the no-escrow edge.
    fireEvent.click(
      screen.getByRole("button", { name: /Sign in with Google/i }),
    );
    expect(
      await screen.findByRole("heading", { name: /No escrow on server/i }),
    ).toBeInTheDocument();
    expect(signIn).toHaveBeenCalledTimes(2);
  });
});

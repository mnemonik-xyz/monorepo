// T25 — Onboarding auto-generates a keypair on the fresh-user branch.
//
// Pre-T25 behaviour: when Google sign-in returned a `null` existing
// pubkey AND `chrome.storage.local` had no `identity` / `identity_
// secret` blob, the orchestrator landed on a typed error
// ("no local keypair available to encrypt — generate one before
// enabling Cloud sync"). That blocked every first-time user — the
// popup has no manual mint-keypair UI to break the deadlock.
//
// T25 wires the WASM `generate_keypair` export through Onboarding so a
// fresh user gets a keypair seeded into chrome.storage.local
// transparently, then the flow continues into <SetPassphrase>.
//
// This test pins both halves of that contract:
//
//   1. with empty storage, the orchestrator successfully reaches the
//      <SetPassphrase> step (no error frame); and
//   2. by the time <SetPassphrase> renders, chrome.storage.local has
//      the canonical `identity = {pubkey_base58, created_at}` +
//      `identity_secret = number[64]` pair.
//
// The WASM `generate_keypair` export is stubbed via
// `__setWasmForTesting` so we never load the real wasm binary in this
// jsdom env.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { Onboarding } from "../../../src/popup/Onboarding.js";
import { setRuntime, type PopupRuntime } from "../../../src/popup/runtime.js";
import {
  IDENTITY_KEY,
  IDENTITY_SECRET_KEY,
} from "../../../src/auth/storage-keys.js";
import { __setWasmForTesting } from "../../../src/runtime/sign/cose.js";
import type {
  GoogleSignInResult,
  LookupResult,
  Session,
} from "../../../src/auth/types.js";

const FRESH_PUBKEY = "AutogenPubKey0xCAFEBABE";

function makeSignInResult(): GoogleSignInResult {
  return {
    jwt: "ext.h.p.s",
    googleSub: "google-sub-NEW",
    profile: {
      sub: "google-sub-NEW",
      email: "new@example.com",
      email_verified: true,
      name: "New User",
      picture: "",
    },
    jwtExpiresAt: Date.now() + 60 * 60 * 1000,
  };
}

function installChromeStub(): { store: Map<string, unknown> } {
  const store = new Map<string, unknown>();
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
      local: { get, set, remove: async () => undefined },
      sync: { get: async () => ({}), set: async () => undefined },
    },
    runtime: {
      sendMessage: async () => undefined,
      openOptionsPage: async () => undefined,
    },
  };
  return { store };
}

function installWasmStubWithGenerate(pubkey: string): void {
  // 64-byte deterministic secret so the test assertions can read the
  // exact bytes back out of chrome.storage. We never log this — only
  // the test reads it.
  const secret: number[] = [];
  for (let i = 0; i < 64; i++) secret.push((i * 13 + 7) & 0xff);
  __setWasmForTesting({
    default: () => Promise.resolve(undefined),
    generate_keypair: () => ({ pubkey_base58: pubkey, secret }),
    sign_challenge: () => new Uint8Array(64),
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

function makeRuntime(overrides: {
  signIn: () => Promise<GoogleSignInResult>;
  lookupExisting: () => Promise<LookupResult>;
}): PopupRuntime {
  const session: PopupRuntime["session"] = {
    get: async () => null,
    set: async () => undefined,
    clear: async () => undefined,
    jwtExpiresAtMs: () => null,
  };
  return {
    loadIdentity: async () => null,
    loadStorageTier: async () => "local",
    getActiveTabAdapter: async () => null,
    getActiveTabSelection: async () => "",
    getActiveTabConversation: async () => null,
    signMemory: async () => {
      throw new Error("not used");
    },
    signRemote: async () => undefined,
    recall: async () => [],
    verify: async () => ({ status: "not_found" }),
    auth: {
      signIn: overrides.signIn,
      lookupExisting: overrides.lookupExisting,
      ensureExtensionJwt: async () => "ext.jwt",
      linkGoogle: async () => undefined,
    },
    session,
    cloudSync: {
      signRemote: async () => null,
      recallRemote: async () => null,
      verifyRemote: async () => null,
    },
    keyEscrow: {
      wrap: async () => ({
        ciphertext_b64: "AAEC",
        nonce_b64: "AAECAwQFBgcICQoL",
        kdf: "argon2id",
        kdf_params: JSON.stringify({ salt_b64: "AAAA", m: 8, t: 1, p: 1 }),
        pubkey_base58: FRESH_PUBKEY,
      }),
      unwrap: async () => new Uint8Array(64),
      upload: async () => undefined,
      fetch: async () => {
        throw new Error("not used");
      },
      delete: async () => undefined,
      rotate: async () => undefined,
      hasBlob: async () => false,
    },
  };
}

describe("Onboarding — T25 auto-generate identity on fresh sign-in", () => {
  beforeEach(() => {
    setRuntime(null);
  });
  afterEach(() => {
    setRuntime(null);
    __setWasmForTesting(null);
  });

  it("mints + persists a keypair when none exists and reaches set_passphrase", async () => {
    const { store } = installChromeStub();
    installWasmStubWithGenerate(FRESH_PUBKEY);

    // Sanity: storage starts empty.
    expect(store.has(IDENTITY_KEY)).toBe(false);
    expect(store.has(IDENTITY_SECRET_KEY)).toBe(false);

    const signIn = vi
      .fn<[], Promise<GoogleSignInResult>>()
      .mockResolvedValue(makeSignInResult());
    const linkChallenge = btoa("link-challenge-XYZ");
    const lookupExisting = vi
      .fn<[string], Promise<LookupResult>>()
      .mockResolvedValue({
        existingPubkey: null,
        escrowPresent: false,
        linkChallenge,
      });
    setRuntime(makeRuntime({ signIn, lookupExisting }));

    render(<Onboarding onComplete={() => undefined} />);

    fireEvent.click(
      screen.getByRole("button", { name: /Sign in with Google/i }),
    );

    // Either the SetPassphrase headline renders (happy path) OR an
    // error frame surfaces. We assert the happy path.
    await waitFor(
      () => {
        expect(
          screen.queryByText(/Set recovery passphrase/i),
        ).toBeInTheDocument();
      },
      { timeout: 3000 },
    );

    // chrome.storage.local now holds the canonical identity layout.
    const identity = store.get(IDENTITY_KEY) as
      | { pubkey_base58?: string; created_at?: number }
      | undefined;
    expect(identity?.pubkey_base58).toBe(FRESH_PUBKEY);
    expect(typeof identity?.created_at).toBe("number");
    const secret = store.get(IDENTITY_SECRET_KEY);
    expect(Array.isArray(secret)).toBe(true);
    expect((secret as number[]).length).toBe(64);
    // Sanity check on the byte range — keep a structured-clone bug
    // from silently shipping a wrong-shape secret.
    for (const n of secret as number[]) {
      expect(Number.isInteger(n)).toBe(true);
      expect(n).toBeGreaterThanOrEqual(0);
      expect(n).toBeLessThanOrEqual(255);
    }
  });

  // PR136-C-03: regression-lock the "do NOT autogen when a keypair
  // already exists" half of the contract. The pre-T25 deadlock was on
  // fresh-user storage; if a future commit inverts the conditional
  // (e.g. `if (keypair)` instead of `if (!keypair)`), it would silently
  // overwrite the existing keypair on every sign-in. This test fails
  // loudly in that case.
  it("does NOT regenerate the keypair when one already exists in storage", async () => {
    const { store } = installChromeStub();
    // Pre-seed canonical identity layout: a 64-byte secret + the
    // public-half row. The auto-gen path must NOT touch either.
    const PRE_SEEDED_PUBKEY = "PreSeededPubKey0xCAFE";
    const preSeededSecret: number[] = [];
    for (let i = 0; i < 64; i++) preSeededSecret.push((i * 7 + 3) & 0xff);
    store.set(IDENTITY_KEY, {
      pubkey_base58: PRE_SEEDED_PUBKEY,
      created_at: 1700000000000,
    });
    store.set(IDENTITY_SECRET_KEY, preSeededSecret);

    // Install a WASM stub that would surface a different pubkey if the
    // autogen path fires by mistake — the assertion compares the
    // stored pubkey against PRE_SEEDED_PUBKEY, so this drift is what
    // makes the test definitive.
    const generateKeypairSpy = vi.fn(() => ({
      pubkey_base58: "SHOULD_NOT_BE_USED",
      secret: Array.from({ length: 64 }, () => 0),
    }));
    __setWasmForTesting({
      default: () => Promise.resolve(undefined),
      generate_keypair: generateKeypairSpy,
      sign_challenge: () => new Uint8Array(64),
      sign_cose_payload: () => new Uint8Array([0x84]),
      import_keypair_json: () => ({}),
      export_keypair_json: () => "{}",
      compress_embedding: () => new Uint8Array(0),
      decompress_embedding: () => new Float32Array(0),
      to_canonical_cbor_bytes: () => new Uint8Array(0),
      blake3_hash: () => new Uint8Array(32),
      blake3_hash_hex: () => "00".repeat(32),
    } as unknown as Parameters<typeof __setWasmForTesting>[0]);

    const signIn = vi
      .fn<[], Promise<GoogleSignInResult>>()
      .mockResolvedValue(makeSignInResult());
    // Fresh-user branch (existingPubkey === null) — exactly the branch
    // that triggers mintAndPersistKeypair in the pre-T25 contract. The
    // guard prevents autogen ONLY when storage already has a keypair.
    const lookupExisting = vi
      .fn<[string], Promise<LookupResult>>()
      .mockResolvedValue({
        existingPubkey: null,
        escrowPresent: false,
        linkChallenge: btoa("link-challenge-XYZ"),
      });
    setRuntime(makeRuntime({ signIn, lookupExisting }));

    render(<Onboarding onComplete={() => undefined} />);
    fireEvent.click(
      screen.getByRole("button", { name: /Sign in with Google/i }),
    );

    // Wait until the flow reaches SetPassphrase so we know the
    // orchestrator has finished its sign-in branch.
    await waitFor(
      () => {
        expect(
          screen.queryByText(/Set recovery passphrase/i),
        ).toBeInTheDocument();
      },
      { timeout: 3000 },
    );

    // The stored values are byte-identical to the pre-seeded ones —
    // mintAndPersistKeypair did NOT fire.
    expect(generateKeypairSpy).not.toHaveBeenCalled();
    const identity = store.get(IDENTITY_KEY) as {
      pubkey_base58?: string;
      created_at?: number;
    };
    expect(identity.pubkey_base58).toBe(PRE_SEEDED_PUBKEY);
    expect(identity.created_at).toBe(1700000000000);
    expect(store.get(IDENTITY_SECRET_KEY)).toEqual(preSeededSecret);
  });

  it("surfaces a typed error when generate_keypair throws", async () => {
    installChromeStub();
    // Wasm stub whose generate_keypair throws synchronously — the
    // orchestrator must surface the error frame, NOT the
    // SetPassphrase step.
    __setWasmForTesting({
      default: () => Promise.resolve(undefined),
      generate_keypair: () => {
        throw new Error("WASM init failed in test");
      },
      sign_challenge: () => new Uint8Array(64),
      sign_cose_payload: () => new Uint8Array([0x84]),
      import_keypair_json: () => ({}),
      export_keypair_json: () => "{}",
      compress_embedding: () => new Uint8Array(0),
      decompress_embedding: () => new Float32Array(0),
      to_canonical_cbor_bytes: () => new Uint8Array(0),
      blake3_hash: () => new Uint8Array(32),
      blake3_hash_hex: () => "00".repeat(32),
    } as unknown as Parameters<typeof __setWasmForTesting>[0]);

    const signIn = vi
      .fn<[], Promise<GoogleSignInResult>>()
      .mockResolvedValue(makeSignInResult());
    const lookupExisting = vi
      .fn<[string], Promise<LookupResult>>()
      .mockResolvedValue({
        existingPubkey: null,
        escrowPresent: false,
        linkChallenge: btoa("link-challenge-XYZ"),
      });
    setRuntime(makeRuntime({ signIn, lookupExisting }));

    render(<Onboarding onComplete={() => undefined} />);
    fireEvent.click(
      screen.getByRole("button", { name: /Sign in with Google/i }),
    );

    expect(
      await screen.findByText(
        /keypair generation failed:.*WASM init failed in test/i,
      ),
    ).toBeInTheDocument();
  });
});

// Use the Session type so the import isn't pruned when this file is
// minified — keeps the type-only import explicit for reviewers.
type _UnusedSession = Session;

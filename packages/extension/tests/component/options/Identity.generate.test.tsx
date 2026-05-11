// T25 — Identity.generate flow.
//
// Covers the three TDD anchors for the Generate / Re-generate button:
//
//   1. fresh_user_clicks_generate_persists_keypair — empty storage,
//      one click, the runtime's `generate()` seam is called exactly
//      once, the displayed identity badge updates, and the success
//      toast surfaces.
//
//   2. existing_identity_regenerate_confirms_then_replaces — an
//      identity is already loaded. The user clicks Re-generate; the
//      confirm dialog fires. On accept we expect `clear()` then
//      `generate()` to run, the badge to update, and the success
//      toast to surface. On cancel we expect NEITHER to run and the
//      original identity to stay put.
//
//   3. generate_failure_surfaces_a_toast — the runtime throws; the
//      error message lands in a toast WITHOUT leaking the underlying
//      error stack (we just assert the toast text shape).
//
// We mock the IdentityFacade rather than the WASM `generate_keypair`
// directly: the component's contract with the runtime is the seam we
// regression-lock here. A separate test under `tests/unit/sign/` will
// cover the WASM path once `core/pkg-web` is checked in.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Identity } from "../../../src/options/sections/Identity.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
  type IdentitySnapshot,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1 } from "../../../src/settings.js";

function makeRuntime(overrides: {
  pubkey?: string | null;
  generated?: IdentitySnapshot;
  generate?: () => Promise<IdentitySnapshot>;
  clear?: () => Promise<void>;
}): OptionsRuntime {
  const generated: IdentitySnapshot = overrides.generated ?? {
    pubkey_base58: "FreshlyMintedPubKey",
    did: "did:sol:FreshlyMintedPubKey",
    created_at: 1715472000000,
  };
  return {
    settings: {
      load: async () => DEFAULT_SETTINGS_V1,
      update: async () => DEFAULT_SETTINGS_V1,
    },
    auth: {
      signIn: async () => ({ googleSub: "g", email: "u", jwt: "j" }),
      getSession: async () => null,
      clearSession: async () => undefined,
    },
    identity: {
      load: async () =>
        overrides.pubkey
          ? {
              pubkey_base58: overrides.pubkey,
              did: `did:sol:${overrides.pubkey}`,
            }
          : null,
      generate: overrides.generate ?? (async () => generated),
      clear: overrides.clear ?? (async () => undefined),
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      exportPlain: async () => ({
        version: 1,
        pubkey_base58: overrides.pubkey ?? "x",
        secret: Array.from({ length: 64 }, () => 0),
        exported_at: "2026-05-11T00:00:00.000Z",
        warning: "",
      }),
      importPlain: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
    },
    keyEscrow: {
      rotate: async () => undefined,
      delete: async () => undefined,
      hasBlob: async () => false,
    },
    cloudSync: {
      countLocalAttestations: async () => 0,
      countCloudAttestations: async () => 0,
      enqueueAll: async () => undefined,
      subscribeProgress: () => () => undefined,
      exportAll: async () => new Uint8Array(),
    },
    about: { version: "0.1.0" },
  };
}

describe("Identity section — T25 generate", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("fresh_user_clicks_generate_persists_keypair", async () => {
    const generate = vi.fn<[], Promise<IdentitySnapshot>>().mockResolvedValue({
      pubkey_base58: "NewBornPubKey0xCAFEBABE",
      did: "did:sol:NewBornPubKey0xCAFEBABE",
      created_at: Date.now(),
    });
    const clear = vi.fn<[], Promise<void>>().mockResolvedValue();
    setOptionsRuntime(makeRuntime({ pubkey: null, generate, clear }));

    render(<Identity />);
    // Empty-state copy renders first.
    await screen.findByText(/No agent identity found/i);

    fireEvent.click(
      screen.getByRole("button", { name: /^Generate identity$/i }),
    );

    await waitFor(() => expect(generate).toHaveBeenCalledTimes(1));
    // No existing identity → no clear() call.
    expect(clear).not.toHaveBeenCalled();
    // Badge updates to the freshly-minted pubkey.
    expect(
      await screen.findByText("did:sol:NewBornPubKey0xCAFEBABE"),
    ).toBeInTheDocument();
    // Success toast surfaces.
    expect(
      await screen.findByText(/New identity generated/i),
    ).toBeInTheDocument();
  });

  it("existing_identity_regenerate_confirms_then_replaces", async () => {
    const generate = vi.fn<[], Promise<IdentitySnapshot>>().mockResolvedValue({
      pubkey_base58: "ReplacementPubKey0xC0DE",
      did: "did:sol:ReplacementPubKey0xC0DE",
      created_at: Date.now(),
    });
    const clear = vi.fn<[], Promise<void>>().mockResolvedValue();
    const confirm = vi
      .fn<[string], boolean>()
      // Accept the regenerate confirmation.
      .mockReturnValue(true);
    setOptionsRuntime(
      makeRuntime({ pubkey: "OriginalPubKey0xDEAD", generate, clear }),
    );

    render(<Identity confirm={confirm} />);
    await screen.findByText("did:sol:OriginalPubKey0xDEAD");

    fireEvent.click(
      screen.getByRole("button", { name: /^Re-generate identity$/i }),
    );

    await waitFor(() => expect(generate).toHaveBeenCalledTimes(1));
    // The destructive-confirm message must reference the old pubkey
    // truncation so the user knows what they're about to lose. The
    // `shortPubkey` helper keeps `head=6` / `tail=4` chars — i.e.
    // "Origin…DEAD" for `OriginalPubKey0xDEAD` (20 chars).
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(confirm.mock.calls[0]![0]).toMatch(/Origin…DEAD/);
    // clear() runs BEFORE generate() so storage.set inside the
    // production runtime doesn't trip the "refusing to overwrite"
    // guard.
    expect(clear).toHaveBeenCalledTimes(1);
    expect(clear.mock.invocationCallOrder[0]!).toBeLessThan(
      generate.mock.invocationCallOrder[0]!,
    );
    expect(
      await screen.findByText("did:sol:ReplacementPubKey0xC0DE"),
    ).toBeInTheDocument();
  });

  it("regenerate_cancel_keeps_existing_identity", async () => {
    const generate = vi.fn<[], Promise<IdentitySnapshot>>().mockResolvedValue({
      pubkey_base58: "WontHappen",
      did: "did:sol:WontHappen",
    });
    const clear = vi.fn<[], Promise<void>>().mockResolvedValue();
    const confirm = vi.fn<[string], boolean>().mockReturnValue(false);
    setOptionsRuntime(
      makeRuntime({ pubkey: "PersistedPubKey0xFACE", generate, clear }),
    );

    render(<Identity confirm={confirm} />);
    await screen.findByText("did:sol:PersistedPubKey0xFACE");

    fireEvent.click(
      screen.getByRole("button", { name: /^Re-generate identity$/i }),
    );

    // Microtask drain so any in-flight onGenerate work resolves.
    await Promise.resolve();
    await Promise.resolve();
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(clear).not.toHaveBeenCalled();
    expect(generate).not.toHaveBeenCalled();
    // Identity badge unchanged.
    expect(
      screen.getByText("did:sol:PersistedPubKey0xFACE"),
    ).toBeInTheDocument();
  });

  it("generate_failure_surfaces_a_toast", async () => {
    const generate = vi
      .fn<[], Promise<IdentitySnapshot>>()
      .mockRejectedValue(new Error("WASM init failed"));
    setOptionsRuntime(makeRuntime({ pubkey: null, generate }));

    render(<Identity />);
    await screen.findByText(/No agent identity found/i);

    fireEvent.click(
      screen.getByRole("button", { name: /^Generate identity$/i }),
    );

    expect(
      await screen.findByText(/Generate failed:.*WASM init failed/i),
    ).toBeInTheDocument();
    // Empty-state stays put.
    expect(screen.getByText(/No agent identity found/i)).toBeInTheDocument();
  });
});

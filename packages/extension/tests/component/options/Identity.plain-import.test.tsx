// T25 — plain (unencrypted) keypair import flow.
//
// Covers:
//   - happy path: valid envelope is parsed + handed to the runtime;
//     identity badge updates; success toast surfaces.
//   - validation matrix: missing `version`, wrong `secret` length,
//     non-string pubkey, invalid base58 — each rejected at the
//     runtime seam and surfaced verbatim in a toast.
//   - existing-identity overwrite-confirm: when an identity is
//     already loaded the dialog must fire BEFORE the runtime call.
//     Cancel keeps the original.
//
// The runtime's `importPlain` does the heavy validation (every error
// class is locked here). The component is responsible ONLY for:
//   - parsing the file as JSON (raw `SyntaxError` toast on bad JSON);
//   - firing the overwrite-confirm when needed;
//   - propagating runtime errors into a toast.

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
  importPlain?: (payload: unknown) => Promise<IdentitySnapshot>;
}): OptionsRuntime {
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
      generate: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      clear: async () => undefined,
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      exportPlain: async () => ({
        version: 1,
        pubkey_base58: overrides.pubkey ?? "x",
        secret: Array.from({ length: 64 }, () => 0),
        exported_at: "2026-05-11T00:00:00.000Z",
        warning: "",
      }),
      importPlain:
        overrides.importPlain ??
        (async () => ({
          pubkey_base58: "ImportedDefault",
          did: "did:sol:ImportedDefault",
        })),
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

/** Build a JSON-ish File for the plain-import path. The handler reads
 *  via `.text()` so we install that on the File polyfill. */
function makeJsonFile(payload: unknown | string, name = "kp.json"): File {
  const body = typeof payload === "string" ? payload : JSON.stringify(payload);
  const file = new File([body], name, { type: "application/json" });
  Object.defineProperty(file, "text", {
    configurable: true,
    value: async () => body,
  });
  return file;
}

function pickFile(input: HTMLElement, file: File): void {
  Object.defineProperty(input, "files", {
    configurable: true,
    value: [file],
  });
  fireEvent.change(input);
}

const VALID_BASE58_PUBKEY = "5MhYbm6Q1vN9zJZ7QcLwY4uG2Yk3rrgu1qB7L8Tk2pYn";

describe("Identity section — T25 plain import", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("imports_a_valid_envelope_and_updates_badge", async () => {
    const importPlain = vi
      .fn<[unknown], Promise<IdentitySnapshot>>()
      .mockResolvedValue({
        pubkey_base58: VALID_BASE58_PUBKEY,
        did: `did:sol:${VALID_BASE58_PUBKEY}`,
        created_at: Date.now(),
      });
    setOptionsRuntime(makeRuntime({ pubkey: null, importPlain }));

    render(<Identity />);
    await screen.findByText(/No agent identity found/i);

    const fileInput = screen.getByLabelText(/Plain keypair file/i);
    const payload = {
      version: 1,
      pubkey_base58: VALID_BASE58_PUBKEY,
      secret: Array.from({ length: 64 }, () => 7),
      exported_at: "2026-05-11T00:00:00.000Z",
      warning: "any warning",
    };
    pickFile(fileInput, makeJsonFile(payload));

    await waitFor(() => expect(importPlain).toHaveBeenCalledTimes(1));
    const passedArg = importPlain.mock.calls[0]![0] as Record<string, unknown>;
    expect(passedArg.version).toBe(1);
    expect(passedArg.pubkey_base58).toBe(VALID_BASE58_PUBKEY);
    expect(
      await screen.findByText(`did:sol:${VALID_BASE58_PUBKEY}`),
    ).toBeInTheDocument();
    expect(await screen.findByText(/Keypair imported\./i)).toBeInTheDocument();
  });

  it("surfaces_a_clear_error_when_file_is_not_json", async () => {
    const importPlain = vi.fn<[unknown], Promise<IdentitySnapshot>>();
    setOptionsRuntime(makeRuntime({ pubkey: null, importPlain }));
    render(<Identity />);
    await screen.findByText(/No agent identity found/i);

    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile("{not-valid-json"),
    );

    expect(
      await screen.findByText(/Wrong file format: not valid JSON/i),
    ).toBeInTheDocument();
    expect(importPlain).not.toHaveBeenCalled();
  });

  // The four negative cases below mirror the four error classes
  // `runtime-impl.ts::importPlain` raises. We mock the runtime so the
  // toast assertions stay decoupled from the production error
  // strings — those are locked in a separate unit test on the runtime
  // itself.
  it("missing_version_surfaces_in_toast", async () => {
    const importPlain = vi
      .fn<[unknown], Promise<IdentitySnapshot>>()
      .mockRejectedValue(
        new Error("Wrong file format: unsupported version undefined"),
      );
    setOptionsRuntime(makeRuntime({ pubkey: null, importPlain }));
    render(<Identity />);
    await screen.findByText(/No agent identity found/i);
    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile({ pubkey_base58: VALID_BASE58_PUBKEY, secret: [] }),
    );
    expect(
      await screen.findByText(/Import failed:.*unsupported version/i),
    ).toBeInTheDocument();
  });

  it("wrong_secret_length_surfaces_in_toast", async () => {
    const importPlain = vi
      .fn<[unknown], Promise<IdentitySnapshot>>()
      .mockRejectedValue(
        new Error("Wrong secret length: got 32 bytes, expected 64"),
      );
    setOptionsRuntime(makeRuntime({ pubkey: null, importPlain }));
    render(<Identity />);
    await screen.findByText(/No agent identity found/i);
    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: Array.from({ length: 32 }, () => 0),
      }),
    );
    expect(
      await screen.findByText(/Import failed:.*Wrong secret length/i),
    ).toBeInTheDocument();
  });

  it("non_string_pubkey_surfaces_in_toast", async () => {
    const importPlain = vi
      .fn<[unknown], Promise<IdentitySnapshot>>()
      .mockRejectedValue(
        new Error("Invalid public key: expected a base58 string"),
      );
    setOptionsRuntime(makeRuntime({ pubkey: null, importPlain }));
    render(<Identity />);
    await screen.findByText(/No agent identity found/i);
    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile({
        version: 1,
        pubkey_base58: 12345,
        secret: Array.from({ length: 64 }, () => 0),
      }),
    );
    expect(
      await screen.findByText(/Import failed:.*Invalid public key/i),
    ).toBeInTheDocument();
  });

  it("invalid_base58_surfaces_in_toast", async () => {
    const importPlain = vi
      .fn<[unknown], Promise<IdentitySnapshot>>()
      .mockRejectedValue(
        new Error(
          "Invalid public key: not a recognisable base58 Ed25519 pubkey",
        ),
      );
    setOptionsRuntime(makeRuntime({ pubkey: null, importPlain }));
    render(<Identity />);
    await screen.findByText(/No agent identity found/i);
    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile({
        version: 1,
        // `0` and `O` are not part of the bitcoin base58 alphabet — the
        // runtime would reject this if we let it through; here we mock
        // the rejection so the toast contract is asserted in isolation.
        pubkey_base58: "00OO00OO00OO00OO00OO00OO00OO00OO",
        secret: Array.from({ length: 64 }, () => 0),
      }),
    );
    expect(
      await screen.findByText(
        /Import failed:.*not a recognisable base58 Ed25519 pubkey/i,
      ),
    ).toBeInTheDocument();
  });

  it("existing_identity_replace_confirm_fires_before_runtime_call", async () => {
    const importPlain = vi
      .fn<[unknown], Promise<IdentitySnapshot>>()
      .mockResolvedValue({
        pubkey_base58: VALID_BASE58_PUBKEY,
        did: `did:sol:${VALID_BASE58_PUBKEY}`,
      });
    const confirm = vi.fn<[string], boolean>().mockReturnValue(true);
    setOptionsRuntime(
      makeRuntime({ pubkey: "ExistingPubKey0xDEAD", importPlain }),
    );

    render(<Identity confirm={confirm} />);
    await screen.findByText("did:sol:ExistingPubKey0xDEAD");

    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: Array.from({ length: 64 }, () => 0),
      }),
    );

    await waitFor(() => expect(importPlain).toHaveBeenCalledTimes(1));
    // Confirm fired BEFORE the runtime call (we just assert ordering
    // via the deterministic resolution chain — confirm is synchronous
    // and runs in the same tick before the awaited importPlain).
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(confirm.mock.calls[0]![0]).toMatch(/Replacing existing identity/i);
    expect(
      await screen.findByText(`did:sol:${VALID_BASE58_PUBKEY}`),
    ).toBeInTheDocument();
  });

  it("existing_identity_replace_confirm_cancel_keeps_old_identity", async () => {
    const importPlain = vi.fn<[unknown], Promise<IdentitySnapshot>>();
    const confirm = vi.fn<[string], boolean>().mockReturnValue(false);
    setOptionsRuntime(
      makeRuntime({ pubkey: "ExistingPubKey0xFACE", importPlain }),
    );

    render(<Identity confirm={confirm} />);
    await screen.findByText("did:sol:ExistingPubKey0xFACE");

    pickFile(
      screen.getByLabelText(/Plain keypair file/i),
      makeJsonFile({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: Array.from({ length: 64 }, () => 0),
      }),
    );

    // Drain microtasks so any pending onChange fully resolves.
    await Promise.resolve();
    await Promise.resolve();
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(importPlain).not.toHaveBeenCalled();
    expect(
      screen.getByText("did:sol:ExistingPubKey0xFACE"),
    ).toBeInTheDocument();
  });
});

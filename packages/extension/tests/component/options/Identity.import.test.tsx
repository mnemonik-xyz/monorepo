// Identity section — TA-MIN4 coverage. Exercises the encrypted-keypair
// import flow: user types the passphrase, picks a file, the section
// hands the bytes + passphrase to `identity.importEncrypted`.
//
// The flow is symmetric to the export pipeline (`Identity.test.tsx`
// covers export). On success the displayed identity badge updates and
// the passphrase input clears; on failure (wrong passphrase, malformed
// blob) a toast surfaces the error message and the previously-loaded
// identity stays put.
//
// We deliberately mock `getOptionsRuntime().identity.importEncrypted`
// rather than the file-picker DOM API: the file picker is a native UI
// surface, but the change event we dispatch carries the exact `File`
// shape the production handler reads (`file.arrayBuffer()`).

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Identity } from "../../../src/options/sections/Identity.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
  type IdentitySnapshot,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1 } from "../../../src/settings.js";

function makeRuntime(
  overrides: {
    pubkey?: string | null;
    importEncrypted?: OptionsRuntime["identity"]["importEncrypted"];
  } = {},
): OptionsRuntime {
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
      generate: async () => ({
        pubkey_base58: "GeneratedPubKey",
        did: "did:sol:GeneratedPubKey",
      }),
      clear: async () => undefined,
      exportEncrypted: async () => new Uint8Array([1, 2, 3, 4]),
      importEncrypted:
        overrides.importEncrypted ??
        (async () => ({
          pubkey_base58: "imported-default",
          did: "did:sol:imported-default",
        })),
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

/** Build a File the production handler can call `.arrayBuffer()` on.
 *  We pass `Uint8Array` as the underlying chunk so the bytes round-trip
 *  to the importer exactly.
 *
 *  jsdom's `File` polyfill (as of v24.x) does NOT implement
 *  `arrayBuffer()` — we install a per-instance method that returns the
 *  original bytes. This matches the browser contract well enough for
 *  the handler under test. */
function makeFile(bytes: Uint8Array, name = "keypair.enc.json"): File {
  const file = new File([bytes], name, { type: "application/json" });
  // Copy the bytes so a later in-place mutation in the test body
  // doesn't poison the buffer the handler reads.
  const copy = bytes.slice();
  Object.defineProperty(file, "arrayBuffer", {
    configurable: true,
    value: async () =>
      copy.buffer.slice(copy.byteOffset, copy.byteOffset + copy.byteLength),
  });
  return file;
}

/** Simulate the native file-picker: dispatch a change event whose
 *  `target.files` carries our test File. React 19 + RTL's `fireEvent`
 *  honours the `{target: {files: ...}}` override on `<input
 *  type="file">`. */
function pickFile(input: HTMLElement, file: File): void {
  Object.defineProperty(input, "files", {
    configurable: true,
    value: [file],
  });
  fireEvent.change(input);
}

describe("Identity section — import flow (TA-MIN4)", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("imports an encrypted blob and updates the displayed identity", async () => {
    const restored: IdentitySnapshot = {
      pubkey_base58: "NewPubKeyAfterImport",
      did: "did:sol:NewPubKeyAfterImport",
    };
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockResolvedValue(restored);
    setOptionsRuntime(
      makeRuntime({
        pubkey: "OldPubKeyOnDevice",
        importEncrypted,
      }),
    );

    render(<Identity />);
    expect(
      await screen.findByText("did:sol:OldPubKeyOnDevice"),
    ).toBeInTheDocument();

    const passInput = screen.getByLabelText(/Import passphrase/i);
    fireEvent.change(passInput, { target: { value: "correct-passphrase-12" } });

    const blobBytes = new Uint8Array([0xde, 0xad, 0xbe, 0xef]);
    const fileInput = screen.getByLabelText(/Encrypted keypair file/i);
    pickFile(fileInput, makeFile(blobBytes));

    await waitFor(() => expect(importEncrypted).toHaveBeenCalledTimes(1));
    const [gotBytes, gotPp] = importEncrypted.mock.calls[0]!;
    expect(Array.from(gotBytes)).toEqual(Array.from(blobBytes));
    expect(gotPp).toBe("correct-passphrase-12");

    // Badge now shows the newly-imported identity (the success toast
    // also surfaces, but the badge is the load-bearing UX signal).
    expect(
      await screen.findByText("did:sol:NewPubKeyAfterImport"),
    ).toBeInTheDocument();
    // Passphrase field clears on success so the next import attempt is
    // a clean slate.
    expect((passInput as HTMLInputElement).value).toBe("");
  });

  it("rejects an empty passphrase BEFORE calling the runtime", async () => {
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockResolvedValue({
        pubkey_base58: "WontBeReached",
        did: "did:sol:WontBeReached",
      });
    setOptionsRuntime(
      makeRuntime({
        pubkey: "OriginalPub",
        importEncrypted,
      }),
    );
    render(<Identity />);
    await screen.findByText("did:sol:OriginalPub");

    // Skip the passphrase field, pick a file directly.
    const fileInput = screen.getByLabelText(/Encrypted keypair file/i);
    pickFile(fileInput, makeFile(new Uint8Array([1, 2, 3])));

    // Toast surfaces the validation error; importEncrypted is NEVER
    // invoked — this is the rate-limit-budget guard at the UI seam.
    expect(
      await screen.findByText(/Import passphrase is required\./i),
    ).toBeInTheDocument();
    expect(importEncrypted).not.toHaveBeenCalled();
    // Identity badge unchanged.
    expect(screen.getByText("did:sol:OriginalPub")).toBeInTheDocument();
  });

  // The four tests below are the regression-lock matrix for the
  // documented failure classes the runtime can throw (wrong passphrase,
  // malformed JSON, missing field, wrong byte length). They look
  // similar because the component handles every rejection the same
  // way — `Import failed: <err.message>` toast + badge unchanged — but
  // we keep them as four distinct cases so a future change that
  // accidentally swallows one class still surfaces here.
  // (Round-1 review finding 1.)
  it("surfaces a wrong-passphrase error from the runtime in a toast", async () => {
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockRejectedValue(new Error("wrong passphrase or tampered ciphertext"));
    setOptionsRuntime(
      makeRuntime({
        pubkey: "PrePub",
        importEncrypted,
      }),
    );
    render(<Identity />);
    await screen.findByText("did:sol:PrePub");

    fireEvent.change(screen.getByLabelText(/Import passphrase/i), {
      target: { value: "wrong-pp" },
    });
    pickFile(
      screen.getByLabelText(/Encrypted keypair file/i),
      makeFile(new Uint8Array([9, 9, 9])),
    );

    expect(
      await screen.findByText(
        /Import failed:.*wrong passphrase or tampered ciphertext/i,
      ),
    ).toBeInTheDocument();
    // Identity badge unchanged — the wrong-passphrase failure must NOT
    // wipe the currently-loaded keypair.
    expect(screen.getByText("did:sol:PrePub")).toBeInTheDocument();
  });

  it("surfaces a malformed-blob error from the runtime (bad JSON)", async () => {
    // The production importer parses the JSON envelope before unwrap;
    // a `SyntaxError`-shaped failure should appear in the toast as-is.
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockRejectedValue(new SyntaxError("Unexpected token in JSON at pos 0"));
    setOptionsRuntime(
      makeRuntime({
        pubkey: "PrePub",
        importEncrypted,
      }),
    );
    render(<Identity />);
    await screen.findByText("did:sol:PrePub");

    fireEvent.change(screen.getByLabelText(/Import passphrase/i), {
      target: { value: "pp-12-chars-or-more" },
    });
    pickFile(
      screen.getByLabelText(/Encrypted keypair file/i),
      makeFile(new Uint8Array([0x7b, 0x62, 0x61, 0x64])), // "{bad"
    );

    expect(
      await screen.findByText(/Import failed:.*Unexpected token/i),
    ).toBeInTheDocument();
    expect(screen.getByText("did:sol:PrePub")).toBeInTheDocument();
  });

  it("surfaces a missing-field error from the runtime", async () => {
    // The production importer rejects blobs that decrypt to a JSON
    // payload missing required fields (e.g. no `secret_b64`). The error
    // message is opaque to the UI; we only assert it surfaces verbatim.
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockRejectedValue(new Error("keypair payload is missing 'secret_b64'"));
    setOptionsRuntime(
      makeRuntime({
        pubkey: "PrePub",
        importEncrypted,
      }),
    );
    render(<Identity />);
    await screen.findByText("did:sol:PrePub");

    fireEvent.change(screen.getByLabelText(/Import passphrase/i), {
      target: { value: "pp-12-chars-or-more" },
    });
    pickFile(
      screen.getByLabelText(/Encrypted keypair file/i),
      makeFile(new Uint8Array([1, 2, 3])),
    );

    expect(
      await screen.findByText(/Import failed:.*missing 'secret_b64'/i),
    ).toBeInTheDocument();
  });

  it("surfaces a wrong-byte-length error from the runtime", async () => {
    // Ed25519 secrets must be 32 or 64 bytes. A 30-byte payload is
    // accepted at the file boundary but rejected at the keypair-shape
    // boundary — the runtime returns a typed error which we surface.
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockRejectedValue(
        new Error("invalid Ed25519 secret length: got 30, want 32 or 64"),
      );
    setOptionsRuntime(
      makeRuntime({
        pubkey: "PrePub",
        importEncrypted,
      }),
    );
    render(<Identity />);
    await screen.findByText("did:sol:PrePub");

    fireEvent.change(screen.getByLabelText(/Import passphrase/i), {
      target: { value: "pp-12-chars-or-more" },
    });
    pickFile(
      screen.getByLabelText(/Encrypted keypair file/i),
      makeFile(new Uint8Array(30)),
    );

    expect(
      await screen.findByText(
        /Import failed:.*invalid Ed25519 secret length.*30.*32 or 64/i,
      ),
    ).toBeInTheDocument();
  });

  it("export → triggers a download with the encrypted blob bytes", async () => {
    // The export path is exercised in `Identity.test.tsx` at the
    // runtime-call level (asserts `exportEncrypted(passphrase)`); here
    // we close the loop by asserting the *download surface* — i.e. a
    // Blob URL is created. Together they bind the canonical
    // "Export → download" UX without overlapping coverage.
    const encrypted = new Uint8Array([0x01, 0x02, 0x03, 0x04, 0x05]);
    const runtime = makeRuntime({ pubkey: "ExportPub" });
    runtime.identity.exportEncrypted = async () => encrypted;
    setOptionsRuntime(runtime);

    const createObjectURL = vi
      .fn<[Blob], string>()
      .mockReturnValue("blob:fake-url");
    const revokeObjectURL = vi.fn<[string], void>();
    const originalCreate = URL.createObjectURL;
    const originalRevoke = URL.revokeObjectURL;
    URL.createObjectURL =
      createObjectURL as unknown as typeof URL.createObjectURL;
    URL.revokeObjectURL =
      revokeObjectURL as unknown as typeof URL.revokeObjectURL;
    try {
      render(<Identity />);
      await screen.findByText("did:sol:ExportPub");

      const passInput = await screen.findByLabelText(/Export passphrase/i);
      // zxcvbn-friendly long string — see Identity.test.tsx for the
      // same passphrase shape.
      fireEvent.change(passInput, {
        target: { value: "Tr0ub4dor&3-Mnemonik-lazy-bear" },
      });
      fireEvent.click(
        screen.getByRole("button", { name: /^export keypair$/i }),
      );

      await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1));
      const blobArg = createObjectURL.mock.calls[0]![0];
      expect(blobArg).toBeInstanceOf(Blob);
      expect(blobArg.type).toBe("application/json");
      // The download helper revokes the URL after click — proves the
      // bookkeeping path runs (rather than leaving a dangling Blob URL).
      await waitFor(() =>
        expect(revokeObjectURL).toHaveBeenCalledWith("blob:fake-url"),
      );
    } finally {
      URL.createObjectURL = originalCreate;
      URL.revokeObjectURL = originalRevoke;
    }
  });

  it("does not crash when the file input fires a change with no file", async () => {
    // Defensive: a user can clear a file picker via the OS dialog
    // ("Cancel" on macOS); the event fires with an empty FileList. The
    // section's onChange short-circuits when `e.target.files?.[0]` is
    // undefined — no toast, no importer call.
    const importEncrypted = vi
      .fn<[Uint8Array, string], Promise<IdentitySnapshot>>()
      .mockResolvedValue({
        pubkey_base58: "WontHappen",
        did: "did:sol:WontHappen",
      });
    setOptionsRuntime(
      makeRuntime({
        pubkey: "Stable",
        importEncrypted,
      }),
    );
    render(<Identity />);
    await screen.findByText("did:sol:Stable");

    const fileInput = screen.getByLabelText(/Encrypted keypair file/i);
    Object.defineProperty(fileInput, "files", {
      configurable: true,
      value: [],
    });
    fireEvent.change(fileInput);

    // Give the microtask queue a tick; the importer must remain idle.
    await Promise.resolve();
    expect(importEncrypted).not.toHaveBeenCalled();
    expect(screen.getByText("did:sol:Stable")).toBeInTheDocument();
  });
});

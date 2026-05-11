// T25 — plain (unencrypted) keypair export flow.
//
// The Export button:
//
//   1. asks the runtime for a `PlainKeypairExport` envelope (the
//      runtime is the single seam that reads chrome.storage and
//      stamps the warning + ISO timestamp);
//   2. wraps the envelope into a Blob (`application/json`);
//   3. triggers a download via `URL.createObjectURL` + a synthesized
//      `<a download="mnemonik-keypair-<short>.json">`;
//   4. surfaces a non-removable warning toast.
//
// We mock `URL.createObjectURL` + `revokeObjectURL` so the test
// observes the Blob bytes without driving a real download. We also
// assert the JSON shape (`version === 1`, `secret.length === 64`,
// `warning` non-empty) so a future commit that drops the warning by
// accident trips this regression.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Identity } from "../../../src/options/sections/Identity.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
  type IdentitySnapshot,
  type PlainKeypairExport,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1 } from "../../../src/settings.js";

// Spy on the shared download helper so we capture the raw bytes the
// component asks the browser to save. jsdom's `Blob` does NOT
// implement `text()` / `arrayBuffer()` so reading the bytes back out
// of the Blob is unreliable — capturing them at the seam is both
// simpler and a tighter contract (we lock the producer side directly).
import * as downloadModule from "../../../src/options/utils/download.js";

function makeRuntime(overrides: {
  pubkey?: string | null;
  exportPlain?: () => Promise<PlainKeypairExport>;
}): OptionsRuntime {
  const snapshot: IdentitySnapshot | null = overrides.pubkey
    ? {
        pubkey_base58: overrides.pubkey,
        did: `did:sol:${overrides.pubkey}`,
        created_at: 1715472000000,
      }
    : null;
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
      load: async () => snapshot,
      generate: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      clear: async () => undefined,
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      exportPlain:
        overrides.exportPlain ??
        (async () => ({
          version: 1,
          pubkey_base58: overrides.pubkey ?? "x",
          secret: Array.from({ length: 64 }, (_, i) => i),
          exported_at: "2026-05-11T12:34:56.000Z",
          warning:
            "Never share this file — anyone with it controls your identity. Store it in a password manager.",
        })),
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

describe("Identity section — T25 plain export", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("export_button_disabled_when_no_identity", async () => {
    setOptionsRuntime(makeRuntime({ pubkey: null }));
    render(<Identity />);
    await screen.findByText(/No agent identity found/i);
    const btn = screen.getByRole("button", { name: /^Export$/ });
    expect(btn).toBeDisabled();
    expect(btn.getAttribute("title")).toMatch(/No identity to export/i);
  });

  it("export_triggers_download_with_warning_and_correct_filename", async () => {
    // Spy on the shared download helper. The component passes the
    // serialized JSON bytes + filename + mime; we read them straight
    // from the spy call so we don't need to round-trip through jsdom's
    // (no-op `.text()`) Blob.
    const triggerSpy = vi
      .spyOn(downloadModule, "triggerDownload")
      .mockImplementation(() => undefined);
    try {
      // 12-char base58 pubkey so the truncation rule fires (head=6,
      // tail=4 → "PubKey…0xAB").
      setOptionsRuntime(makeRuntime({ pubkey: "PubKeyAA0xAB" }));
      render(<Identity />);
      await screen.findByText("did:sol:PubKeyAA0xAB");

      fireEvent.click(screen.getByRole("button", { name: /^Export$/ }));
      await waitFor(() => expect(triggerSpy).toHaveBeenCalledTimes(1));

      const [bytes, filename, mime] = triggerSpy.mock.calls[0]!;
      // Cross-realm Uint8Array equality is unreliable under jsdom (the
      // component lives in the test realm but the constructor identity
      // can differ across the testing-library boundary) — assert the
      // duck-typed shape instead.
      expect(bytes).toBeDefined();
      expect((bytes as Uint8Array).byteLength).toBeGreaterThan(0);
      expect(mime).toBe("application/json");
      const text = new TextDecoder().decode(bytes as Uint8Array);
      const parsed = JSON.parse(text) as PlainKeypairExport;
      expect(parsed.version).toBe(1);
      expect(parsed.pubkey_base58).toBe("PubKeyAA0xAB");
      expect(parsed.secret).toHaveLength(64);
      expect(parsed.exported_at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
      // Security review: the warning string is mandatory. Audit the
      // exact text shape so a "shortened for brevity" edit trips the
      // regression.
      expect(parsed.warning).toMatch(/Never share this file/i);
      expect(parsed.warning).toMatch(/controls your identity/i);
      // The download filename includes a TRUNCATED pubkey (head=6,
      // tail=4) so the file label leaks the minimum identifier.
      expect(filename).toBe("mnemonik-keypair-PubKey…0xAB.json");
      // Success toast surfaces.
      expect(
        await screen.findByText(/Keypair downloaded\./i),
      ).toBeInTheDocument();
    } finally {
      triggerSpy.mockRestore();
    }
  });

  it("export_surface_renders_persistent_warning_block", async () => {
    setOptionsRuntime(makeRuntime({ pubkey: "PubKeyAA0xAB" }));
    render(<Identity />);
    await screen.findByText("did:sol:PubKeyAA0xAB");
    // Audit-mandated non-removable warning: ensure the dedicated
    // warning section renders even before the user has clicked Export.
    expect(
      screen.getByText(/Plain export warning/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/anyone with it controls your identity/i),
    ).toBeInTheDocument();
  });
});

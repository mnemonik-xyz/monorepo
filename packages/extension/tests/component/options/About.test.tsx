// About section — TA-MIN2 coverage. The component is a read-only
// metadata panel that reads version/buildHash from the runtime facade
// (which production wires to `chrome.runtime.getManifest`) and renders
// the canonical out-bound links (privacy policy + source repo).
//
// Note on scope (T24 coverage-gap doc): the round-2 task brief sketched
// a richer About panel (MCP endpoint, Discord, "Reset all data" button).
// That UX has not landed — the current implementation is intentionally
// minimal. These tests bind the *actual* surface; the richer surface is
// tracked in `decisions.md` (T24 coverage gaps).

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { About } from "../../../src/options/sections/About.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1 } from "../../../src/settings.js";

function makeRuntime(
  about: OptionsRuntime["about"] = { version: "0.1.0" },
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
      load: async () => null,
      generate: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      clear: async () => undefined,
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      exportPlain: async () => ({
        version: 1,
        pubkey_base58: "x",
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
    about,
  };
}

describe("About section", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("renders the section heading", async () => {
    setOptionsRuntime(makeRuntime());
    render(<About />);
    expect(
      screen.getByRole("heading", { name: /^about$/i }),
    ).toBeInTheDocument();
  });

  it("renders the version from the runtime facade", async () => {
    setOptionsRuntime(makeRuntime({ version: "1.2.3" }));
    render(<About />);
    // `useEffect` hydrates the value after first paint; `findByText`
    // waits for it.
    expect(await screen.findByText("1.2.3")).toBeInTheDocument();
  });

  it("renders the build hash row only when a hash is provided", async () => {
    setOptionsRuntime(makeRuntime({ version: "1.2.3", buildHash: "abc1234" }));
    render(<About />);
    expect(await screen.findByText("abc1234")).toBeInTheDocument();
    // Both Version and Build labels are present in the build-hash variant.
    expect(screen.getByText(/^Version$/i)).toBeInTheDocument();
    expect(screen.getByText(/^Build$/i)).toBeInTheDocument();
  });

  it("omits the build hash row when buildHash is undefined", async () => {
    setOptionsRuntime(makeRuntime({ version: "0.1.0" }));
    render(<About />);
    // Wait for the version to land so the effect has run, then assert
    // the Build label is NOT in the DOM.
    await screen.findByText("0.1.0");
    expect(screen.queryByText(/^Build$/i)).not.toBeInTheDocument();
  });

  it("renders the privacy-policy link with safe rel attributes", async () => {
    setOptionsRuntime(makeRuntime());
    render(<About />);
    const privacy = await screen.findByRole("link", {
      name: /mnemonik\.xyz\/privacy/i,
    });
    expect(privacy).toHaveAttribute("href", "https://mnemonik.xyz/privacy");
    expect(privacy).toHaveAttribute("target", "_blank");
    // `noreferrer noopener` mitigates `window.opener` cross-origin
    // leakage from third-party tabs spawned via this anchor.
    const rel = privacy.getAttribute("rel") ?? "";
    expect(rel).toContain("noreferrer");
    expect(rel).toContain("noopener");
  });

  it("renders the source-repo link with safe rel attributes", async () => {
    setOptionsRuntime(makeRuntime());
    render(<About />);
    const repo = await screen.findByRole("link", {
      name: /github\.com\/mnemonik-xyz\/monorepo/i,
    });
    expect(repo).toHaveAttribute(
      "href",
      "https://github.com/mnemonik-xyz/monorepo",
    );
    expect(repo).toHaveAttribute("target", "_blank");
    const rel = repo.getAttribute("rel") ?? "";
    expect(rel).toContain("noreferrer");
    expect(rel).toContain("noopener");
  });

  it("re-renders when the runtime is swapped between mounts", async () => {
    setOptionsRuntime(makeRuntime({ version: "1.0.0" }));
    const first = render(<About />);
    await waitFor(() => expect(first.getByText("1.0.0")).toBeInTheDocument());
    first.unmount();

    setOptionsRuntime(makeRuntime({ version: "2.0.0", buildHash: "deadbeef" }));
    render(<About />);
    expect(await screen.findByText("2.0.0")).toBeInTheDocument();
    expect(screen.getByText("deadbeef")).toBeInTheDocument();
  });
});

import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import IdentityPanel from "./IdentityPanel";

// Mock the WASM loader so the component test never touches the real binary.
vi.mock("../lib/wasm", () => {
  const fakeKp = {
    secret: Array.from({ length: 64 }, (_, i) => i % 256),
    pubkey_base58: "FakePubKeyBase58Test1234567890ABCDEFG",
  };
  return {
    loadWasm: vi.fn(async () => ({
      default: vi.fn(async () => undefined),
      generate_keypair: vi.fn(() => fakeKp),
      sign_challenge: vi.fn(),
      sign_attestation_bundle: vi.fn(),
      export_keypair_json: vi.fn(() => JSON.stringify(fakeKp)),
      import_keypair_json: vi.fn((s: string) => JSON.parse(s)),
    })),
    __resetWasmForTests: vi.fn(),
  };
});

describe("IdentityPanel", () => {
  beforeEach(() => {
    localStorage.clear();
  });
  afterEach(() => {
    localStorage.clear();
  });

  it("renders_did_after_generate", async () => {
    const { fireEvent } = await import("@testing-library/react");
    render(<IdentityPanel />);

    // Pre-state: no identity → Generate button visible, DID block absent.
    expect(screen.queryByTestId("identity-did")).toBeNull();
    const generate = screen.getByTestId("identity-generate");
    fireEvent.click(generate);

    // After click, the WASM generate_keypair mock returns the canned keypair
    // and the panel should render the DID + base58 pubkey.
    await waitFor(() => {
      expect(screen.getByTestId("identity-did")).toBeInTheDocument();
    });

    const did = screen.getByTestId("identity-did");
    expect(did.textContent).toBe(
      "did:sol:FakePubKeyBase58Test1234567890ABCDEFG"
    );
    const pubkey = screen.getByTestId("identity-pubkey");
    expect(pubkey.textContent).toBe("FakePubKeyBase58Test1234567890ABCDEFG");

    // Identity persists to localStorage.
    const stored = localStorage.getItem("mnemonic.identity");
    expect(stored).not.toBeNull();
    expect(JSON.parse(stored!).pubkey_base58).toBe(
      "FakePubKeyBase58Test1234567890ABCDEFG"
    );
  });
});

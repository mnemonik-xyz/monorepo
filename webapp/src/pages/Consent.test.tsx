import { act, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Consent from "./Consent";

vi.mock("../lib/wasm", () => ({
  loadWasm: vi.fn(async () => ({
    default: vi.fn(async () => undefined),
    generate_keypair: vi.fn(),
    sign_challenge: vi.fn(() => new Uint8Array([1, 2, 3, 4])),
    sign_attestation_bundle: vi.fn(),
    export_keypair_json: vi.fn(),
    import_keypair_json: vi.fn(),
  })),
  __resetWasmForTests: vi.fn(),
}));

describe("Consent page", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem(
      "mnemonic.identity",
      JSON.stringify({
        secret: Array.from({ length: 64 }, (_, i) => i),
        pubkey_base58: "TunnelConsentPubkey111111111111111111111",
      }),
    );
  });

  afterEach(() => {
    localStorage.clear();
    vi.unstubAllGlobals();
  });

  it("posts approval to the MCP origin carried by the authorize redirect", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response("state already consumed in test", {
          status: 401,
          headers: { "Content-Type": "text/plain" },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(
      <MemoryRouter
        initialEntries={[
          "/oauth/consent?challenge=YWJj&state=csrf-dev-tunnel&mcp_base=https%3A%2F%2Fpenguin-catherine-literature-articles.trycloudflare.com",
        ]}
      >
        <Routes>
          <Route path="/oauth/consent" element={<Consent />} />
        </Routes>
      </MemoryRouter>,
    );

    await act(async () => {
      fireEvent.click(await screen.findByTestId("consent-approve"));
    });

    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });
    await screen.findByTestId("consent-error");

    const [url, init] = fetchMock.mock.calls[0] as unknown as [
      string,
      RequestInit,
    ];
    expect(url).toBe(
      "https://penguin-catherine-literature-articles.trycloudflare.com/oauth/authorize",
    );
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toMatchObject({
      state: "csrf-dev-tunnel",
      signer_pubkey: "TunnelConsentPubkey111111111111111111111",
    });
  });
});

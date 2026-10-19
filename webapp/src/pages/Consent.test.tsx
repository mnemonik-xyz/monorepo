import { act, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Consent, { buildRedirectUrl } from "./Consent";

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

function renderConsent(query: string) {
  return render(
    <MemoryRouter initialEntries={[`/oauth/consent?${query}`]}>
      <Routes>
        <Route path="/oauth/consent" element={<Consent />} />
      </Routes>
    </MemoryRouter>,
  );
}

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

  it("posts approval to the hosted MCP origin carried by the authorize redirect", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response("state already consumed in test", {
          status: 401,
          headers: { "Content-Type": "text/plain" },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    renderConsent(
      "challenge=YWJj&state=csrf-dev-tunnel&mcp_base=https%3A%2F%2Fmcp2.mnemonik.xyz",
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
    expect(url).toBe("https://mcp2.mnemonik.xyz/oauth/authorize");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toMatchObject({
      state: "csrf-dev-tunnel",
      signer_pubkey: "TunnelConsentPubkey111111111111111111111",
    });
  });

  it("refuses to sign for an mcp_base that is not a hosted operator", async () => {
    // The keypair in localStorage would sign attacker-chosen challenge
    // bytes and POST the signature to the named origin. Any https host is
    // not enough: only mnemonik.xyz operators and loopback are accepted.
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    renderConsent(
      "challenge=YWJj&state=s&mcp_base=https%3A%2F%2Fevil.example",
    );

    const err = await screen.findByTestId("consent-error");
    expect(err.textContent).toContain("not a hosted Mnemonic operator");
    expect(screen.queryByTestId("consent-approve")).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("does not navigate to a redirect_uri with a script scheme", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            code: "c0de",
            state: "s",
            redirect_uri: "javascript:alert(document.domain)//",
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    renderConsent("challenge=YWJj&state=s");

    await act(async () => {
      fireEvent.click(await screen.findByTestId("consent-approve"));
    });

    const err = await screen.findByTestId("consent-error");
    expect(err.textContent).toContain("scheme this page will not open");
    expect(screen.queryByTestId("consent-status")).toBeNull();
  });
});

describe("buildRedirectUrl", () => {
  it("appends code and state to allowed schemes", () => {
    expect(
      buildRedirectUrl("http://127.0.0.1:8765/callback?x=1", "c", "s"),
    ).toBe("http://127.0.0.1:8765/callback?x=1&code=c&state=s");
    expect(
      buildRedirectUrl("cursor://anysphere.cursor-deeplink/mcp/oauth", "c", "s"),
    ).toBe("cursor://anysphere.cursor-deeplink/mcp/oauth?code=c&state=s");
    expect(buildRedirectUrl("vscode:mcp/callback", "c", "s")).toBe(
      "vscode:mcp/callback?code=c&state=s",
    );
  });

  it("rejects script, data and malformed URIs", () => {
    expect(buildRedirectUrl("javascript:alert(1)//", "c", "s")).toBeNull();
    expect(buildRedirectUrl("data:text/html,hi", "c", "s")).toBeNull();
    expect(buildRedirectUrl("not a url", "c", "s")).toBeNull();
  });
});

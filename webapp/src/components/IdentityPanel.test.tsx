import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

/**
 * Identity-panel test suite.
 *
 *   1. Generate populates DID + pubkey + localStorage (legacy coverage).
 *   2. "Send to CLI" POSTs to `/api/cli-bootstrap/issue` with the JWT,
 *      renders the paste-command, and the Copy button writes the exact
 *      `mnemonic identity import --ticket <uuid>` string to the clipboard
 *      (drives Decision-7 frontend per `tasks/7.md` TDD anchor).
 *   3. 429 surfaces the per-user cap message inline.
 *   4. 401 redirects the user to `/oauth/consent` to re-auth.
 */
describe("IdentityPanel", () => {
  beforeEach(() => {
    localStorage.clear();
  });
  afterEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  it("renders_did_after_generate", async () => {
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

  it("send_to_cli_calls_endpoint_and_displays_ticket", async () => {
    // Seed an identity + JWT so the button is enabled and the click handler
    // can attach the Authorization header.
    localStorage.setItem(
      "mnemonic.identity",
      JSON.stringify({
        secret: Array.from({ length: 64 }, (_, i) => i % 256),
        pubkey_base58: "FakePubKeyBase58Test1234567890ABCDEFG",
      })
    );
    localStorage.setItem("mnemonic.jwt", "test-jwt-abc");

    const ticketId = "550e8400-e29b-41d4-a716-446655440000";
    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ ticket_id: ticketId }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
    );
    vi.stubGlobal("fetch", fetchMock);

    // navigator.clipboard is not present in jsdom by default — install a
    // spy-able stub that the Copy button can call.
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });

    render(<IdentityPanel />);

    const button = screen.getByTestId("identity-send-to-cli");
    expect(button).toBeInTheDocument();
    fireEvent.click(button);

    // The endpoint was called with Bearer header + the keypair JSON.
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [url, init] = fetchMock.mock.calls[0] as unknown as [
      string,
      RequestInit
    ];
    expect(url).toMatch(/\/api\/cli-bootstrap\/issue$/);
    expect(init.method).toBe("POST");
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer test-jwt-abc");
    expect(headers["Content-Type"]).toBe("application/json");
    const body = JSON.parse((init.body as string) ?? "{}");
    expect(typeof body.keypair_json).toBe("string");
    // The webapp posts the raw localStorage value verbatim — server stores
    // it without inspection (see api.rs::bootstrap_issue_handler).
    const parsedKp = JSON.parse(body.keypair_json);
    expect(parsedKp.pubkey_base58).toBe(
      "FakePubKeyBase58Test1234567890ABCDEFG"
    );

    // Ticket is rendered inside the code block as the paste-command.
    const command = await screen.findByTestId("identity-cli-command");
    expect(command.textContent).toBe(
      `mnemonic identity import --ticket ${ticketId}`
    );

    // Copy button writes the exact paste-command to the clipboard.
    fireEvent.click(screen.getByTestId("identity-cli-copy"));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(
        `mnemonic identity import --ticket ${ticketId}`
      );
    });
  });

  it("send_to_cli_429_shows_per_user_cap_message", async () => {
    localStorage.setItem(
      "mnemonic.identity",
      JSON.stringify({
        secret: Array.from({ length: 64 }, (_, i) => i % 256),
        pubkey_base58: "FakePubKeyBase58Test1234567890ABCDEFG",
      })
    );
    localStorage.setItem("mnemonic.jwt", "test-jwt-abc");

    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            error: "per-user bootstrap-ticket cap exceeded (3 active)",
          }),
          { status: 429, headers: { "Content-Type": "application/json" } }
        )
    );
    vi.stubGlobal("fetch", fetchMock);

    render(<IdentityPanel />);
    fireEvent.click(screen.getByTestId("identity-send-to-cli"));

    const errorMsg = await screen.findByTestId("identity-cli-error");
    expect(errorMsg.textContent).toContain("3 active CLI tickets");
    // Code block must NOT render on the error path.
    expect(screen.queryByTestId("identity-cli-command")).toBeNull();
  });

  it("send_to_cli_401_redirects_to_oauth_consent", async () => {
    localStorage.setItem(
      "mnemonic.identity",
      JSON.stringify({
        secret: Array.from({ length: 64 }, (_, i) => i % 256),
        pubkey_base58: "FakePubKeyBase58Test1234567890ABCDEFG",
      })
    );
    localStorage.setItem("mnemonic.jwt", "expired-jwt");

    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ error: "invalid token" }), {
          status: 401,
          headers: { "Content-Type": "application/json" },
        })
    );
    vi.stubGlobal("fetch", fetchMock);

    // jsdom's `window.location.assign` is read-only by default; replace it
    // with a spy so we can assert the redirect target.
    const assign = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, assign },
    });

    render(<IdentityPanel />);
    fireEvent.click(screen.getByTestId("identity-send-to-cli"));

    await waitFor(() => {
      expect(assign).toHaveBeenCalledWith("/oauth/consent");
    });
    // No ticket displayed on the redirect path.
    expect(screen.queryByTestId("identity-cli-command")).toBeNull();
  });
});

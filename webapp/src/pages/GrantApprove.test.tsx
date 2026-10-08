import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import GrantApprove from "./GrantApprove";
import { MCP_BASE } from "../lib/api";

vi.mock("../lib/wasm", () => ({
  loadWasm: vi.fn(async () => ({
    default: vi.fn(async () => undefined),
    unwrap_content_key: undefined, // not yet available
    wrap_content_key_to_recipient: undefined,
  })),
  __resetWasmForTests: vi.fn(),
}));

const VALID_HASH =
  "a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4";
const READER_DID = "did:sol:ReaderPubkeyAbc123";
const GRANT_TOKEN = "grant-token-xyz";

function renderGrantApprove(
  hash = VALID_HASH,
  reader = READER_DID,
  token = GRANT_TOKEN,
) {
  const url = `/grant/approve?memory_hash=${encodeURIComponent(hash)}&reader=${encodeURIComponent(reader)}&grant_token=${encodeURIComponent(token)}`;
  return render(
    <MemoryRouter initialEntries={[url]}>
      <GrantApprove />
    </MemoryRouter>,
  );
}

describe("GrantApprove page", () => {
  let originalFetch: typeof fetch;

  beforeEach(() => {
    localStorage.setItem(
      "mnemonic.identity",
      JSON.stringify({ secret: [1, 2, 3], pubkey_base58: "AuthorPubkey123" }),
    );
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    localStorage.clear();
  });

  it("renders_page_and_shows_memory_details", async () => {
    renderGrantApprove();
    expect(
      await screen.findByTestId("grant-approve-page"),
    ).toBeInTheDocument();
    expect(screen.getByText(VALID_HASH)).toBeInTheDocument();
    expect(screen.getByText(READER_DID)).toBeInTheDocument();
  });

  it("shows_grant_access_and_reject_buttons", async () => {
    renderGrantApprove();
    await screen.findByTestId("grant-proceed");
    expect(screen.getByTestId("grant-proceed")).toBeInTheDocument();
    expect(screen.getByTestId("grant-reject")).toBeInTheDocument();
  });

  it("reject_button_shows_rejected_state", async () => {
    const user = userEvent.setup();
    renderGrantApprove();
    await screen.findByTestId("grant-reject");
    await user.click(screen.getByTestId("grant-reject"));
    const error = await screen.findByTestId("grant-error");
    expect(error).toHaveTextContent(/rejected/i);
  });

  it("proceed_shows_no_revocation_warning", async () => {
    const user = userEvent.setup();
    renderGrantApprove();
    await screen.findByTestId("grant-proceed");
    await user.click(screen.getByTestId("grant-proceed"));
    expect(
      await screen.findByTestId("grant-warn"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/grants cannot be revoked/i),
    ).toBeInTheDocument();
    // Check warning text — may be split across elements, use container search.
    expect(
      screen.getByText(/once granted/i),
    ).toBeInTheDocument();
  });

  it("cancel_from_warn_returns_to_ready", async () => {
    const user = userEvent.setup();
    renderGrantApprove();
    await screen.findByTestId("grant-proceed");
    await user.click(screen.getByTestId("grant-proceed"));
    await screen.findByTestId("grant-warn");
    await user.click(screen.getByTestId("grant-cancel-warn"));
    expect(screen.queryByTestId("grant-warn")).not.toBeInTheDocument();
    expect(screen.getByTestId("grant-proceed")).toBeInTheDocument();
  });

  it("confirm_grant_shows_error_when_wasm_not_available", async () => {
    const user = userEvent.setup();
    // Mock fetch for the sealed bytes.
    globalThis.fetch = vi.fn(async () =>
      new Response(new Uint8Array([1, 2, 3]), {
        status: 200,
        headers: { "content-type": "application/cbor" },
      }),
    ) as unknown as typeof fetch;

    renderGrantApprove();
    await screen.findByTestId("grant-proceed");
    await user.click(screen.getByTestId("grant-proceed"));
    await screen.findByTestId("grant-warn");
    await user.click(screen.getByTestId("grant-confirm"));
    const error = await screen.findByTestId("grant-error", {}, { timeout: 3000 });
    expect(error).toBeInTheDocument();
    expect(error.textContent).toMatch(/newer version|not available/i);
  });

  it("shows_error_when_missing_memory_hash_param", () => {
    render(
      <MemoryRouter initialEntries={["/grant/approve?reader=did:sol:abc"]}>
        <GrantApprove />
      </MemoryRouter>,
    );
    // Should show an error about missing params.
    expect(
      screen.getByText(/Missing `memory_hash` or `reader`/i),
    ).toBeInTheDocument();
  });

  async function confirmWithMcpBase(mcpBase: string): Promise<string> {
    const fetchMock = vi.fn(async () =>
      new Response(new Uint8Array([1, 2, 3]), {
        status: 200,
        headers: { "content-type": "application/cbor" },
      }),
    );
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const user = userEvent.setup();
    const url =
      `/grant/approve?correlation_id=c1&memory_hash=${VALID_HASH}` +
      `&reader=${encodeURIComponent(READER_DID)}&owner=did%3Asol%3AO` +
      `&mcp_base=${encodeURIComponent(mcpBase)}`;
    render(
      <MemoryRouter initialEntries={[url]}>
        <GrantApprove />
      </MemoryRouter>,
    );
    await user.click(await screen.findByTestId("grant-proceed"));
    await user.click(await screen.findByTestId("grant-confirm"));
    await screen.findByTestId("grant-error", {}, { timeout: 3000 });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    return String((fetchMock.mock.calls[0] as unknown[])[0]);
  }

  it("uses_the_operator_named_by_mcp_base", async () => {
    const fetched = await confirmWithMcpBase("https://mcp2.mnemonik.xyz");
    expect(fetched).toBe(`https://mcp2.mnemonik.xyz/api/sealed/${VALID_HASH}`);
  });

  it("ignores_mcp_base_outside_hosted_operators", async () => {
    const fetched = await confirmWithMcpBase("https://evil.example");
    expect(fetched).toBe(`${MCP_BASE}/api/sealed/${VALID_HASH}`);
  });

  it("shows_no_identity_state_when_no_keypair", async () => {
    localStorage.clear(); // no identity
    renderGrantApprove();
    await screen.findByText(/No keypair found in this browser/i);
  });
});

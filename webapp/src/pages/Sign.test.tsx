import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type Mock,
} from "vitest";
import Sign from "./Sign";

vi.mock("../lib/wasm", () => ({
  loadWasm: vi.fn(async () => ({
    default: vi.fn(async () => undefined),
    generate_keypair: vi.fn(),
    sign_challenge: vi.fn(),
    sign_attestation_bundle: vi.fn(() => new Uint8Array([1, 2, 3])),
    sign_cose_payload: vi.fn(() => new Uint8Array([1, 2, 3])),
    export_keypair_json: vi.fn(),
    import_keypair_json: vi.fn(),
  })),
  __resetWasmForTests: vi.fn(),
}));

const TEST_UUID = "11111111-2222-4333-8444-555555555555";

function buildJwt(sub: string): string {
  // Unsigned JWT (browser-side decode only) — payload contains `sub`.
  const header = btoa(JSON.stringify({ alg: "HS256", typ: "JWT" }));
  const payload = btoa(JSON.stringify({ sub, iss: "mcp.mnemonik.xyz" }));
  return `${header}.${payload}.fakesig`;
}

describe("Sign page", () => {
  let originalFetch: typeof fetch;
  let fetchMock: Mock;
  // Wallclock the test code controls; the fetch mock + Date.now() both read it.
  let mockNow = Date.UTC(2026, 3, 26, 12, 0, 0); // 2026-04-26T12:00:00Z
  let originalDateNow: () => number;

  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("mnemonic.jwt", buildJwt("TestSubPubKey123"));
    // Sign's first effect redirects to /install when no identity is stored, which
    // unmounts the page before the countdown can render — the real cause of the
    // pre-existing flake. Seed a valid identity so the page actually mounts.
    localStorage.setItem(
      "mnemonic.identity",
      JSON.stringify({ secret: [1, 2, 3], pubkey_base58: "TestSubPubKey123" }),
    );

    mockNow = Date.UTC(2026, 3, 26, 12, 0, 0);
    originalDateNow = Date.now;
    Date.now = () => mockNow;

    originalFetch = globalThis.fetch;
    fetchMock = vi.fn(async () => {
      const expiresAtSec = Math.floor(mockNow / 1000) + 5 * 60; // exactly 5min from now
      const fakeCbor = new TextEncoder().encode(
        '{"content":"hello world test bundle preview"}',
      );
      return new Response(fakeCbor, {
        status: 200,
        headers: {
          "content-type": "application/cbor",
          "x-mnemonic-content-hash": "deadbeef",
          "x-mnemonic-expires-at": expiresAtSec.toString(),
        },
      });
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    Date.now = originalDateNow;
    localStorage.clear();
  });

  it("countdown_displays_mm_ss", async () => {
    // Real timers throughout: the earlier fake-setInterval + real-waitFor mix
    // raced the bundle-fetch microtasks against the poll loop and flaked. Using
    // RTL's findBy* (the canonical async query) to await the countdown, then a
    // waitFor on the next 1s tick, makes the result deterministic — the label
    // WILL reach 04:59 once the mocked wall-clock advances and the interval fires.
    render(
      <MemoryRouter initialEntries={[`/sign/${TEST_UUID}`]}>
        <Routes>
          <Route path="/sign/:correlationId" element={<Sign />} />
        </Routes>
      </MemoryRouter>,
    );

    // Appears once the bundle fetch resolves (microtask-driven); findBy* retries.
    const countdown = await screen.findByTestId(
      "sign-countdown",
      {},
      { timeout: 3000 },
    );
    // Initial: ~05:00 (allow 04:59 if a tick already fired between mount + read).
    expect(countdown.textContent).toMatch(/Expires in 0[45]:\d{2}/);

    // The countdown re-reads Date.now() on a 1s interval. Advance the mocked
    // wall-clock; the next tick must count the label down to 04:59.
    mockNow += 1000;
    await waitFor(
      () =>
        expect(screen.getByTestId("sign-countdown").textContent).toMatch(
          /Expires in 04:59/,
        ),
      { timeout: 3000, interval: 50 },
    );
  });

  it("offers a seamless session first and pay-once second after 402", async () => {
    let call = 0;
    fetchMock.mockImplementation(async () => {
      call += 1;
      if (call === 1) {
        return new Response(new Uint8Array([0xa0]), {
          status: 200,
          headers: {
            "content-type": "application/cbor",
            "x-mnemonic-content-hash": "deadbeef",
            "x-mnemonic-expires-at": String(Math.floor(mockNow / 1000) + 300),
          },
        });
      }
      return new Response(
        JSON.stringify({
          status: "payment_required",
          operation_id: TEST_UUID,
          artifact_hash: "deadbeef",
          binding_digest: "binding-digest",
          workspace: "workspace-alpha",
          binding: {
            scope: {
              visibility: "private",
              action: "pre_compaction",
              workspace_hash: "workspace-hash",
            },
          },
          quote: {
            amount: "1000",
            asset: "0xasset",
            network: "eip155:5042002",
            pay_to: "0xpayee",
            expires_at: "2026-07-13T12:00:00Z",
            accepts: [
              {
                scheme: "stake",
                payment_url: "https://pay.example/approve",
                recommended_cap: "5000000",
                max_per_anchor: "50000",
                valid_for_seconds: 604800,
              },
              { scheme: "exact", protocol: "x402" },
            ],
          },
        }),
        { status: 402, headers: { "content-type": "application/json" } },
      );
    });

    render(
      <MemoryRouter initialEntries={[`/sign/${TEST_UUID}`]}>
        <Routes>
          <Route path="/sign/:correlationId" element={<Sign />} />
        </Routes>
      </MemoryRouter>,
    );
    fireEvent.click(await screen.findByTestId("sign-approve"));
    await screen.findByTestId("sign-payment-required");
    const session = screen.getByTestId("start-paid-session") as HTMLAnchorElement;
    const once = screen.getByTestId("pay-once") as HTMLAnchorElement;
    expect(session.textContent).toContain("Start seamless anchoring");
    expect(session.href).toContain("scheme=stake");
    expect(session.href).toContain(`operation_id=${TEST_UUID}`);
    expect(session.href).not.toContain("binding_digest=");
    expect(once.href).toContain("scheme=exact");
    expect(screen.getByText("workspace-alpha")).toBeInTheDocument();
    expect(screen.getByText("Before context compression")).toBeInTheDocument();
    expect(screen.getByText(/never renew automatically/i)).toBeInTheDocument();
  });
});

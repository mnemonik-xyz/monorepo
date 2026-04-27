import { act, render, screen, waitFor } from "@testing-library/react";
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

    mockNow = Date.UTC(2026, 3, 26, 12, 0, 0);
    originalDateNow = Date.now;
    Date.now = () => mockNow;

    originalFetch = globalThis.fetch;
    fetchMock = vi.fn(async () => {
      const expiresAtSec = Math.floor(mockNow / 1000) + 5 * 60; // exactly 5min from now
      const fakeCbor = new TextEncoder().encode(
        '{"content":"hello world test bundle preview"}'
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
    // Fake only setInterval / clearInterval. Leave setTimeout real so
    // testing-library's waitFor (which polls via setTimeout) keeps working.
    // The component's countdown uses setInterval; that's all we need to drive.
    vi.useFakeTimers({
      toFake: ["setInterval", "clearInterval"],
    });

    render(
      <MemoryRouter initialEntries={[`/sign/${TEST_UUID}`]}>
        <Routes>
          <Route path="/sign/:correlationId" element={<Sign />} />
        </Routes>
      </MemoryRouter>
    );

    // Wait for the fetch to resolve and the countdown element to appear.
    // waitFor here uses real microtasks since Promise wasn't faked.
    await waitFor(
      () => {
        const el = screen.queryByTestId("sign-countdown");
        if (!el) throw new Error("countdown not yet rendered");
      },
      { timeout: 3000, interval: 25 }
    );

    const countdown = screen.getByTestId("sign-countdown");
    // Initial: ~05:00 (allow 04:59 if a tick already fired).
    expect(countdown.textContent).toMatch(/Expires in 0[45]:\d{2}/);

    // Advance wall-clock 1s and trigger the countdown's setInterval tick.
    mockNow += 1000;
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });

    const after = screen.getByTestId("sign-countdown").textContent ?? "";
    expect(after).toMatch(/Expires in \d{2}:\d{2}/);

    vi.useRealTimers();
  });
});

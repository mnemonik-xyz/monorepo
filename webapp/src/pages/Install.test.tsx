import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock the WASM loader used transitively by IdentityPanel — Install renders
// IdentityPanel, which calls loadWasm() in its effects. We never need real
// WASM for the redeem-wire-shape test.
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

// Pin tweetnacl: deterministic ephemeral keypair + fixed plaintext so the
// pubkey-verification step downstream matches the server-supplied
// `issuer_pubkey_base58`. The 64-byte "secret" we return from nacl.box.open
// has its last 32 bytes encoded as bs58 = TEST_PUBKEY below.
vi.mock("tweetnacl", () => {
  // 32 bytes of 0x01 → bs58 begins with a deterministic value. We compute the
  // expected pubkey from this seed and assert against it in the test.
  const seedPub = new Uint8Array(32).fill(0x01);
  const plaintext64 = new Uint8Array(64);
  // first 32 bytes can be anything (the Ed25519 seed); the last 32 must be
  // the pubkey portion that we'll bs58-encode for the verification check.
  plaintext64.set(seedPub, 32);

  const boxFn = {
    keyPair: vi.fn(() => ({
      publicKey: new Uint8Array(32).fill(0x02),
      secretKey: new Uint8Array(32).fill(0x03),
    })),
    open: vi.fn(() => plaintext64),
    nonceLength: 24,
  } as unknown as typeof import("tweetnacl").box;

  return {
    default: { box: boxFn },
  };
});

describe("Install — ?pull= redeem wire shape", () => {
  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
  });
  afterEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("reads issuer_pubkey_base58 from CliRedeemResponse and writes identity to localStorage", async () => {
    // Seed a JWT so the component does not redirect to /oauth/consent.
    localStorage.setItem("mnemonic.jwt", "test-jwt-abc");

    // Derive the expected pubkey base58 from the mocked plaintext (the last
    // 32 bytes are 0x01 * 32). Mirror the same encoding the component does.
    // We import bs58 lazily inside the test so the top-level mock for
    // tweetnacl is set up first.
    const { default: bs58 } = await import("bs58");
    const expectedPubkey = bs58.encode(new Uint8Array(32).fill(0x01));

    // base64-encoded nonce[24] || ciphertext blob. The nacl.box.open mock
    // ignores the input and returns the canned plaintext, so any non-empty
    // wrapped value with the right length passes the slice() check.
    const wrappedBytes = new Uint8Array(24 + 32);
    const wrappedB64 = btoa(String.fromCharCode(...wrappedBytes));
    const ephPubB64 = btoa(String.fromCharCode(...new Uint8Array(32)));

    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            wrapped_secret: wrappedB64,
            eph_pub: ephPubB64,
            origin: "Cli",
            issuer_pubkey_base58: expectedPubkey,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    // Render with ?pull=ABCD-1234 so the effect fires.
    const { default: Install } = await import("./Install");
    render(
      <MemoryRouter initialEntries={["/install?pull=ABCD-1234"]}>
        <Install />
      </MemoryRouter>,
    );

    // Wait until fetch was called.
    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    // Assert the request shape.
    const call = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(call).toBeDefined();
    const [url, init] = call;
    expect(url).toMatch(/\/api\/cli-bootstrap\/redeem$/);
    expect(init.method).toBe("POST");
    const reqBody = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(reqBody.short_code).toBe("ABCD-1234");
    expect(typeof reqBody.redeemer_eph_pub).toBe("string");

    // Assert localStorage was updated by writeIdentity with the issuer pubkey.
    await vi.waitFor(() => {
      const stored = localStorage.getItem("mnemonic.identity");
      expect(stored).not.toBeNull();
      const parsed = JSON.parse(stored as string);
      expect(parsed.pubkey_base58).toBe(expectedPubkey);
      expect(Array.isArray(parsed.secret)).toBe(true);
      expect(parsed.secret.length).toBe(64);
    });
  });

  it("rejects a redeem response that uses the old pubkey_base58 field name", async () => {
    localStorage.setItem("mnemonic.jwt", "test-jwt-abc");
    const { default: bs58 } = await import("bs58");
    const expectedPubkey = bs58.encode(new Uint8Array(32).fill(0x01));

    const wrappedBytes = new Uint8Array(24 + 32);
    const wrappedB64 = btoa(String.fromCharCode(...wrappedBytes));
    const ephPubB64 = btoa(String.fromCharCode(...new Uint8Array(32)));

    // Old-shape response (regression guard): emits pubkey_base58 instead of
    // issuer_pubkey_base58. The component MUST reject this as malformed.
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            wrapped_secret: wrappedB64,
            eph_pub: ephPubB64,
            origin: "Cli",
            pubkey_base58: expectedPubkey,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const { default: Install } = await import("./Install");
    render(
      <MemoryRouter initialEntries={["/install?pull=ZZZZ-9999"]}>
        <Install />
      </MemoryRouter>,
    );

    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    // No identity written for malformed-shape response.
    await vi.waitFor(() => {
      expect(localStorage.getItem("mnemonic.identity")).toBeNull();
    });
  });
});

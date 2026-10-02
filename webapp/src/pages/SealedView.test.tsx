import { render, screen, waitFor } from "@testing-library/react";
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
import SealedView, {
  __test__extractKFromFragment,
  __test__parseSealedArtifact,
} from "./SealedView";
import { Encoder } from "cbor-x";
import { loadWasm } from "../lib/wasm";

vi.mock("../lib/wasm", () => ({
  loadWasm: vi.fn(async () => ({
    default: vi.fn(async () => undefined),
    blake3_hash: vi.fn((_input: Uint8Array) => new Uint8Array(32)),
    open_with_key: undefined, // not yet available
  })),
  __resetWasmForTests: vi.fn(),
}));

const encoder = new Encoder();

const VALID_HASH =
  "a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4";

function buildSealedCbor(overrides: Record<string, unknown> = {}): Uint8Array {
  const artifact = {
    artifact_id: "art:sealed-test-001",
    type: "sealed",
    schema_version: 1,
    alg: "xchacha20poly1305",
    nonce: new Uint8Array(24),
    ct: new Uint8Array(64),
    kc: new Uint8Array(32),
    wraps: [],
    created_at: "2026-09-28T10:00:00Z",
    producer: "did:sol:TestProducerPubkey1234",
    ...overrides,
  };
  return encoder.encode(artifact);
}

function renderSealedView(hash: string, fragmentHash = "") {
  // Mock window.location.hash before render.
  Object.defineProperty(window, "location", {
    writable: true,
    value: {
      ...window.location,
      hash: fragmentHash,
      href: `http://localhost/m/${hash}${fragmentHash}`,
    },
  });

  return render(
    <MemoryRouter initialEntries={[`/m/${hash}`]}>
      <Routes>
        <Route path="/m/:hash" element={<SealedView />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("SealedView — extractKFromFragment", () => {
  it("returns null for empty fragment", () => {
    expect(__test__extractKFromFragment("")).toBeNull();
    expect(__test__extractKFromFragment("#")).toBeNull();
    expect(__test__extractKFromFragment("#other=value")).toBeNull();
  });

  it("returns the key for a valid #k= fragment", () => {
    // 43-char base64url = 32 bytes
    const fakeKey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    expect(__test__extractKFromFragment(`#k=${fakeKey}`)).toBe(fakeKey);
  });

  it("returns null for wrong length key", () => {
    // 44 chars (padded base64) — should be rejected
    const tooLong = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    expect(__test__extractKFromFragment(`#k=${tooLong}`)).toBeNull();
    // 42 chars — too short
    const tooShort = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".slice(0, 42);
    expect(__test__extractKFromFragment(`#k=${tooShort}`)).toBeNull();
  });

  it("returns null when key contains invalid base64url chars", () => {
    // Standard base64 uses + and / which are NOT base64url
    const invalidChars = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".slice(0, 42) + "+";
    expect(__test__extractKFromFragment(`#k=${invalidChars}`)).toBeNull();
  });
});

describe("SealedView — parseSealedArtifact", () => {
  it("parses a valid SEALED_V1 CBOR artifact", () => {
    const cbor = buildSealedCbor();
    const artifact = __test__parseSealedArtifact(cbor);
    expect(artifact.type).toBe("sealed");
    expect(artifact.producer).toBe("did:sol:TestProducerPubkey1234");
    expect(artifact.createdAt).toBe("2026-09-28T10:00:00Z");
    expect(artifact.kc).toBeInstanceOf(Uint8Array);
    expect(artifact.kc?.length).toBe(32);
  });

  it("returns a fallback struct for malformed bytes", () => {
    const garbage = new Uint8Array([0xff, 0xff, 0xff, 0xff]);
    const artifact = __test__parseSealedArtifact(garbage);
    expect(artifact.producer).toBe("(malformed)");
    expect(artifact.signatureValid).toBe(false);
  });
});

describe("SealedView — network fragment security", () => {
  let originalFetch: typeof fetch;
  let fetchMock: Mock;
  const capturedUrls: string[] = [];

  beforeEach(() => {
    capturedUrls.length = 0;
    originalFetch = globalThis.fetch;
    fetchMock = vi.fn(async (url: RequestInfo | URL) => {
      const urlStr = typeof url === "string" ? url : url.toString();
      capturedUrls.push(urlStr);
      const sealedCbor = buildSealedCbor();
      return new Response(sealedCbor, {
        status: 200,
        headers: { "content-type": "application/cbor" },
      });
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    localStorage.clear();
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    localStorage.clear();
  });

  it("fails closed when the installed WASM cannot open sealed memories", async () => {
    renderSealedView(VALID_HASH, "#k=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
    expect(await screen.findByRole("alert")).toHaveTextContent("requires a newer version");
    expect(screen.queryByTestId("sealed-open")).not.toBeInTheDocument();
  });

  it("preserves native key commitment rejection", async () => {
    vi.mocked(loadWasm).mockResolvedValueOnce({
      open_with_key: () => { throw new Error("key commitment mismatch"); },
    } as unknown as Awaited<ReturnType<typeof loadWasm>>);
    renderSealedView(VALID_HASH, "#k=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
    expect(await screen.findByTestId("sealed-kc-fail")).toHaveTextContent("Key commitment check failed");
    expect(screen.queryByTestId("sealed-open")).not.toBeInTheDocument();
  });

  it("decodes the byte result only after the native opener succeeds", async () => {
    const open = vi.fn(() => Uint8Array.from(new TextEncoder().encode(JSON.stringify({ content: "verified plaintext" }))));
    vi.mocked(loadWasm).mockResolvedValueOnce({
      open_with_key: open,
    } as unknown as Awaited<ReturnType<typeof loadWasm>>);
    renderSealedView(VALID_HASH, "#k=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
    expect(await screen.findByTestId("sealed-plaintext")).toHaveTextContent("verified plaintext");
    expect(open).toHaveBeenCalledOnce();
    expect(open.mock.calls[0]).toHaveLength(2);
  });

  it.each([new Uint8Array(), Uint8Array.from(new TextEncoder().encode("sensitive malformed plaintext"))])(
    "rejects malformed decrypted memory without displaying its bytes",
    async (bytes) => {
      vi.mocked(loadWasm).mockResolvedValueOnce({
        open_with_key: () => bytes,
      } as unknown as Awaited<ReturnType<typeof loadWasm>>);
      renderSealedView(VALID_HASH, "#k=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
      expect(await screen.findByRole("alert")).toHaveTextContent("Invalid decrypted memory format");
      expect(screen.queryByText(/sensitive malformed plaintext/)).not.toBeInTheDocument();
      expect(screen.queryByTestId("sealed-open")).not.toBeInTheDocument();
    },
  );

  it("fragment_K_is_never_sent_in_any_network_request", async () => {
    const fakeKey = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    renderSealedView(VALID_HASH, `#k=${fakeKey}`);

    // Wait for the fetch to complete.
    await waitFor(
      () => {
        expect(fetchMock).toHaveBeenCalled();
      },
      { timeout: 3000 },
    );

    // Verify that NO network request contains the fragment key.
    for (const url of capturedUrls) {
      expect(url).not.toContain(fakeKey);
      expect(url).not.toContain("#k=");
      expect(url).not.toContain("#");
    }

    // Also verify the request bodies (POST/PUT). All our fetch calls are GETs,
    // so body is null — but double-check via call args.
    for (const call of fetchMock.mock.calls) {
      const [_url, init] = call as [RequestInfo | URL, RequestInit | undefined];
      if (init?.body) {
        const body = String(init.body);
        expect(body).not.toContain(fakeKey);
        expect(body).not.toContain("#k=");
      }
    }
  });

  it("network_log_contains_no_request_with_K", async () => {
    const sensitiveKey = "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
    renderSealedView(VALID_HASH, `#k=${sensitiveKey}`);

    await waitFor(() => expect(fetchMock).toHaveBeenCalled(), {
      timeout: 3000,
    });

    // Confirm the URLs only contain the memory hash, not the key.
    expect(capturedUrls.length).toBeGreaterThan(0);
    for (const url of capturedUrls) {
      expect(url).toContain(VALID_HASH);
      expect(url).not.toContain("CCCCCCCCCCC");
    }
  });

  it("shows_sealed_metadata_without_a_fragment_key", async () => {
    // No fragment key — should show public metadata only.
    renderSealedView(VALID_HASH, "");

    const meta = await screen.findByTestId("sealed-meta", {}, { timeout: 3000 });
    expect(meta).toBeInTheDocument();

    const badge = screen.getByTestId("sealed-badge");
    expect(badge).toHaveTextContent("Sealed");

    // No-key panel should appear.
    await waitFor(() =>
      expect(screen.getByTestId("sealed-no-key")).toBeInTheDocument(),
    );
  });

  it("shows_error_state_when_server_returns_404", async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(null, { status: 404 }),
    );
    renderSealedView(VALID_HASH, "");
    await waitFor(
      () => expect(screen.getByTestId("sealed-error")).toBeInTheDocument(),
      { timeout: 3000 },
    );
    expect(screen.getByTestId("sealed-error")).toHaveTextContent(
      "Memory not found",
    );
  });
});

describe("SealedView — invalid hash in URL", () => {
  it("shows_error_for_malformed_hash", () => {
    render(
      <MemoryRouter initialEntries={["/m/not-a-hash"]}>
        <Routes>
          <Route path="/m/:hash" element={<SealedView />} />
        </Routes>
      </MemoryRouter>,
    );
    expect(
      screen.getByText(/hash in this URL is malformed/i),
    ).toBeInTheDocument();
  });
});

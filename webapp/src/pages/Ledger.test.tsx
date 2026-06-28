import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Ledger from "./Ledger";
import type { ArtifactPage } from "../lib/ledger";

// SiteFooter pulls in unrelated state we don't exercise here.
vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));

// Mock the ledger client but keep the REAL sample data + filtering semantics,
// so "search filters the list" is exercised honestly without any network.
vi.mock("../lib/ledger", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/ledger")>("../lib/ledger");
  return {
    ...actual,
    fetchArtifacts: vi.fn(
      async (opts?: { q?: string }): Promise<ArtifactPage> => {
        const all = actual.sampleArtifacts();
        const q = opts?.q?.toLowerCase().trim();
        const artifacts = q
          ? all.filter(
              (a) =>
                a.content.toLowerCase().includes(q) ||
                a.tags.some((t) => t.toLowerCase().includes(q)),
            )
          : all;
        return { artifacts, total: artifacts.length, sample: true };
      },
    ),
  };
});

import { fetchArtifacts } from "../lib/ledger";

function renderLedger() {
  return render(
    <MemoryRouter initialEntries={["/ledger"]}>
      <Ledger />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.mocked(fetchArtifacts).mockClear();
  // jsdom exposes `navigator.clipboard` as a getter-only prop; redefine it.
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: vi.fn(async () => undefined) },
  });
});

describe("Ledger page", () => {
  it("renders_receipt_cards_from_sample_data", async () => {
    renderLedger();
    expect(
      await screen.findByText(/Shipped v0\.2 on Tuesday/i),
    ).toBeInTheDocument();
    // A semantic list of artifacts is present.
    const list = screen.getByRole("list", { name: /artifacts/i });
    expect(within(list).getAllByRole("listitem").length).toBeGreaterThan(0);
  });

  it("shows_sample_not_live_banner_when_sample_true", async () => {
    renderLedger();
    const banner = await screen.findByTestId("sample-banner");
    expect(banner).toHaveTextContent(/sample/i);
    expect(banner).toHaveTextContent(/not live/i);
  });

  it("renders_local_tx_as_plain_text_not_a_link", async () => {
    renderLedger();
    // a3 sample row carries solana_tx + arweave_tx "local:offline-7f3a".
    const localTxs = await screen.findAllByText(/local:offline-7f3a/i);
    expect(localTxs.length).toBeGreaterThan(0);
    for (const el of localTxs) {
      expect(el.closest("a")).toBeNull();
    }
  });

  it("renders_real_anchor_as_explorer_link", async () => {
    renderLedger();
    await screen.findByText(/Shipped v0\.2 on Tuesday/i);
    const solanaLink = screen
      .getAllByRole("link")
      .find((a) =>
        a.getAttribute("href")?.startsWith("https://explorer.solana.com/tx/"),
      );
    expect(solanaLink).toBeDefined();
    expect(solanaLink).toHaveAttribute("target", "_blank");
    expect(solanaLink).toHaveAttribute(
      "rel",
      expect.stringContaining("noopener"),
    );
  });

  it("search_narrows_the_list", async () => {
    const user = userEvent.setup();
    renderLedger();
    await screen.findByText(/Shipped v0\.2 on Tuesday/i);

    const input = screen.getByRole("searchbox", { name: /recall by meaning/i });
    await user.type(input, "turboquant{enter}");

    await waitFor(() =>
      expect(
        screen.queryByText(/Shipped v0\.2 on Tuesday/i),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByText(/TurboQuant bit width stays at 4/i),
    ).toBeInTheDocument();
    expect(fetchArtifacts).toHaveBeenLastCalledWith({ q: "turboquant" });
  });

  it("write_mode_filter_chip_narrows_to_on_chain_rows", async () => {
    const user = userEvent.setup();
    renderLedger();
    await screen.findByText(/Shipped v0\.2 on Tuesday/i);

    await user.click(screen.getByRole("button", { name: /on-chain/i }));

    // Local-only row disappears; on-chain row stays.
    expect(
      screen.queryByText(/TurboQuant bit width stays at 4/i),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/Shipped v0\.2 on Tuesday/i)).toBeInTheDocument();
  });

  it("copies_content_hash_to_clipboard", async () => {
    const user = userEvent.setup();
    renderLedger();
    await screen.findByText(/Shipped v0\.2 on Tuesday/i);

    const [firstCopy] = screen.getAllByRole("button", {
      name: /copy content hash/i,
    });
    await user.click(firstCopy!);

    // userEvent.setup() installs its own clipboard stub; read the value back.
    expect(await navigator.clipboard.readText()).toBe(
      "b1946ac92492d2347c6235b4d2611184d3f0a3f9e1c6a2e7c0d4b8a1f2e3d4c5",
    );
  });

  it("shows_empty_state_when_no_matches", async () => {
    const user = userEvent.setup();
    renderLedger();
    await screen.findByText(/Shipped v0\.2 on Tuesday/i);

    const input = screen.getByRole("searchbox", { name: /recall by meaning/i });
    await user.type(input, "zzz-no-such-memory{enter}");

    expect(await screen.findByTestId("ledger-empty")).toBeInTheDocument();
  });

  it("shows_error_state_when_fetch_rejects", async () => {
    vi.mocked(fetchArtifacts).mockRejectedValueOnce(new Error("boom"));
    renderLedger();
    const err = await screen.findByTestId("ledger-error");
    expect(err).toHaveAttribute("role", "alert");
  });
});

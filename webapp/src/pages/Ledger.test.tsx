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

// Local fixture standing in for live `/artifacts` rows, with the variety the
// rendering tests need: a real-anchored participate row, a local-only row, and
// a `local:`-prefixed anchor row. The mock filters it by `q` so "search filters
// the list" is exercised honestly without any network.
import type { Artifact } from "../lib/ledger";

const FIXTURE: Artifact[] = [
  {
    id: "a1c0ffee-0001-4a00-9c01-000000000001",
    content: "Shipped v0.2 on Tuesday with Alex as release owner.",
    content_hash:
      "b1946ac92492d2347c6235b4d2611184d3f0a3f9e1c6a2e7c0d4b8a1f2e3d4c5",
    tags: ["release", "v0.2"],
    solana_tx: "5Nf5h5x2qQk8wq7Yk3J9p1d2c3b4a5n6m7l8k9j0i1h2g3f4e5d6c7b8a9",
    arweave_tx: "kTQ7t1f9c2X3v4B5n6M7l8K9j0I1h2G3f4E5d6C7b8A",
    created_at: "2026-06-26T00:00:00.000Z",
    write_mode: "participate",
  },
  {
    id: "a1c0ffee-0002-4a00-9c01-000000000002",
    content: "Decided TurboQuant bit width stays at 4 for the production DB.",
    content_hash:
      "3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b",
    tags: ["architecture", "turboquant"],
    solana_tx: null,
    arweave_tx: null,
    created_at: "2026-06-24T00:00:00.000Z",
    write_mode: "local",
  },
  {
    id: "a1c0ffee-0003-4a00-9c01-000000000003",
    content: "Agent note: recall spans both local and participate writes.",
    content_hash:
      "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae",
    tags: ["recall"],
    solana_tx: "local:offline-7f3a",
    arweave_tx: "local:offline-7f3a",
    created_at: "2026-06-22T00:00:00.000Z",
    write_mode: "local",
  },
];

vi.mock("../lib/ledger", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/ledger")>("../lib/ledger");
  return {
    ...actual,
    fetchArtifacts: vi.fn(
      async (opts?: { q?: string; limit?: number }): Promise<ArtifactPage> => {
        const q = opts?.q?.toLowerCase().trim();
        const artifacts = q
          ? FIXTURE.filter(
              (a) =>
                a.content.toLowerCase().includes(q) ||
                a.tags.some((t) => t.toLowerCase().includes(q)),
            )
          : FIXTURE;
        return { artifacts, total: artifacts.length };
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
  it("renders_receipt_cards_from_live_data", async () => {
    renderLedger();
    expect(
      await screen.findByText(/Shipped v0\.2 on Tuesday/i),
    ).toBeInTheDocument();
    // A semantic list of artifacts is present.
    const list = screen.getByRole("list", { name: /artifacts/i });
    expect(within(list).getAllByRole("listitem").length).toBeGreaterThan(0);
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
    expect(fetchArtifacts).toHaveBeenLastCalledWith({
      q: "turboquant",
      limit: 200,
    });
  });

  it("requests_the_bounded_maximum_so_chain_rows_do_not_hide_on_node_rows", async () => {
    renderLedger();
    await screen.findByText(/Shipped v0\.2 on Tuesday/i);

    expect(fetchArtifacts).toHaveBeenLastCalledWith({ q: "", limit: 200 });
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

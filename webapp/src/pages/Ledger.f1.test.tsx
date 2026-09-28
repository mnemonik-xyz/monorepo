import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import Ledger from "./Ledger";
import type { Artifact, ArtifactPage } from "../lib/ledger";

/**
 * Tests for the F1 label in Ledger.tsx (tech-spec §2, §10).
 *
 * Finding F1: anchored rows labelled `private` before encryption existed are
 * readable by anyone on Arweave. The UI must say:
 * "Published before encryption existed. Readable on Arweave."
 *
 * The label appears when:
 *   - write_mode !== "local" (anchored on-chain)
 *   - privacy is absent (undefined) OR "plaintext"
 * It does NOT appear when:
 *   - privacy === "sealed"
 *   - write_mode === "local"
 */

vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));

const PLAINTEXT_ANCHORED: Artifact = {
  id: "f1-test-0001-4a00-9c01-000000000001",
  content: "Old anchored memory written before encryption.",
  content_hash: "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2",
  tags: [],
  solana_tx: "5Nf5h5x2qQk8wq7Yk3J9p1d2c3b4a5n6m7l8k9j0",
  arweave_tx: "kTQ7t1f9c2X3v4B5n6M7l8K9j0I1h2G3f",
  created_at: "2026-01-01T00:00:00.000Z",
  write_mode: "anchored",
  // privacy absent — legacy row, should get F1 label
};

const PLAINTEXT_ANCHORED_EXPLICIT: Artifact = {
  ...PLAINTEXT_ANCHORED,
  id: "f1-test-0002-4a00-9c01-000000000002",
  privacy: "plaintext",
};

const SEALED_ANCHORED: Artifact = {
  ...PLAINTEXT_ANCHORED,
  id: "f1-test-0003-4a00-9c01-000000000003",
  content: "Sealed encrypted memory.",
  privacy: "sealed",
  write_mode: "anchored",
};

const LOCAL_PLAINTEXT: Artifact = {
  ...PLAINTEXT_ANCHORED,
  id: "f1-test-0004-4a00-9c01-000000000004",
  content: "Local only memory.",
  write_mode: "local",
  solana_tx: null,
  arweave_tx: null,
  privacy: "plaintext",
};

vi.mock("../lib/ledger", () => {
  // Source-aware mock: the Ledger page calls fetchArtifacts twice
  // (on_node + on_chain). Return each artifact only from its appropriate
  // source so they don't appear twice in the merged list.
  return {
    fetchArtifacts: vi.fn(
      async (opts?: {
        q?: string;
        limit?: number;
        source?: "all" | "on_node" | "on_chain";
      }): Promise<ArtifactPage> => {
        if (opts?.source === "on_node") {
          return { artifacts: [LOCAL_PLAINTEXT], total: 1 };
        }
        // on_chain or unspecified
        return {
          artifacts: [
            PLAINTEXT_ANCHORED,
            PLAINTEXT_ANCHORED_EXPLICIT,
            SEALED_ANCHORED,
          ],
          total: 3,
        };
      },
    ),
    fetchAttestationTimeline: vi.fn(async () => ({
      buckets: [],
      total_on_node: 0,
      total_on_chain: 0,
      unique_users: 0,
    })),
  };
});

function renderLedger() {
  return render(
    <MemoryRouter initialEntries={["/ledger"]}>
      <Ledger />
    </MemoryRouter>,
  );
}

describe("Ledger — F1 plaintext label", () => {
  it("shows_f1_label_for_anchored_row_without_privacy_field", async () => {
    renderLedger();
    // Wait until the artifact list is rendered (no loading state).
    const list = await screen.findByRole("list", { name: /artifacts/i }, { timeout: 5000 });
    const items = within(list).getAllByRole("listitem");
    expect(items.length).toBeGreaterThan(0);
    const labels = screen.getAllByTestId("f1-label");
    // Should appear for PLAINTEXT_ANCHORED (no privacy) and PLAINTEXT_ANCHORED_EXPLICIT.
    expect(labels.length).toBeGreaterThanOrEqual(2);
    expect(labels[0]).toHaveTextContent(
      "Published before encryption existed. Readable on Arweave.",
    );
  });

  it("shows_f1_label_for_anchored_row_with_privacy_plaintext", async () => {
    renderLedger();
    const list = await screen.findByRole("list", { name: /artifacts/i }, { timeout: 5000 });
    within(list).getAllByRole("listitem");
    // Both plaintext anchored rows get the label.
    const labels = screen.getAllByTestId("f1-label");
    expect(labels.length).toBeGreaterThanOrEqual(2);
  });

  it("does_not_show_f1_label_for_sealed_anchored_row", async () => {
    renderLedger();
    const list = await screen.findByRole("list", { name: /artifacts/i }, { timeout: 5000 });
    within(list).getAllByRole("listitem");
    // Only the plaintext rows should have the label — not the sealed row.
    const labels = screen.getAllByTestId("f1-label");
    // There are 2 plaintext-anchored rows and 1 sealed row and 1 local row.
    // So exactly 2 labels should appear.
    expect(labels.length).toBe(2);
  });

  it("does_not_show_f1_label_for_local_plaintext_row", async () => {
    renderLedger();
    // Wait for all artifacts to render.
    const list = await screen.findByRole("list", { name: /artifacts/i }, { timeout: 5000 });
    within(list).getAllByRole("listitem");
    // Local rows are not on-chain — no F1 label.
    const labels = screen.getAllByTestId("f1-label");
    // Only the 2 anchored plaintext rows get labels.
    expect(labels.length).toBe(2);
  });
});

import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { ROADMAP_ITEMS } from "../lib/roadmap.generated";
import Roadmap from "./Roadmap";

// SiteFooter pulls in network code we don't care about in a static-rendering
// test; stub it to a no-op so the snapshot of the board stays focused.
vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));

function renderRoadmap() {
  return render(
    <MemoryRouter initialEntries={["/roadmap"]}>
      <Roadmap />
    </MemoryRouter>,
  );
}

describe("Roadmap page", () => {
  it("renders_three_status_columns_with_correct_counts", () => {
    renderRoadmap();

    const expected = {
      shipped: ROADMAP_ITEMS.filter((i) => i.status === "shipped").length,
      in_progress: ROADMAP_ITEMS.filter((i) => i.status === "in_progress")
        .length,
      planned: ROADMAP_ITEMS.filter((i) => i.status === "planned").length,
    };

    const board = screen.getByTestId("roadmap-board");
    expect(board).toBeInTheDocument();

    for (const status of ["shipped", "in_progress", "planned"] as const) {
      const col = within(board).getByTestId(`roadmap-column-${status}`);
      const cards = within(col).queryAllByRole("article");
      expect(cards.length).toBe(expected[status]);
    }
  });

  it("renders_each_item_title_in_its_status_column", () => {
    renderRoadmap();
    for (const item of ROADMAP_ITEMS) {
      const col = screen.getByTestId(`roadmap-column-${item.status}`);
      expect(within(col).getByText(item.title)).toBeInTheDocument();
    }
  });

  it("links_back_to_landing_via_back_chevron", () => {
    renderRoadmap();
    const backLink = screen.getByRole("link", { name: /←\s*back/i });
    expect(backLink).toHaveAttribute("href", "/");
  });
});

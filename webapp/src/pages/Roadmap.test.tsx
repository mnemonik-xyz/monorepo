import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import Roadmap from "./Roadmap";

// SiteFooter pulls in unrelated state we don't care about here.
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
  it("renders_h1_from_markdown", () => {
    renderRoadmap();
    expect(
      screen.getByRole("heading", { level: 1, name: /roadmap/i }),
    ).toBeInTheDocument();
  });

  it("renders_three_status_sections_as_h2", () => {
    renderRoadmap();
    const article = screen.getByTestId("roadmap-article");
    const h2s = within(article).getAllByRole("heading", { level: 2 });
    const labels = h2s.map((h) => h.textContent?.trim().toLowerCase());
    expect(labels).toContain("shipped");
    expect(labels).toContain("in progress");
    expect(labels).toContain("planned");
  });

  it("links_back_to_landing_via_back_chevron", () => {
    renderRoadmap();
    const backLink = screen.getByRole("link", { name: /←\s*back/i });
    expect(backLink).toHaveAttribute("href", "/");
  });

  it("external_source_links_open_in_new_tab", () => {
    renderRoadmap();
    const externals = screen
      .getAllByRole("link")
      .filter((a) => a.getAttribute("href")?.startsWith("https://"));
    expect(externals.length).toBeGreaterThan(0);
    for (const a of externals) {
      expect(a).toHaveAttribute("target", "_blank");
      expect(a).toHaveAttribute("rel", expect.stringContaining("noopener"));
    }
  });
});

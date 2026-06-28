import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import SiteHeader from "./SiteHeader";

function renderHeader() {
  return render(
    <MemoryRouter>
      <SiteHeader />
    </MemoryRouter>,
  );
}

describe("SiteHeader navigation", () => {
  it("links_to_ledger", () => {
    renderHeader();
    const link = screen.getByRole("link", { name: /ledger/i });
    expect(link).toHaveAttribute("href", "/ledger");
    expect(link).toHaveTextContent("Ledger");
  });

  it("links_to_analytics", () => {
    renderHeader();
    const link = screen.getByRole("link", { name: /analytics/i });
    expect(link).toHaveAttribute("href", "/analytics");
    expect(link).toHaveTextContent("Analytics");
  });

  it("links_to_blog", () => {
    renderHeader();
    const link = screen.getByRole("link", { name: /blog/i });
    expect(link).toHaveAttribute("href", "/blog");
    expect(link).toHaveTextContent("Blog");
  });
});

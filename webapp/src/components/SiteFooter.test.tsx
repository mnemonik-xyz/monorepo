import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import SiteFooter from "./SiteFooter";

function renderFooter() {
  return render(
    <MemoryRouter>
      <SiteFooter />
    </MemoryRouter>,
  );
}

describe("SiteFooter navigation", () => {
  it("links_to_ledger", () => {
    renderFooter();
    expect(screen.getByRole("link", { name: /ledger/i })).toHaveAttribute(
      "href",
      "/ledger",
    );
  });

  it("links_to_analytics", () => {
    renderFooter();
    expect(screen.getByRole("link", { name: /analytics/i })).toHaveAttribute(
      "href",
      "/analytics",
    );
  });

  it("links_to_blog", () => {
    renderFooter();
    expect(screen.getByRole("link", { name: /blog/i })).toHaveAttribute(
      "href",
      "/blog",
    );
  });
});

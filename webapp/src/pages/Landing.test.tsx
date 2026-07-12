import { render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Landing from "./Landing";

vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));
vi.mock("../lib/seo", () => ({
  Seo: () => null,
  organizationJsonLd: () => ({}),
}));

function renderLanding() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <Landing />
    </MemoryRouter>,
  );
}

describe("Landing page", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows users memories and records counters", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(
      new Response(
        JSON.stringify({
          unique_users: 7,
          saved_on_node: 42,
          saved_onchain: 11,
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );

    renderLanding();

    const strip = screen.getByTestId("traction-strip");
    await waitFor(() => expect(within(strip).getByText("7")).toBeVisible());
    expect(within(strip).getByText("42")).toBeVisible();
    expect(within(strip).getByText("11")).toBeVisible();
    expect(within(strip).getByText("Users")).toBeVisible();
    expect(within(strip).getByText("Memories")).toBeVisible();
    expect(within(strip).getByText("Records")).toBeVisible();
  });
});

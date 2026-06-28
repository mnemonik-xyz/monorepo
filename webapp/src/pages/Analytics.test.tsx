import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import TimelineChart from "../components/TimelineChart";
import type { AttestationTimeline, TimeRange } from "../lib/ledger";
import Analytics from "./Analytics";

// SiteFooter / Seo pull in unrelated concerns (links, React 19 head hoisting)
// that this page test does not care about.
vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));
vi.mock("../lib/seo", () => ({ Seo: () => null }));

const TIMELINE_30D: AttestationTimeline = {
  buckets: [
    { date: "2026-06-01", on_node: 10, on_chain: 3 },
    { date: "2026-06-02", on_node: 20, on_chain: 7 },
    { date: "2026-06-03", on_node: 15, on_chain: 5 },
  ],
  total_on_node: 45,
  total_on_chain: 15,
  unique_users: 23,
};

const TIMELINE_90D: AttestationTimeline = {
  buckets: [
    { date: "2026-04-01", on_node: 100, on_chain: 40 },
    { date: "2026-05-01", on_node: 200, on_chain: 80 },
    { date: "2026-06-01", on_node: 267, on_chain: 114 },
  ],
  total_on_node: 567,
  total_on_chain: 234,
  unique_users: 88,
};

const fetchAttestationTimeline = vi.fn();

vi.mock("../lib/ledger", () => ({
  fetchAttestationTimeline: (range: TimeRange) =>
    fetchAttestationTimeline(range),
}));

function renderAnalytics() {
  return render(
    <MemoryRouter initialEntries={["/analytics"]}>
      <Analytics />
    </MemoryRouter>,
  );
}

/** Line paths carry a stroke; the area fill path has stroke="none". */
function linePaths(container: HTMLElement): SVGPathElement[] {
  return [...container.querySelectorAll<SVGPathElement>("path")].filter((p) =>
    p.getAttribute("stroke")?.startsWith("var(--color-accent"),
  );
}

describe("Analytics page", () => {
  beforeEach(() => {
    // mockReset clears both calls and any per-test implementation, so each
    // test starts from the same default (range -> deterministic timeline).
    fetchAttestationTimeline.mockReset();
    fetchAttestationTimeline.mockImplementation(async (range: TimeRange) =>
      range === "90d" ? TIMELINE_90D : TIMELINE_30D,
    );
  });

  it("renders_svg_path_from_timeline", async () => {
    const { container } = renderAnalytics();
    await screen.findByTestId("timeline-chart");
    // The on-node and on-chain series each render an SVG <path>.
    const paths = container.querySelectorAll("svg path");
    expect(paths.length).toBeGreaterThanOrEqual(1);
    // No degenerate geometry leaked into any rendered path.
    for (const p of paths) {
      expect(p.getAttribute("d") ?? "").not.toContain("NaN");
    }
  });

  it("svg_path_shape_ties_to_buckets_and_has_no_nan", () => {
    // Rendered as a pure component so the scale math is asserted directly.
    const { container } = render(
      <TimelineChart buckets={TIMELINE_30D.buckets} ariaLabel="test" />,
    );
    const lines = linePaths(container);
    expect(lines.length).toBe(2); // on-node + on-chain
    for (const p of lines) {
      const d = p.getAttribute("d") ?? "";
      expect(d).not.toContain("NaN");
      // One move-to, then one line-to per subsequent bucket.
      expect((d.match(/M/g) ?? []).length).toBe(1);
      expect((d.match(/L/g) ?? []).length).toBe(
        TIMELINE_30D.buckets.length - 1,
      );
    }
  });

  it("chart_single_bucket_renders_markers_without_nan", () => {
    // n === 1 exercises the centred-x branch and the point-marker fallback.
    const { container } = render(
      <TimelineChart
        buckets={[{ date: "2026-06-01", on_node: 5, on_chain: 2 }]}
        ariaLabel="one"
      />,
    );
    expect(container.querySelectorAll("circle").length).toBeGreaterThanOrEqual(
      2,
    );
    for (const p of container.querySelectorAll("path")) {
      expect(p.getAttribute("d") ?? "").not.toContain("NaN");
    }
  });

  it("chart_all_zero_counts_has_no_nan", () => {
    // All-zero counts exercise the max>=1 guard (no divide-by-zero / NaN).
    const { container } = render(
      <TimelineChart
        buckets={[
          { date: "2026-06-01", on_node: 0, on_chain: 0 },
          { date: "2026-06-02", on_node: 0, on_chain: 0 },
        ]}
        ariaLabel="zero"
      />,
    );
    for (const p of container.querySelectorAll("path")) {
      expect(p.getAttribute("d") ?? "").not.toContain("NaN");
    }
    for (const t of container.querySelectorAll("text")) {
      expect(t.textContent ?? "").not.toContain("NaN");
    }
  });

  it("empty_buckets_show_empty_state_and_no_chart", async () => {
    fetchAttestationTimeline.mockResolvedValue({
      buckets: [],
      total_on_node: 0,
      total_on_chain: 0,
      unique_users: 0,
    } satisfies AttestationTimeline);
    renderAnalytics();
    expect(
      await screen.findByText(/no attestations recorded/i),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("timeline-chart")).toBeNull();
  });

  it("shows_loading_state_before_resolve", async () => {
    let resolve!: (v: AttestationTimeline) => void;
    fetchAttestationTimeline.mockImplementation(
      () =>
        new Promise<AttestationTimeline>((r) => {
          resolve = r;
        }),
    );
    renderAnalytics();
    expect(screen.getByText(/loading timeline/i)).toBeInTheDocument();
    await act(async () => {
      resolve(TIMELINE_30D);
    });
    await screen.findByTestId("timeline-chart");
  });

  it("shows_error_state_when_fetch_rejects", async () => {
    fetchAttestationTimeline.mockRejectedValue(new Error("boom"));
    renderAnalytics();
    expect(await screen.findByText(/could not load/i)).toBeInTheDocument();
    expect(screen.queryByTestId("timeline-chart")).toBeNull();
  });

  it("range_toggle_refetches_and_updates", async () => {
    renderAnalytics();
    // Initial 30d load.
    await waitFor(() =>
      expect(fetchAttestationTimeline).toHaveBeenCalledWith("30d"),
    );
    expect(screen.getByTestId("stat-on-node")).toHaveTextContent("45");

    await userEvent.click(screen.getByRole("button", { name: /90d/i }));

    await waitFor(() =>
      expect(fetchAttestationTimeline).toHaveBeenCalledWith("90d"),
    );
    await waitFor(() =>
      expect(screen.getByTestId("stat-on-node")).toHaveTextContent("567"),
    );
  });

  it("stale_response_is_ignored_last_write_wins", async () => {
    // Hand-controlled deferreds per range so we can resolve them out of order.
    const resolvers: Partial<Record<TimeRange, () => void>> = {};
    fetchAttestationTimeline.mockImplementation(
      (range: TimeRange) =>
        new Promise<AttestationTimeline>((resolve) => {
          resolvers[range] = () =>
            resolve(range === "90d" ? TIMELINE_90D : TIMELINE_30D);
        }),
    );

    renderAnalytics();
    await waitFor(() =>
      expect(fetchAttestationTimeline).toHaveBeenCalledWith("30d"),
    );

    // Switch to 90d while the 30d request is still in flight.
    await userEvent.click(screen.getByRole("button", { name: /90d/i }));
    await waitFor(() =>
      expect(fetchAttestationTimeline).toHaveBeenCalledWith("90d"),
    );

    // The stale 30d response lands first — it must be discarded.
    await act(async () => {
      resolvers["30d"]?.();
    });
    expect(screen.getByTestId("stat-on-node")).not.toHaveTextContent("45");

    // The current 90d response then resolves and wins.
    await act(async () => {
      resolvers["90d"]?.();
    });
    await waitFor(() =>
      expect(screen.getByTestId("stat-on-node")).toHaveTextContent("567"),
    );
  });

  it("summary_cells_show_totals", async () => {
    renderAnalytics();
    await screen.findByTestId("timeline-chart");
    expect(screen.getByTestId("stat-on-node")).toHaveTextContent("45");
    expect(screen.getByTestId("stat-on-chain")).toHaveTextContent("15");
    expect(screen.getByTestId("stat-users")).toHaveTextContent("23");
  });

  it("chart_has_img_role_and_aria_label", async () => {
    renderAnalytics();
    const chart = await screen.findByTestId("timeline-chart");
    const img = within(chart).getByRole("img");
    expect(img).toHaveAttribute("aria-label");
    expect(img.getAttribute("aria-label")?.length ?? 0).toBeGreaterThan(0);
  });
});

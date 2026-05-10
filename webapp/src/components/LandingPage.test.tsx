import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LandingPage from "./LandingPage";
import type { Message } from "../types";

function renderLanding() {
  const props = {
    onStartChat: vi.fn(),
    messages: [] as Message[],
    setMessages: vi.fn(),
    messageCount: 0,
    setMessageCount: vi.fn(),
    sessionId: "test-session",
    onNavigateToChat: vi.fn(),
  };
  return render(<LandingPage {...props} />);
}

describe("LandingPage traction counters", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders_three_counters_after_stats_load", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockImplementation(
      (input: RequestInfo) => {
        if (typeof input === "string" && input === "/stats") {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                unique_users: 1234,
                saved_on_node: 5678,
                saved_onchain: 90,
              }),
              { status: 200, headers: { "Content-Type": "application/json" } },
            ),
          );
        }
        return Promise.reject(new Error(`unexpected fetch: ${input}`));
      },
    );

    renderLanding();

    const tractionRegion = await screen.findByRole("region", {
      name: /network traction/i,
    });

    await waitFor(() => {
      expect(tractionRegion).toHaveTextContent("1,234");
    });
    expect(tractionRegion).toHaveTextContent("5,678");
    expect(tractionRegion).toHaveTextContent("90");
    expect(tractionRegion).toHaveTextContent(/unique users/i);
    expect(tractionRegion).toHaveTextContent(/memories on node/i);
    expect(tractionRegion).toHaveTextContent(/memories on-chain/i);
  });

  it("hides_counters_when_stats_endpoint_fails", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockImplementation(
      (input: RequestInfo) => {
        if (typeof input === "string" && input === "/stats") {
          return Promise.resolve(new Response("err", { status: 500 }));
        }
        return Promise.reject(new Error(`unexpected fetch: ${input}`));
      },
    );

    renderLanding();

    // Wait for the fetch to settle, then assert the strip never renders.
    await waitFor(() => {
      expect(fetch).toHaveBeenCalledWith("/stats", { method: "GET" });
    });
    expect(
      screen.queryByRole("region", { name: /network traction/i }),
    ).toBeNull();
  });
});

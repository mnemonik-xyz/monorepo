import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { DeliveryStatus } from "./DeliveryStatus";
import type { DeliveryStatus as Delivery } from "../lib/delivery";

const ID = "rBjKMGzX-t-oYMOaKkXOE42q-1UN-YkPT7fFuVhNJDc";

describe("DeliveryStatus", () => {
  it("shows the reached stage and a link that works now", async () => {
    const check = vi.fn(
      async (): Promise<Delivery> => ({
        stage: "uploaded",
        readUrl: `https://turbo-gateway.com/${ID}`,
      }),
    );
    render(<DeliveryStatus arweaveTx={ID} check={check} pollMs={60_000} />);
    await waitFor(() =>
      expect(screen.getByTestId("delivery-step-uploaded")).toHaveAttribute("data-state", "done"),
    );
    expect(screen.getByTestId("delivery-step-bundled")).toHaveAttribute("data-state", "waiting");
    expect(screen.getByTestId("delivery-read-link")).toHaveAttribute(
      "href",
      `https://turbo-gateway.com/${ID}`,
    );
    expect(screen.getByTestId("delivery-read-link")).toHaveTextContent("View now");
    expect(screen.queryByText(/Solscan/)).toBeNull();
  });

  it("polls until permanent, then links arweave.net and stops polling", async () => {
    const answers: Delivery[] = [
      { stage: "bundled", bundleId: "B", readUrl: `https://turbo-gateway.com/${ID}` },
      { stage: "permanent", blockHeight: 7, readUrl: `https://arweave.net/${ID}` },
    ];
    const check = vi.fn(async () => answers.shift() ?? answers[0]!);
    render(<DeliveryStatus arweaveTx={ID} check={check} pollMs={5} />);
    await waitFor(() =>
      expect(screen.getByTestId("delivery-step-permanent")).toHaveAttribute("data-state", "done"),
    );
    expect(screen.getByText(/block 7/)).toBeInTheDocument();
    expect(screen.getByTestId("delivery-read-link")).toHaveAttribute(
      "href",
      `https://arweave.net/${ID}`,
    );
    const calls = check.mock.calls.length;
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(check.mock.calls.length).toBe(calls);
  });

  it("does not render or poll for synthetic local ids", async () => {
    const check = vi.fn(async (): Promise<Delivery> => ({ stage: "unknown", readUrl: null }));
    const { container } = render(<DeliveryStatus arweaveTx="local:abc" check={check} pollMs={5} />);
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(check).not.toHaveBeenCalled();
    expect(container).toBeEmptyDOMElement();
  });

  it("never moves back to an earlier stage after a failed check", async () => {
    const answers: Delivery[] = [
      { stage: "bundled", readUrl: `https://turbo-gateway.com/${ID}` },
      { stage: "unknown", readUrl: null },
    ];
    const check = vi.fn(
      async (): Promise<Delivery> => answers.shift() ?? { stage: "unknown", readUrl: null },
    );
    render(<DeliveryStatus arweaveTx={ID} check={check} pollMs={5} />);
    await waitFor(() => expect(check.mock.calls.length).toBeGreaterThanOrEqual(2));
    expect(screen.getByTestId("delivery-step-bundled")).toHaveAttribute("data-state", "done");
  });
});

// Capture-tab tests. Drives the T11 TDD anchor
// `sign_button_calls_signMemory`: mount the tab with a prefilled
// selection, click Sign, and assert the runtime stub was invoked
// with the expected `content` + `tags`. The runtime facade is the
// only seam to mock — popup code never touches the WASM pipeline or
// IndexedDB directly.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Capture } from "../../../src/popup/tabs/Capture.js";
import {
  setRuntime,
  type PopupRuntime,
  type SignMemoryArgs,
  type SignMemoryResult,
} from "../../../src/popup/runtime.js";
import type { ChatAdapter } from "../../../src/runtime/chat/types.js";

function makeRuntime(overrides: Partial<PopupRuntime> = {}): PopupRuntime {
  return {
    loadIdentity: async () => null,
    loadStorageTier: async () => "local" as const,
    getActiveTabAdapter: async () => null,
    getActiveTabSelection: async () => "",
    getActiveTabConversation: async () => null,
    signMemory: async (_args: SignMemoryArgs): Promise<SignMemoryResult> => ({
      attestation_id: "att_deadbeef",
      content_hash: "deadbeef".repeat(8),
      signer_pubkey: "Pub",
      solana_tx: "local:deadbeef",
      arweave_tx: "local:deadbeef",
      created_at: "2026-05-11T00:00:00Z",
    }),
    signRemote: async () => undefined,
    recall: async () => [],
    verify: async () => ({ status: "not_found" }),
    ...overrides,
  };
}

const chatgptAdapter: ChatAdapter = {
  hostPattern: /^chatgpt\.com\//,
  platform: "chatgpt",
  extractConversation: () => [],
  findInputBox: () => null,
  getChatId: () => null,
};

describe("Capture tab", () => {
  beforeEach(() => {
    setRuntime(null);
  });
  afterEach(() => {
    setRuntime(null);
  });

  it("sign_button_calls_signMemory", async () => {
    // TDD anchor #1: clicking Sign with a prefilled selection
    // invokes runtime.signMemory with the textarea text + parsed tags.
    const signMemory = vi
      .fn<[SignMemoryArgs], Promise<SignMemoryResult>>()
      .mockResolvedValue({
        attestation_id: "att_cafef00d",
        content_hash: "cafef00d".repeat(8),
        signer_pubkey: "Pub",
        solana_tx: "local:cafef00d",
        arweave_tx: "local:cafef00d",
        created_at: "2026-05-11T00:00:00Z",
      });
    setRuntime(makeRuntime({ signMemory }));

    render(
      <Capture
        adapter={chatgptAdapter}
        prefilledSelection="captured paragraph"
      />
    );

    const tagInput = screen.getByLabelText("Tags");
    fireEvent.change(tagInput, { target: { value: "alpha, beta" } });

    const signButton = screen.getByRole("button", { name: /^sign$/i });
    fireEvent.click(signButton);

    await waitFor(() => expect(signMemory).toHaveBeenCalledTimes(1));
    const args = signMemory.mock.calls[0]?.[0];
    expect(args?.content).toBe("captured paragraph");
    expect(args?.tags).toContain("alpha");
    expect(args?.tags).toContain("beta");
    expect(args?.tags).toContain("source:chatgpt");
    expect(args?.source?.platform).toBe("chatgpt");
  });

  it("renders the success toast with the truncated attestation id after sign", async () => {
    const signMemory = vi
      .fn<[SignMemoryArgs], Promise<SignMemoryResult>>()
      .mockResolvedValue({
        attestation_id: "att_abcdef0123456789",
        content_hash: "abcd".repeat(16),
        signer_pubkey: "Pub",
        solana_tx: "local:abcdef",
        arweave_tx: "local:abcdef",
        created_at: "2026-05-11T01:00:00Z",
      });
    setRuntime(makeRuntime({ signMemory }));

    render(<Capture adapter={null} prefilledSelection="hello mnemonik" />);
    fireEvent.click(screen.getByRole("button", { name: /^sign$/i }));

    const toast = await screen.findByRole("status");
    expect(toast.textContent ?? "").toMatch(/Signed/);
    expect(toast.textContent ?? "").toMatch(/att_abcdef/);
  });

  it("rejects empty content with an error toast", async () => {
    const signMemory = vi.fn();
    setRuntime(makeRuntime({ signMemory }));

    render(<Capture adapter={null} prefilledSelection="" />);
    fireEvent.click(screen.getByRole("button", { name: /^sign$/i }));

    const toast = await screen.findByRole("status");
    expect(toast.textContent ?? "").toMatch(/Add something/i);
    expect(signMemory).not.toHaveBeenCalled();
  });
});

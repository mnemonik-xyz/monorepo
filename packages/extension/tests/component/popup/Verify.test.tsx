// Verify-tab tests. Exercises the three terminal states: verified,
// tampered, not_found. The runtime stub returns each outcome on demand
// so we don't need to spin up the real WASM verifier or the IndexedDB
// store.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Verify } from "../../../src/popup/tabs/Verify.js";
import {
  setRuntime,
  type PopupRuntime,
  type VerifyArgs,
  type VerifyOutcome,
} from "../../../src/popup/runtime.js";

function makeRuntime(
  verifyFn: (args: VerifyArgs) => Promise<VerifyOutcome>,
): PopupRuntime {
  return {
    loadIdentity: async () => null,
    loadStorageTier: async () => "local" as const,
    getActiveTabAdapter: async () => null,
    getActiveTabSelection: async () => "",
    getActiveTabConversation: async () => null,
    signMemory: async () => ({
      attestation_id: "att_x",
      content_hash: "x".repeat(64),
      signer_pubkey: "Pub",
      solana_tx: "local:x",
      arweave_tx: "local:x",
      created_at: "2026-05-11T00:00:00Z",
    }),
    signRemote: async () => undefined,
    recall: async () => [],
    verify: verifyFn,
    auth: {
      signIn: async () => {
        throw new Error("auth.signIn stub: not configured in test");
      },
      lookupExisting: async () => {
        throw new Error("auth.lookupExisting stub: not configured in test");
      },
    },
    session: {
      get: async () => null,
      set: async () => undefined,
      clear: async () => undefined,
    },
  };
}

describe("Verify tab", () => {
  beforeEach(() => setRuntime(null));
  afterEach(() => setRuntime(null));

  it("renders a verified outcome with signer + hash", async () => {
    const verify = vi
      .fn<[VerifyArgs], Promise<VerifyOutcome>>()
      .mockResolvedValue({
        status: "verified",
        signer_pubkey: "PubKeyAbCdEf123456",
        created_at: "2026-05-10T12:34:56Z",
        content_hash: "deadbeef".repeat(8),
        source: { platform: "chatgpt" },
        solana_tx: "local:abc",
      });
    setRuntime(makeRuntime(verify));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_aaa" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    await waitFor(() => expect(verify).toHaveBeenCalledTimes(1));
    const status = await screen.findByRole("status");
    expect(status.textContent ?? "").toMatch(/verified/i);
    expect(status.textContent ?? "").toMatch(/chatgpt/i);
  });

  it("renders a tampered outcome with reason", async () => {
    const verify = vi
      .fn<[VerifyArgs], Promise<VerifyOutcome>>()
      .mockResolvedValue({
        status: "tampered",
        reason: "signature mismatch",
      });
    setRuntime(makeRuntime(verify));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_bbb" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    const status = await screen.findByRole("status");
    expect(status.textContent ?? "").toMatch(/tampered/i);
    expect(status.textContent ?? "").toMatch(/signature mismatch/i);
  });

  it("renders a not-found outcome when the id is unknown", async () => {
    const verify = vi
      .fn<[VerifyArgs], Promise<VerifyOutcome>>()
      .mockResolvedValue({
        status: "not_found",
        attestation_id: "att_ccc",
      });
    setRuntime(makeRuntime(verify));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_ccc" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    const status = await screen.findByRole("status");
    expect(status.textContent ?? "").toMatch(/not found/i);
  });

  it("rejects an empty paste with an inline error", async () => {
    const verify = vi.fn();
    setRuntime(makeRuntime(verify as never));

    render(<Verify />);
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));
    await screen.findByText(/paste an attestation/i);
    expect(verify).not.toHaveBeenCalled();
  });
});

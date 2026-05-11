// Verify-tab tests for Cloud-tier. The Verify tab calls
// `runtime.verify(args)`; in Cloud-mode the runtime's `verify` impl
// will route through `cloudSync.verifyRemote` (T18 D2 — cloud =
// thin-client to the hosted `full` MCP). These tests install a
// runtime stub whose `verify` mirrors that delegation, then assert
// (a) `cloudSync.verifyRemote` was called with the attestation id and
// (b) each of the three terminal outcomes from the cloud client
// (`verified` / `tampered` / `not_found`) renders the correct popup
// panel.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Verify } from "../../../src/popup/tabs/Verify.js";
import {
  setRuntime,
  type CloudVerifyOutcome,
  type PopupRuntime,
  type VerifyArgs,
  type VerifyOutcome,
} from "../../../src/popup/runtime.js";

/**
 * Cloud-tier runtime stub. The `verify` seam delegates to
 * `cloudSync.verifyRemote`, projecting the cloud-shaped result
 * (signer / arweave_tx / solana_tx with `not_found` carrying no id)
 * into the popup's `VerifyOutcome` discriminated union. This mirrors
 * how the production `runtime.ts` will route Verify in Cloud mode
 * once the seam is wired (see decisions.md T18 Cloud-tier contract).
 *
 * Tests inject the `verifyRemote` mock to drive each outcome.
 */
function makeCloudRuntime(
  verifyRemote: PopupRuntime["cloudSync"]["verifyRemote"],
): PopupRuntime {
  return {
    loadIdentity: async () => null,
    loadStorageTier: async () => "cloud" as const,
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
    async verify(args: VerifyArgs): Promise<VerifyOutcome> {
      // Cloud-mode delegation: fall straight through to the cloud
      // client. The popup never passes `fileBytes` in this path —
      // file-drop verify is a documented backlog item — so we keep
      // the test stub focused on the id path.
      const id = args.attestationId?.trim();
      if (!id) return { status: "not_found" };
      const cloud = await this.cloudSync.verifyRemote(id);
      if (cloud === null) {
        // No JWT-bearing session — the popup renders "not found"
        // rather than crashing. (Production runtime should surface a
        // distinct sign-in prompt; tests here use the cloud mock to
        // assert delegation, so the null branch is unreachable in
        // these tests.)
        return { status: "not_found", attestation_id: id };
      }
      return projectCloudVerifyOutcome(cloud, id);
    },
    auth: {
      signIn: async () => {
        throw new Error("auth.signIn stub: not configured in test");
      },
      lookupExisting: async () => {
        throw new Error("auth.lookupExisting stub: not configured in test");
      },
      ensureExtensionJwt: async () => {
        throw new Error("auth.ensureExtensionJwt stub: not configured in test");
      },
      linkGoogle: async () => {
        throw new Error("auth.linkGoogle stub: not configured in test");
      },
    },
    session: {
      get: async () => null,
      set: async () => undefined,
      clear: async () => undefined,
      jwtExpiresAtMs: () => null,
    },
    cloudSync: {
      signRemote: async () => null,
      recallRemote: async () => null,
      verifyRemote,
    },
    keyEscrow: {
      wrap: async () => {
        throw new Error("keyEscrow.wrap stub: not configured in test");
      },
      unwrap: async () => {
        throw new Error("keyEscrow.unwrap stub: not configured in test");
      },
      upload: async () => {
        throw new Error("keyEscrow.upload stub: not configured in test");
      },
      fetch: async () => {
        throw new Error("keyEscrow.fetch stub: not configured in test");
      },
      delete: async () => {
        throw new Error("keyEscrow.delete stub: not configured in test");
      },
      rotate: async () => {
        throw new Error("keyEscrow.rotate stub: not configured in test");
      },
      hasBlob: async () => false,
    },
  };
}

/** Project the `CloudVerifyOutcome` (server wire shape) onto the
 *  popup's `VerifyOutcome`. Pure helper kept local to this test file
 *  so we don't need to add a production export for a not-yet-wired
 *  seam. */
function projectCloudVerifyOutcome(
  cloud: CloudVerifyOutcome,
  id: string,
): VerifyOutcome {
  if (cloud.status === "verified") {
    const out: VerifyOutcome = {
      status: "verified",
      signer_pubkey: cloud.signer,
      // The cloud surface doesn't return `created_at` or
      // `content_hash` on the verify path — the popup renderer
      // tolerates empty strings (it just truncates).
      created_at: "",
      content_hash: "",
    };
    if (cloud.solana_tx) out.solana_tx = cloud.solana_tx;
    if (cloud.arweave_tx) out.arweave_tx = cloud.arweave_tx;
    return out;
  }
  if (cloud.status === "tampered") {
    return { status: "tampered", reason: cloud.reason };
  }
  return { status: "not_found", attestation_id: id };
}

describe("Verify tab — Cloud tier", () => {
  beforeEach(() => setRuntime(null));
  afterEach(() => setRuntime(null));

  it("paste-id flow calls cloudSync.verifyRemote with the attestation id", async () => {
    const verifyRemote = vi
      .fn<[string], Promise<CloudVerifyOutcome | null>>()
      .mockResolvedValue({
        status: "verified",
        signer: "PubCloudSignerXyz",
        solana_tx: "SolRealTxAaa",
        arweave_tx: "ArRealTxBbb",
      });
    setRuntime(makeCloudRuntime(verifyRemote));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_cloud_one" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    await waitFor(() => expect(verifyRemote).toHaveBeenCalledTimes(1));
    expect(verifyRemote).toHaveBeenCalledWith("att_cloud_one");
  });

  it("verified path renders the verified panel with cloud signer + tx ids", async () => {
    const verifyRemote = vi
      .fn<[string], Promise<CloudVerifyOutcome | null>>()
      .mockResolvedValue({
        status: "verified",
        signer: "PubCloudSignerXyz",
        solana_tx: "SolRealTxAaa",
        arweave_tx: "ArRealTxBbb",
      });
    setRuntime(makeCloudRuntime(verifyRemote));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_cloud_one" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    const status = await screen.findByRole("status");
    expect(status.textContent ?? "").toMatch(/verified/i);
    // Cloud-side signer (not the `local:` placeholder).
    expect(status.textContent ?? "").toMatch(/PubClo/);
    // Cloud anchors render their truncated form.
    expect(status.textContent ?? "").toMatch(/SolRea/);
    expect(status.textContent ?? "").toMatch(/ArReal/);
  });

  it("tampered path renders the tampered panel with the cloud-side reason", async () => {
    const verifyRemote = vi
      .fn<[string], Promise<CloudVerifyOutcome | null>>()
      .mockResolvedValue({
        status: "tampered",
        signer: "PubCloudSignerXyz",
        reason: "blake3 hash mismatch",
      });
    setRuntime(makeCloudRuntime(verifyRemote));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_tamper" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    await waitFor(() => expect(verifyRemote).toHaveBeenCalledTimes(1));
    const status = await screen.findByRole("status");
    expect(status.textContent ?? "").toMatch(/tampered/i);
    expect(status.textContent ?? "").toMatch(/blake3 hash mismatch/);
  });

  it("not_found path renders the not-found panel", async () => {
    const verifyRemote = vi
      .fn<[string], Promise<CloudVerifyOutcome | null>>()
      .mockResolvedValue({ status: "not_found" });
    setRuntime(makeCloudRuntime(verifyRemote));

    render(<Verify />);
    fireEvent.change(screen.getByLabelText("Attestation id"), {
      target: { value: "att_unknown" },
    });
    fireEvent.click(screen.getByRole("button", { name: /verify/i }));

    await waitFor(() => expect(verifyRemote).toHaveBeenCalledTimes(1));
    const status = await screen.findByRole("status");
    expect(status.textContent ?? "").toMatch(/not found/i);
    // Verify.tsx renders the requested id under the not-found banner.
    expect(status.textContent ?? "").toMatch(/att_unknown/);
    // Not the file-drop placeholder.
    expect(status.textContent ?? "").not.toMatch(/file verification/i);
  });
});

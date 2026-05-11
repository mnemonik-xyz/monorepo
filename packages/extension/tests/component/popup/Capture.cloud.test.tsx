// Capture-tab tests for Cloud-tier. Covers the deferred-signing happy
// path (T18 spec D2) and the error taxonomy the runtime catches:
// `TransientSyncError` (alarm-drain retries), `PermanentSyncError`
// (row marked `sync_failed_permanent`), `ReauthRequiredError`
// (`mnemonik:re-auth-required` dispatched, popup falls back to
// onboarding).
//
// The runtime facade exposes two seams here: the high-level
// `signRemote(args)` wrapper (whose error-handling semantics mirror
// what `runtime.ts` does in production — try cloud push, branch on
// error class, dispatch the re-auth event, swallow Transient, re-throw
// Permanent) and the low-level `cloudSync.signRemote` (the actual
// HTTP call). The IDB enqueue/dequeue side-effects production performs
// are deliberately NOT modelled — these tests assert the popup ↔
// runtime contract, not the queue's persistence. The cloud-sync drain
// (tests/unit/background/cloud-sync.test.ts) already covers that.
//
// To preserve the contract that "popup invokes cloudSync.signRemote
// with the right payload" the test `signRemote` delegates to the
// mocked `cloudSync.signRemote` — that way both seams are observable.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Capture } from "../../../src/popup/tabs/Capture.js";
import {
  setRuntime,
  type PopupRuntime,
  type SignMemoryArgs,
  type SignMemoryResult,
} from "../../../src/popup/runtime.js";
import {
  PermanentSyncError,
  ReauthRequiredError,
  TransientSyncError,
} from "../../../src/runtime/sync/cloud-client.js";
import type { ChatAdapter } from "../../../src/runtime/chat/types.js";

const chatgptAdapter: ChatAdapter = {
  hostPattern: /^chatgpt\.com\//,
  platform: "chatgpt",
  supportsInsert: true,
  extractConversation: () => [],
  findInputBox: () => null,
  getChatId: () => null,
};

const SIGN_RESULT: SignMemoryResult = {
  attestation_id: "att_cloudbeef",
  // 64 hex chars — matches the canonical blake3 hex length used
  // everywhere else in the suite.
  content_hash: "cb".repeat(32),
  signer_pubkey: "PubCloud",
  // The local-pipeline always writes synthetic `local:` ids first; the
  // cloud-side `signRemote` overwrites them with real tx ids.
  solana_tx: "local:cloudbeef",
  arweave_tx: "local:cloudbeef",
  created_at: "2026-05-11T12:00:00Z",
};

const CLOUD_TX = {
  solana_tx: "SolRealTxAaa111",
  arweave_tx: "ArRealTxBbb222",
};

interface CloudRuntimeOverrides {
  signMemory?: PopupRuntime["signMemory"];
  cloudSignRemote?: PopupRuntime["cloudSync"]["signRemote"];
}

/**
 * Build a runtime stub whose top-level `signRemote` mirrors the
 * production semantics in `runtime.ts`: try `cloudSync.signRemote`
 * once, swallow `TransientSyncError` (alarm-drain retries),
 * dispatch `mnemonik:re-auth-required` on `ReauthRequiredError`,
 * re-throw `PermanentSyncError`. Tests can override the cloud seam
 * via `cloudSignRemote` to drive each branch.
 */
function makeCloudRuntime(overrides: CloudRuntimeOverrides = {}): PopupRuntime {
  const cloudSignRemote = overrides.cloudSignRemote ?? (async () => CLOUD_TX);
  const signMemory =
    overrides.signMemory ??
    (async (_args: SignMemoryArgs): Promise<SignMemoryResult> => SIGN_RESULT);
  return {
    loadIdentity: async () => null,
    loadStorageTier: async () => "cloud" as const,
    getActiveTabAdapter: async () => null,
    getActiveTabSelection: async () => "",
    getActiveTabConversation: async () => null,
    signMemory,
    async signRemote(args): Promise<void> {
      // Mirror runtime.ts behaviour so the popup observes the same
      // contract the production code path provides.
      try {
        await this.cloudSync.signRemote(args);
      } catch (err) {
        if (err instanceof ReauthRequiredError) {
          const g = globalThis as {
            dispatchEvent?: (e: Event) => boolean;
            CustomEvent?: typeof CustomEvent;
          };
          if (
            typeof g.dispatchEvent === "function" &&
            typeof g.CustomEvent === "function"
          ) {
            g.dispatchEvent(new g.CustomEvent("mnemonik:re-auth-required"));
          }
          return;
        }
        if (err instanceof PermanentSyncError) {
          throw err;
        }
        // TransientSyncError or unknown — leave queued, swallow.
      }
    },
    recall: async () => [],
    verify: async () => ({ status: "not_found" }),
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
      signRemote: cloudSignRemote,
      recallRemote: async () => null,
      verifyRemote: async () => null,
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

describe("Capture tab — Cloud tier", () => {
  beforeEach(() => {
    setRuntime(null);
  });
  afterEach(() => {
    setRuntime(null);
  });

  it("cloud_push_succeeds — deferred-signing happy path drives signRemote with full payload", async () => {
    // TDD anchor: cloud_push_succeeds. Sign with prefilled selection;
    // assert the cloud-side signRemote was called exactly once with
    // the canonical payload (content + tags + the SignMemoryResult
    // fields the popup forwards). Success toast renders with the
    // returned attestation_id.
    const cloudSignRemote = vi
      .fn<
        [SignMemoryArgs & SignMemoryResult],
        Promise<{ solana_tx: string; arweave_tx: string } | null>
      >()
      .mockResolvedValue(CLOUD_TX);
    setRuntime(makeCloudRuntime({ cloudSignRemote }));

    render(
      <Capture
        adapter={chatgptAdapter}
        prefilledSelection="cloud-bound memory"
      />,
    );

    fireEvent.change(screen.getByLabelText("Tags"), {
      target: { value: "alpha" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^sign$/i }));

    await waitFor(() => expect(cloudSignRemote).toHaveBeenCalledTimes(1));
    const payload = cloudSignRemote.mock.calls[0]?.[0];
    expect(payload?.content).toBe("cloud-bound memory");
    expect(payload?.tags).toContain("alpha");
    expect(payload?.tags).toContain("source:chatgpt");
    // The popup forwards every field the SignMemoryResult carries —
    // the cloud-client needs `signer_pubkey` to attach to the
    // sign-callback body, plus the attestation_id / content_hash so
    // the server can match the pending bundle.
    expect(payload?.attestation_id).toBe(SIGN_RESULT.attestation_id);
    expect(payload?.content_hash).toBe(SIGN_RESULT.content_hash);
    expect(payload?.signer_pubkey).toBe(SIGN_RESULT.signer_pubkey);

    const toast = await screen.findByRole("status");
    expect(toast.textContent ?? "").toMatch(/Signed/);
    expect(toast.textContent ?? "").toMatch(/att_cloudbeef/);
  });

  it("TransientSyncError leaves the local sign succeeded — no hard error toast", async () => {
    // The runtime swallows TransientSyncError so the alarm-drain can
    // retry on the next tick. Local sign already succeeded (the
    // signMemory call ran and persisted to IDB), so the popup must
    // continue to render the success toast rather than a hard error
    // that would scare the user off a recoverable upstream hiccup.
    const cloudSignRemote = vi
      .fn<
        [SignMemoryArgs & SignMemoryResult],
        Promise<{ solana_tx: string; arweave_tx: string } | null>
      >()
      .mockRejectedValue(new TransientSyncError("upstream 503", 503));
    const signMemory = vi
      .fn<[SignMemoryArgs], Promise<SignMemoryResult>>()
      .mockResolvedValue(SIGN_RESULT);
    setRuntime(makeCloudRuntime({ signMemory, cloudSignRemote }));

    render(<Capture adapter={null} prefilledSelection="will retry later" />);
    fireEvent.click(screen.getByRole("button", { name: /^sign$/i }));

    // Local pipeline ran (row was created in IDB) and cloud push was
    // attempted exactly once.
    await waitFor(() => expect(signMemory).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(cloudSignRemote).toHaveBeenCalledTimes(1));

    // Success toast renders — `signRemote` swallowed the transient
    // error per the runtime contract.
    const toast = await screen.findByRole("status");
    expect(toast.textContent ?? "").toMatch(/Signed/);
    expect(toast.textContent ?? "").toMatch(/att_cloudbeef/);
    // No error-banner copy.
    expect(screen.queryByText(/upstream 503/)).toBeNull();
    expect(screen.queryByText(/error/i)).toBeNull();
  });

  it("PermanentSyncError surfaces a hard error toast (row marked sync_failed_permanent by runtime)", async () => {
    // Mirror runtime.ts: PermanentSyncError is re-thrown. The popup
    // catches it and shows a user-facing error toast.
    const cloudSignRemote = vi
      .fn<
        [SignMemoryArgs & SignMemoryResult],
        Promise<{ solana_tx: string; arweave_tx: string } | null>
      >()
      .mockRejectedValue(
        new PermanentSyncError("sign-callback consumed/expired", 410),
      );
    setRuntime(makeCloudRuntime({ cloudSignRemote }));

    render(<Capture adapter={null} prefilledSelection="bad payload" />);
    fireEvent.click(screen.getByRole("button", { name: /^sign$/i }));

    await waitFor(() => expect(cloudSignRemote).toHaveBeenCalledTimes(1));

    const toast = await screen.findByRole("status");
    expect(toast.textContent ?? "").toMatch(/consumed|expired|sign-callback/i);
    // Success toast NOT rendered for a permanent failure — only the
    // error one. Capture.tsx clears `result` on entry so this is the
    // only role=status node.
    expect(toast.textContent ?? "").not.toMatch(/Signed →/);
  });

  it("ReauthRequiredError dispatches `mnemonik:re-auth-required` and does not block the local sign", async () => {
    // 401 from the cloud surface: the runtime swallows the error and
    // emits a global event so App.tsx flips into onboarding. The
    // local row stays queued for the next tick post re-auth.
    const cloudSignRemote = vi
      .fn<
        [SignMemoryArgs & SignMemoryResult],
        Promise<{ solana_tx: string; arweave_tx: string } | null>
      >()
      .mockRejectedValue(new ReauthRequiredError("sign-callback 401"));
    setRuntime(makeCloudRuntime({ cloudSignRemote }));

    const seen: string[] = [];
    const listener = (e: Event): void => {
      seen.push(e.type);
    };
    window.addEventListener("mnemonik:re-auth-required", listener);

    try {
      render(<Capture adapter={null} prefilledSelection="needs re-auth" />);
      fireEvent.click(screen.getByRole("button", { name: /^sign$/i }));

      await waitFor(() => expect(cloudSignRemote).toHaveBeenCalledTimes(1));
      // Wait for the runtime's signRemote to settle — it's awaited
      // by Capture, so by the time the toast renders the event has
      // fired.
      const toast = await screen.findByRole("status");
      expect(toast.textContent ?? "").toMatch(/Signed/);
      expect(seen).toContain("mnemonik:re-auth-required");
    } finally {
      window.removeEventListener("mnemonik:re-auth-required", listener);
    }
  });
});

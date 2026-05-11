// Recall-tab tests for the T18 Cloud-tier merge contract. The merge
// rule lives in `mergeRecallHits` (pure helper, already unit-tested in
// `Recall.test.tsx`); these component tests drive the full popup path:
//
//   - Local search (`runtime.recall`) and cloud search
//     (`cloudSync.recallRemote`) both fire in parallel.
//   - Results dedupe by `attestation_id`; the higher of the two scores
//     wins on conflict.
//   - Cloud-only entries carry non-`local:` tx ids (they were anchored
//     server-side); local-only entries keep their `local:` placeholders.
//   - Final list is sorted by score descending, truncated to limit.
//
// The TDD anchor `merge_dedupes_and_resorts` drives the canonical
// scenario: three local + three cloud rows with one overlap, asserting
// exact dedupe + score-winner + descending order.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Recall } from "../../../src/popup/tabs/Recall.js";
import {
  setRuntime,
  type CloudRecallHit,
  type PopupRuntime,
} from "../../../src/popup/runtime.js";
import type { ChatAdapter } from "../../../src/runtime/chat/types.js";
import type { SearchResult } from "../../../src/runtime/store/types.js";

const chatgptAdapter: ChatAdapter = {
  hostPattern: /^chatgpt\.com\//,
  platform: "chatgpt",
  supportsInsert: true,
  extractConversation: () => [],
  findInputBox: () => null,
  getChatId: () => null,
};

function localHit(over: Partial<SearchResult>): SearchResult {
  return {
    attestation_id: over.attestation_id ?? "att",
    content: over.content ?? "local content",
    content_hash: over.content_hash ?? "0".repeat(64),
    tags: over.tags ?? [],
    solana_tx: over.solana_tx ?? `local:${over.attestation_id ?? "x"}`,
    arweave_tx: over.arweave_tx ?? `local:${over.attestation_id ?? "x"}`,
    created_at: over.created_at ?? "2026-05-10T00:00:00Z",
    relevance_score: over.relevance_score ?? 0.5,
  };
}

function cloudHit(over: Partial<CloudRecallHit>): CloudRecallHit {
  return {
    attestation_id: over.attestation_id ?? "att",
    content: over.content ?? "cloud content",
    similarity: over.similarity ?? 0.5,
    ...(over.tags !== undefined ? { tags: over.tags } : {}),
    ...(over.signed_at !== undefined ? { signed_at: over.signed_at } : {}),
    ...(over.solana_tx !== undefined ? { solana_tx: over.solana_tx } : {}),
    ...(over.arweave_tx !== undefined ? { arweave_tx: over.arweave_tx } : {}),
  };
}

interface CloudMergeOverrides {
  localResults: SearchResult[];
  cloudResults: CloudRecallHit[] | null;
  cloudThrows?: unknown;
}

function makeCloudMergeRuntime(opts: CloudMergeOverrides): PopupRuntime {
  const recall = vi
    .fn<[string, number], Promise<SearchResult[]>>()
    .mockResolvedValue(opts.localResults);
  const recallRemote = vi
    .fn<[string, number], Promise<CloudRecallHit[] | null>>()
    .mockImplementation(async () => {
      if (opts.cloudThrows !== undefined) {
        throw opts.cloudThrows;
      }
      return opts.cloudResults;
    });
  const loadStorageTier = vi
    .fn<[], Promise<"local" | "cloud">>()
    .mockResolvedValue("cloud" as const);
  const runtime: PopupRuntime = {
    loadIdentity: async () => null,
    loadStorageTier,
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
    recall,
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
      signRemote: async () => null,
      recallRemote,
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
  // Stash the spies on the runtime for asserting calls outside the
  // closure. The cast keeps the runtime type clean for production
  // callers — only tests pull off these handles.
  (
    runtime as unknown as {
      __spies: {
        recall: typeof recall;
        recallRemote: typeof recallRemote;
        loadStorageTier: typeof loadStorageTier;
      };
    }
  ).__spies = {
    recall,
    recallRemote,
    loadStorageTier,
  };
  return runtime;
}

function spies(runtime: PopupRuntime): {
  recall: ReturnType<typeof vi.fn>;
  recallRemote: ReturnType<typeof vi.fn>;
  loadStorageTier: ReturnType<typeof vi.fn>;
} {
  return (
    runtime as unknown as {
      __spies: {
        recall: ReturnType<typeof vi.fn>;
        recallRemote: ReturnType<typeof vi.fn>;
        loadStorageTier: ReturnType<typeof vi.fn>;
      };
    }
  ).__spies;
}

describe("Recall tab — Cloud-tier merge contract", () => {
  beforeEach(() => {
    setRuntime(null);
  });
  afterEach(() => {
    setRuntime(null);
  });

  it("merge_dedupes_and_resorts — overlap deduped by id, best score wins, sorted desc", async () => {
    // TDD anchor: three local + three cloud, one id overlaps. The
    // duplicate row keeps the higher of the two scores; the final
    // rendered list contains exactly five distinct ids in score
    // descending order.
    const localResults: SearchResult[] = [
      localHit({
        attestation_id: "att_dup",
        relevance_score: 0.6,
        content: "shared local-copy",
      }),
      localHit({
        attestation_id: "att_local_only_hi",
        relevance_score: 0.85,
        content: "local-only top",
      }),
      localHit({
        attestation_id: "att_local_only_mid",
        relevance_score: 0.4,
        content: "local-only mid",
      }),
    ];
    const cloudResults: CloudRecallHit[] = [
      // Same id as the local duplicate, but with a higher cloud-side
      // similarity score — the merge keeps the cloud score.
      cloudHit({
        attestation_id: "att_dup",
        similarity: 0.92,
        content: "shared cloud-copy",
        solana_tx: "SolReal_dup",
        arweave_tx: "ArReal_dup",
      }),
      cloudHit({
        attestation_id: "att_cloud_only_hi",
        similarity: 0.78,
        content: "cloud-only top",
        solana_tx: "SolReal_co1",
        arweave_tx: "ArReal_co1",
      }),
      cloudHit({
        attestation_id: "att_cloud_only_lo",
        similarity: 0.2,
        content: "cloud-only bottom",
        solana_tx: "SolReal_co2",
        arweave_tx: "ArReal_co2",
      }),
    ];
    const runtime = makeCloudMergeRuntime({ localResults, cloudResults });
    setRuntime(runtime);

    render(<Recall adapter={chatgptAdapter} />);
    // Wait for `loadStorageTier` to settle so the click runs in cloud
    // mode rather than the default local fallback. The Recall tab
    // reads the tier exactly once on mount; we await that call AND
    // a microtask so the React state commit propagates.
    const { recall, recallRemote, loadStorageTier } = spies(runtime);
    await waitFor(() => expect(loadStorageTier).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));

    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "test query" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));
    await waitFor(() => expect(recall).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(recallRemote).toHaveBeenCalledWith("test query", 5),
    );

    // The merged list — five distinct ids, sorted desc.
    const list = await screen.findByLabelText("Recall results");
    await waitFor(() => {
      expect(list.querySelectorAll("li").length).toBe(5);
    });
    const items = Array.from(list.querySelectorAll("li"));

    // Score-descending order check by parsing the first cell of each
    // <li> (the score badge formatted as `.toFixed(3)`).
    const scores = items.map((li) => {
      const badge = li.querySelector("span");
      return Number(badge?.textContent ?? "0");
    });
    const sorted = [...scores].sort((a, b) => b - a);
    expect(scores).toEqual(sorted);

    // The merged duplicate carries the higher cloud-side score (0.92,
    // not 0.6). Find via the formatted score text.
    expect(list.textContent ?? "").toMatch(/0\.920/);
    // The lower local-side score for the duplicate must NOT appear —
    // dedupe removed it.
    expect(list.textContent ?? "").not.toMatch(/0\.600/);
    // Each of the four non-duplicate scores renders.
    expect(list.textContent ?? "").toMatch(/0\.850/);
    expect(list.textContent ?? "").toMatch(/0\.780/);
    expect(list.textContent ?? "").toMatch(/0\.400/);
    expect(list.textContent ?? "").toMatch(/0\.200/);
  });

  it("cloud-only entries carry non-`local:` tx ids; local-only keep `local:` placeholders", async () => {
    // The merge surfaces tx-id provenance on each row: cloud-only
    // rows reach the renderer with real Solana / Arweave ids the
    // server returned, while local-only rows still carry the synthetic
    // `local:` prefix until the alarm-drain anchors them. We can't
    // read tx ids straight off the DOM (the popup renders only the
    // truncated `attestation_id`), so we re-run the same merge through
    // a fresh `mergeRecallHits` import to assert the projected shape.
    const localResults: SearchResult[] = [
      localHit({
        attestation_id: "att_local_only",
        relevance_score: 0.8,
        solana_tx: "local:LocalOnly",
        arweave_tx: "local:LocalOnly",
      }),
    ];
    const cloudResults: CloudRecallHit[] = [
      cloudHit({
        attestation_id: "att_cloud_only",
        similarity: 0.7,
        content: "cloud-only payload",
        solana_tx: "SolRealCloudOnly",
        arweave_tx: "ArRealCloudOnly",
      }),
    ];
    const runtime = makeCloudMergeRuntime({
      localResults,
      cloudResults,
    });
    setRuntime(runtime);

    render(<Recall adapter={chatgptAdapter} />);
    const { loadStorageTier } = spies(runtime);
    await waitFor(() => expect(loadStorageTier).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    const list = await screen.findByLabelText("Recall results");
    await waitFor(() => {
      expect(list.querySelectorAll("li").length).toBe(2);
    });

    // The cloud-only content rendered.
    expect(list.textContent ?? "").toMatch(/cloud-only payload/);
    // The local-only row rendered with its synthetic `local:`
    // provenance kept (re-derive via the pure helper to assert tx
    // shape, since the renderer doesn't expose tx ids inline).
    const { mergeRecallHits } =
      await import("../../../src/popup/tabs/Recall.js");
    const merged = mergeRecallHits(localResults, cloudResults, 5);
    const localOnly = merged.find((r) => r.attestation_id === "att_local_only");
    const cloudOnly = merged.find((r) => r.attestation_id === "att_cloud_only");
    expect(localOnly?.solana_tx).toBe("local:LocalOnly");
    expect(localOnly?.arweave_tx).toBe("local:LocalOnly");
    expect(cloudOnly?.solana_tx).toBe("SolRealCloudOnly");
    expect(cloudOnly?.arweave_tx).toBe("ArRealCloudOnly");
  });

  it("cloud failure is non-blocking — local results still render, no hard error banner", async () => {
    // Offline-first guarantee: a `recallRemote` rejection must NOT
    // block the local-results path. The popup swallows the error
    // (Recall.tsx logs to `console.warn`) and the rendered list
    // matches what local-only would render.
    const localResults: SearchResult[] = [
      localHit({
        attestation_id: "att_l1",
        relevance_score: 0.9,
        content: "local A",
      }),
      localHit({
        attestation_id: "att_l2",
        relevance_score: 0.55,
        content: "local B",
      }),
    ];
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      const runtime = makeCloudMergeRuntime({
        localResults,
        cloudResults: null,
        cloudThrows: new Error("503 upstream"),
      });
      setRuntime(runtime);

      render(<Recall adapter={chatgptAdapter} />);
      const { loadStorageTier } = spies(runtime);
      await waitFor(() => expect(loadStorageTier).toHaveBeenCalled());
      await new Promise((r) => setTimeout(r, 0));
      fireEvent.change(screen.getByLabelText("Recall query"), {
        target: { value: "anything" },
      });
      fireEvent.click(screen.getByRole("button", { name: /find/i }));

      // Local results render unaffected.
      const list = await screen.findByLabelText("Recall results");
      await waitFor(() => {
        expect(list.querySelectorAll("li").length).toBe(2);
      });
      expect(list.textContent ?? "").toMatch(/local A/);
      expect(list.textContent ?? "").toMatch(/local B/);

      // No hard error banner in the popup body (the inline error
      // <div> would render the string verbatim if it had bubbled up).
      expect(screen.queryByText(/503 upstream/)).toBeNull();

      // BUG2-07: the popup surfaces cloud failure as a non-fatal
      // banner above the results list ("Cloud search unavailable —
      // showing local results only"). Previously a console.warn was
      // the only signal; users had no UI indication anything failed.
      await waitFor(() => {
        expect(
          screen.getByText(/Cloud search unavailable/i),
        ).toBeInTheDocument();
      });
    } finally {
      warnSpy.mockRestore();
    }
  });
});

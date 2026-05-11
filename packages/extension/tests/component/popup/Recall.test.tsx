// Recall-tab tests. Drives the T11 TDD anchor
// `insert_into_chat_disabled_when_no_input_box`: an adapter that ships
// `findInputBox: () => null` produces a disabled "Insert into chat"
// button with an explanatory tooltip. Also covers the basic "results
// render with scores" path the Wave-4 spec calls out.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Recall, mergeRecallHits } from "../../../src/popup/tabs/Recall.js";
import {
  setRuntime,
  type CloudRecallHit,
  type PopupRuntime,
} from "../../../src/popup/runtime.js";
import type { ChatAdapter } from "../../../src/runtime/chat/types.js";
import type { SearchResult } from "../../../src/runtime/store/types.js";

function makeRuntime(overrides: Partial<PopupRuntime> = {}): PopupRuntime {
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
    verify: async () => ({ status: "not_found" }),
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
    cloudSync: {
      // T18 stub: tests default to Local tier so these never fire.
      // Cloud-tier specific tests override `cloudSync.recallRemote`.
      signRemote: async () => null,
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
    ...overrides,
  };
}

// Adapter that opts OUT of insert support per T08 / T09 convention.
const claudeLikeAdapter: ChatAdapter = {
  hostPattern: /^claude\.ai\//,
  platform: "claude",
  supportsInsert: false,
  extractConversation: () => [],
  findInputBox: () => null,
  getChatId: () => null,
};

// Adapter that opts IN — declares insert support via the explicit flag.
const chatgptLikeAdapter: ChatAdapter = {
  hostPattern: /^chatgpt\.com\//,
  platform: "chatgpt",
  supportsInsert: true,
  extractConversation: () => [],
  findInputBox: (doc: Document) => {
    const ta = doc.querySelector('textarea[data-id="root"]');
    return ta instanceof HTMLElement ? ta : null;
  },
  getChatId: () => null,
};

const results: SearchResult[] = [
  {
    attestation_id: "att_aaa111",
    content: "First match — keep going",
    content_hash: "a".repeat(64),
    tags: ["source:chatgpt", "alpha"],
    solana_tx: "local:aaa111",
    arweave_tx: "local:aaa111",
    created_at: "2026-05-10T00:00:00Z",
    relevance_score: 0.92,
  },
  {
    attestation_id: "att_bbb222",
    content: "Second match",
    content_hash: "b".repeat(64),
    tags: ["source:claude", "beta"],
    solana_tx: "local:bbb222",
    arweave_tx: "local:bbb222",
    created_at: "2026-05-10T01:00:00Z",
    relevance_score: 0.71,
  },
];

describe("Recall tab", () => {
  beforeEach(() => {
    setRuntime(null);
  });
  afterEach(() => {
    setRuntime(null);
  });

  it("insert_into_chat_disabled_when_no_input_box", async () => {
    // TDD anchor #2: an adapter whose `findInputBox` returns null
    // unconditionally renders the Insert button as disabled.
    const recall = vi.fn().mockResolvedValue(results);
    setRuntime(makeRuntime({ recall }));

    render(<Recall adapter={claudeLikeAdapter} />);

    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "hello" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    await waitFor(() => expect(recall).toHaveBeenCalled());
    const insertButtons = await screen.findAllByRole("button", {
      name: /insert into chat/i,
    });
    expect(insertButtons.length).toBeGreaterThan(0);
    for (const b of insertButtons) {
      expect(b).toBeDisabled();
    }
    expect(insertButtons[0]?.getAttribute("title") ?? "").toMatch(
      /does not support/i,
    );
  });

  it("renders results with scores + platform pills", async () => {
    const recall = vi.fn().mockResolvedValue(results);
    setRuntime(makeRuntime({ recall }));

    render(<Recall adapter={chatgptLikeAdapter} />);
    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "match" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    await screen.findByText(/0.920/);
    await screen.findByText(/0.710/);
    expect(screen.getAllByText(/chatgpt/i).length).toBeGreaterThan(0);
  });

  it("enables Insert when the adapter supports findInputBox", async () => {
    const recall = vi.fn().mockResolvedValue(results);
    setRuntime(makeRuntime({ recall }));

    render(<Recall adapter={chatgptLikeAdapter} />);
    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    const insertButtons = await screen.findAllByRole("button", {
      name: /insert into chat/i,
    });
    expect(insertButtons.length).toBeGreaterThan(0);
    for (const b of insertButtons) {
      expect(b).not.toBeDisabled();
    }
  });

  it("Cloud-tier: merges local IndexedDB hits with mnemonic_recall hits, dedupes by attestation_id", async () => {
    // T18 Cloud-tier Recall merge: storage tier is "cloud", local
    // returns one of two hits, cloud returns the same id (higher
    // score) plus a brand-new id. The rendered list MUST contain
    // both ids in score order, with the duplicate counted once.
    const recall = vi.fn().mockResolvedValue([results[0]]); // local: 0.92
    const recallRemote = vi.fn().mockResolvedValue([
      // Same id as local[0] but with a higher cloud-side score — the
      // merge keeps the higher of the two.
      {
        attestation_id: "att_aaa111",
        content: "First match — keep going",
        similarity: 0.97,
        tags: ["source:chatgpt", "alpha"],
        signed_at: "2026-05-10T00:00:00Z",
        solana_tx: "RealSolTx1",
        arweave_tx: "RealArTx1",
      },
      // Brand-new id that only exists on the cloud side.
      {
        attestation_id: "att_ccc333",
        content: "Cloud-only memory",
        similarity: 0.55,
        tags: ["source:gemini"],
        signed_at: "2026-05-11T00:00:00Z",
      },
    ]);
    const loadStorageTier = vi.fn().mockResolvedValue("cloud" as const);
    setRuntime(
      makeRuntime({
        recall,
        loadStorageTier,
        cloudSync: {
          signRemote: async () => null,
          recallRemote,
          verifyRemote: async () => null,
        },
      }),
    );

    render(<Recall adapter={chatgptLikeAdapter} />);
    // Wait for the tier-resolving useEffect to settle before issuing
    // the search — otherwise the click can race the async state
    // commit and the component stays in "local" tier for this click.
    await waitFor(() => expect(loadStorageTier).toHaveBeenCalled());
    // One more microtask for the React state commit to propagate.
    await new Promise((r) => setTimeout(r, 0));

    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "match" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    // Both calls fire — local first, cloud in parallel.
    await waitFor(() => expect(recall).toHaveBeenCalled());
    await waitFor(() => expect(recallRemote).toHaveBeenCalledWith("match", 5));

    // The merged list contains exactly 2 distinct rows (att_aaa111
    // and att_ccc333 — the duplicate id was deduped).
    // Higher cloud score wins for the duplicate row.
    await screen.findByText(/0.970/);
    // Cloud-only entry rendered.
    await screen.findByText(/Cloud-only memory/);
    const list = await screen.findByLabelText("Recall results");
    const items = list.querySelectorAll("li");
    expect(items.length).toBe(2);
  });

  it("Cloud-tier: cloud failure does not block local results (offline-first)", async () => {
    // The tech-spec's offline-first guarantee: a cloud-side failure
    // (5xx, network drop, malformed JSON) must NEVER block the local
    // results path. The component swallows the cloud error and
    // renders local hits as if Cloud tier were Local.
    const recall = vi.fn().mockResolvedValue(results);
    const recallRemote = vi.fn().mockRejectedValue(new Error("503 upstream"));
    setRuntime(
      makeRuntime({
        recall,
        loadStorageTier: async () => "cloud" as const,
        cloudSync: {
          signRemote: async () => null,
          recallRemote,
          verifyRemote: async () => null,
        },
      }),
    );

    render(<Recall adapter={chatgptLikeAdapter} />);
    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "match" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    // Local results still render unaffected.
    await screen.findByText(/0.920/);
    await screen.findByText(/0.710/);
    // The component swallowed the cloud error — no error banner.
    expect(screen.queryByText(/503/)).toBeNull();
  });

  it("Local-tier: skips cloudSync.recallRemote entirely (offline-first default)", async () => {
    const recall = vi.fn().mockResolvedValue(results);
    const recallRemote = vi.fn();
    setRuntime(
      makeRuntime({
        recall,
        // Default tier is "local" but be explicit so a regression
        // that flips the default to "cloud" trips this.
        loadStorageTier: async () => "local" as const,
        cloudSync: {
          signRemote: async () => null,
          recallRemote,
          verifyRemote: async () => null,
        },
      }),
    );

    render(<Recall adapter={chatgptLikeAdapter} />);
    fireEvent.change(screen.getByLabelText("Recall query"), {
      target: { value: "x" },
    });
    fireEvent.click(screen.getByRole("button", { name: /find/i }));

    await waitFor(() => expect(recall).toHaveBeenCalled());
    // Cloud recall must NOT have been called.
    expect(recallRemote).not.toHaveBeenCalled();
  });
});

// ────────────────────────────────────────────────────────────────────────
// mergeRecallHits — pure-function unit tests (code-reviewer C-5)
// ────────────────────────────────────────────────────────────────────────

function localHit(over: Partial<SearchResult>): SearchResult {
  return {
    attestation_id: over.attestation_id ?? "att",
    content: over.content ?? "local content",
    content_hash: over.content_hash ?? "0".repeat(64),
    tags: over.tags ?? [],
    solana_tx: over.solana_tx ?? "local:x",
    arweave_tx: over.arweave_tx ?? "local:x",
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

describe("mergeRecallHits (pure)", () => {
  it("returns local slice unchanged when cloud is null", () => {
    const local = [
      localHit({ attestation_id: "a", relevance_score: 0.9 }),
      localHit({ attestation_id: "b", relevance_score: 0.7 }),
    ];
    expect(mergeRecallHits(local, null, 5)).toEqual(local);
  });

  it("returns local slice unchanged when cloud is empty", () => {
    const local = [localHit({ attestation_id: "a", relevance_score: 0.9 })];
    expect(mergeRecallHits(local, [], 5)).toEqual(local);
  });

  it("truncates the result list to limit", () => {
    const local = [
      localHit({ attestation_id: "a", relevance_score: 0.9 }),
      localHit({ attestation_id: "b", relevance_score: 0.8 }),
      localHit({ attestation_id: "c", relevance_score: 0.7 }),
    ];
    const out = mergeRecallHits(local, null, 2);
    expect(out).toHaveLength(2);
    expect(out.map((r) => r.attestation_id)).toEqual(["a", "b"]);
  });

  it("dedupes same-id rows, keeping the higher score when cloud wins", () => {
    const local = [localHit({ attestation_id: "a", relevance_score: 0.5 })];
    const cloud = [cloudHit({ attestation_id: "a", similarity: 0.9 })];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out).toHaveLength(1);
    expect(out[0]?.relevance_score).toBe(0.9);
  });

  it("dedupes same-id rows, keeping the higher score when local wins", () => {
    const local = [localHit({ attestation_id: "a", relevance_score: 0.9 })];
    const cloud = [cloudHit({ attestation_id: "a", similarity: 0.4 })];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out).toHaveLength(1);
    expect(out[0]?.relevance_score).toBe(0.9);
  });

  it("folds cloud tx ids over synthetic local: ids for same-id rows", () => {
    const local = [
      localHit({
        attestation_id: "a",
        relevance_score: 0.5,
        solana_tx: "local:a",
        arweave_tx: "local:a",
      }),
    ];
    const cloud = [
      cloudHit({
        attestation_id: "a",
        similarity: 0.6,
        solana_tx: "RealSol",
        arweave_tx: "RealAr",
      }),
    ];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out[0]?.solana_tx).toBe("RealSol");
    expect(out[0]?.arweave_tx).toBe("RealAr");
  });

  it("keeps real local tx ids when cloud has different real ones (Phase 1 — see decisions.md backlog)", () => {
    // Local row has been drained at least once already (real tx ids).
    // A future cloud-side rotation would return different ids; the
    // current merge rule keeps the local value. This documents the
    // Phase 1 behaviour; T18-C-05 / decisions.md tracks Phase 2.
    const local = [
      localHit({
        attestation_id: "a",
        relevance_score: 0.5,
        solana_tx: "OldRealSol",
        arweave_tx: "OldRealAr",
      }),
    ];
    const cloud = [
      cloudHit({
        attestation_id: "a",
        similarity: 0.6,
        solana_tx: "NewRealSol",
        arweave_tx: "NewRealAr",
      }),
    ];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out[0]?.solana_tx).toBe("OldRealSol");
    expect(out[0]?.arweave_tx).toBe("OldRealAr");
  });

  it("does not fold cloud tx ids when cloud omits them", () => {
    // Local row has synthetic ids; cloud hit has no tx fields. Merge
    // must keep the local placeholders rather than write an empty
    // string (which would break the popup's `local:` heuristic).
    const local = [
      localHit({
        attestation_id: "a",
        relevance_score: 0.5,
        solana_tx: "local:a",
        arweave_tx: "local:a",
      }),
    ];
    const cloud = [cloudHit({ attestation_id: "a", similarity: 0.6 })];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out[0]?.solana_tx).toBe("local:a");
    expect(out[0]?.arweave_tx).toBe("local:a");
  });

  it("includes cloud-only entries projected into SearchResult shape", () => {
    const cloud = [
      cloudHit({
        attestation_id: "cloud-only",
        content: "only on server",
        similarity: 0.7,
        tags: ["t1"],
        signed_at: "2026-05-11T00:00:00Z",
        solana_tx: "Sol",
        arweave_tx: "Ar",
      }),
    ];
    const out = mergeRecallHits([], cloud, 5);
    expect(out).toHaveLength(1);
    expect(out[0]).toEqual({
      attestation_id: "cloud-only",
      content: "only on server",
      content_hash: "",
      tags: ["t1"],
      solana_tx: "Sol",
      arweave_tx: "Ar",
      created_at: "2026-05-11T00:00:00Z",
      relevance_score: 0.7,
    });
  });

  it("re-sorts merged rows by score descending", () => {
    const local = [
      localHit({ attestation_id: "low", relevance_score: 0.2 }),
      localHit({ attestation_id: "high", relevance_score: 0.9 }),
    ];
    const cloud = [cloudHit({ attestation_id: "mid", similarity: 0.5 })];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out.map((r) => r.attestation_id)).toEqual(["high", "mid", "low"]);
  });

  it("drops cloud hits with empty attestation_id (defensive against malformed server response)", () => {
    const local = [localHit({ attestation_id: "a", relevance_score: 0.9 })];
    const cloud = [cloudHit({ attestation_id: "", similarity: 0.5 })];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out).toHaveLength(1);
    expect(out[0]?.attestation_id).toBe("a");
  });

  it("dedupes by content when local and cloud ids differ (BUG2-03)", () => {
    // Local row has `att_<blake3-prefix>` id; cloud-issued UUID is
    // different. Both carry the same content. Merge MUST collapse
    // them to a single result rather than rendering two cards.
    const local = [
      localHit({
        attestation_id: "att_deadbeef00112233",
        relevance_score: 0.4,
        content: "shared content",
        solana_tx: "local:deadbeef",
        arweave_tx: "local:deadbeef",
      }),
    ];
    const cloud = [
      cloudHit({
        attestation_id: "att_server-uuid-xyz",
        similarity: 0.8,
        content: "shared content",
        solana_tx: "SolReal",
        arweave_tx: "ArReal",
      }),
    ];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out).toHaveLength(1);
    // The local id stays canonical (cloud-sync drain has not reflowed
    // the server id yet) but cloud-side tx ids fold in over `local:`.
    expect(out[0]?.attestation_id).toBe("att_deadbeef00112233");
    expect(out[0]?.relevance_score).toBe(0.8);
    expect(out[0]?.solana_tx).toBe("SolReal");
    expect(out[0]?.arweave_tx).toBe("ArReal");
  });
});

// Recall-tab tests. Drives the T11 TDD anchor
// `insert_into_chat_disabled_when_no_input_box`: an adapter that ships
// `findInputBox: () => null` produces a disabled "Insert into chat"
// button with an explanatory tooltip. Also covers the basic "results
// render with scores" path the Wave-4 spec calls out.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Recall } from "../../../src/popup/tabs/Recall.js";
import { setRuntime, type PopupRuntime } from "../../../src/popup/runtime.js";
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
    ...overrides,
  };
}

// Adapter that opts OUT of insert support per T08 / T09 convention.
const claudeLikeAdapter: ChatAdapter = {
  hostPattern: /^claude\.ai\//,
  platform: "claude",
  extractConversation: () => [],
  findInputBox: () => null,
  getChatId: () => null,
};

// Adapter that opts IN — the body has more than a bare `return null`.
const chatgptLikeAdapter: ChatAdapter = {
  hostPattern: /^chatgpt\.com\//,
  platform: "chatgpt",
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
      /does not support/i
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
});

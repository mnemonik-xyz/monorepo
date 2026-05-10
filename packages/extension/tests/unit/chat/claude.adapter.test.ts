import { JSDOM } from "jsdom";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { claudeAdapter } from "../../../src/runtime/chat/adapters/claude.adapter.js";
import {
  __setAdaptersForTesting,
  listAdapters,
  selectAdapter,
} from "../../../src/runtime/chat/registry.js";
import {
  serializeMarkdown,
  serializePayload,
} from "../../../src/runtime/chat/serializer.js";
import type { ChatMeta } from "../../../src/runtime/chat/types.js";

const FIXTURE_DIR = resolve(__dirname, "../../fixtures/claude");

function loadFixture(name: "empty" | "code" | "long"): Document {
  const html = readFileSync(resolve(FIXTURE_DIR, `${name}.html`), "utf8");
  return new JSDOM(html, { url: "https://claude.ai/chat/test" }).window
    .document;
}

afterEach(() => {
  // Restore the post-import state — the adapter self-registers on
  // import, so subsequent tests still see it.
  __setAdaptersForTesting([claudeAdapter]);
});

describe("claude.adapter · self-registration", () => {
  it("registers itself on import (idempotent)", () => {
    expect(listAdapters()).toContain(claudeAdapter);
  });

  it("matches https://claude.ai/chat/<uuid> via selectAdapter", () => {
    expect(
      selectAdapter("https://claude.ai/chat/123e4567-e89b-12d3-a456-426614174000"),
    ).toBe(claudeAdapter);
  });
});

describe("claude.adapter · extractConversation", () => {
  // T08 TDD anchor (D13): drives correct turn segmentation under
  // Claude's nested DOM. ≥6 alternating user/assistant turns from the
  // hand-crafted long fixture (refresh from a live capture before
  // merge — see decisions.md).
  it("extracts_multi_turn_conversation", () => {
    const doc = loadFixture("long");
    const turns = claudeAdapter.extractConversation(doc);
    expect(turns.length).toBeGreaterThanOrEqual(6);

    const roles = turns.map((t) => t.role);
    // Alternation: every consecutive pair must differ.
    for (let i = 1; i < roles.length; i += 1) {
      expect(roles[i]).not.toBe(roles[i - 1]);
    }
    expect(new Set(roles)).toEqual(new Set(["user", "assistant"]));
    expect(roles[0]).toBe("user");
  });

  // T08 TDD anchor (D13): drives parity with the ChatGPT adapter's
  // code-block extraction — `<pre><code class="language-rust">` must
  // round-trip into a fenced ```rust block.
  it("code_blocks_preserved_with_language", () => {
    const doc = loadFixture("code");
    const turns = claudeAdapter.extractConversation(doc);

    const assistant = turns.find((t) => t.role === "assistant");
    expect(assistant).toBeDefined();
    expect(assistant!.content).toContain("```rust");
    expect(assistant!.content).toContain(
      "fn reverse(s: &str) -> String {",
    );
    expect(assistant!.content).toMatch(/```rust\n[\s\S]+?\n```/);
  });

  it("returns an empty array for an empty conversation", () => {
    const doc = loadFixture("empty");
    expect(claudeAdapter.extractConversation(doc)).toEqual([]);
  });

  it("produces a markdown snapshot via serializeMarkdown that survives the fence", () => {
    const doc = loadFixture("code");
    const turns = claudeAdapter.extractConversation(doc);
    const meta: ChatMeta = {
      platform: "claude",
      chatId: "abc",
      url: "https://claude.ai/chat/abc",
    };
    const md = serializeMarkdown(turns, meta);
    expect(md).toContain("### user");
    expect(md).toContain("### assistant");
    expect(md).toContain("```rust");
    expect(md).toContain("How do I reverse a string in Rust?");
  });

  it("emits source/chat tags in the canonical order via serializePayload", () => {
    const doc = loadFixture("code");
    const turns = claudeAdapter.extractConversation(doc);
    const { tags, source_meta } = serializePayload(turns, {
      platform: "claude",
      chatId: "abc",
    });
    expect(tags).toEqual(["source:claude", "chat:abc"]);
    expect(source_meta).toEqual({ platform: "claude", chat_id: "abc" });
  });
});

describe("claude.adapter · getChatId", () => {
  function loc(pathname: string): Location {
    return { pathname } as unknown as Location;
  }

  it("parses the uuid from /chat/<uuid>", () => {
    const doc = loadFixture("empty");
    expect(
      claudeAdapter.getChatId(
        doc,
        loc("/chat/123e4567-e89b-12d3-a456-426614174000"),
      ),
    ).toBe("123e4567-e89b-12d3-a456-426614174000");
  });

  it("returns null on /new and unrelated paths", () => {
    const doc = loadFixture("empty");
    expect(claudeAdapter.getChatId(doc, loc("/new"))).toBeNull();
    expect(claudeAdapter.getChatId(doc, loc("/"))).toBeNull();
    expect(claudeAdapter.getChatId(doc, loc("/projects/foo"))).toBeNull();
  });

  it("rejects paths whose suffix is not a uuid", () => {
    const doc = loadFixture("empty");
    expect(
      claudeAdapter.getChatId(doc, loc("/chat/not-a-uuid")),
    ).toBeNull();
  });
});

describe("claude.adapter · findInputBox / hostPattern", () => {
  it("returns null in Phase 1 (read-only)", () => {
    const doc = loadFixture("empty");
    expect(claudeAdapter.findInputBox(doc)).toBeNull();
  });

  it("hostPattern matches claude.ai but not lookalikes", () => {
    expect(claudeAdapter.hostPattern.test("claude.ai/chat/abc")).toBe(true);
    expect(claudeAdapter.hostPattern.test("notclaude.ai/chat/abc")).toBe(
      false,
    );
    expect(claudeAdapter.hostPattern.test("claude.ai.evil.com/")).toBe(false);
  });
});

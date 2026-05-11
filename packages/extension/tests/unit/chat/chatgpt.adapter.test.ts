import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { JSDOM } from "jsdom";
import { afterEach, describe, expect, it } from "vitest";

import { chatgptAdapter } from "../../../src/runtime/chat/adapters/chatgpt.adapter.js";
import {
  __setAdaptersForTesting,
  selectAdapter,
} from "../../../src/runtime/chat/registry.js";
import { serializeMarkdown } from "../../../src/runtime/chat/serializer.js";
import type { ChatTurn } from "../../../src/runtime/chat/types.js";

// `chatgpt.adapter.ts` self-registers via `registerAdapter` on import,
// so wipe the registry between tests to keep `selectAdapter` cases
// deterministic. Other tests that depend on a stable adapter list use
// `__setAdaptersForTesting` directly.
afterEach(() => {
  __setAdaptersForTesting([]);
});

const FIXTURE_DIR = resolve(__dirname, "../../fixtures/chatgpt");
function loadFixture(name: "empty" | "code" | "long"): Document {
  const html = readFileSync(resolve(FIXTURE_DIR, `${name}.html`), "utf8");
  return new JSDOM(html).window.document;
}
function loadFixtureWithLocation(
  name: "empty" | "code" | "long",
  url: string,
): { doc: Document; location: Location } {
  const html = readFileSync(resolve(FIXTURE_DIR, `${name}.html`), "utf8");
  const dom = new JSDOM(html, { url });
  return { doc: dom.window.document, location: dom.window.location };
}

describe("chatgpt adapter · contract", () => {
  it("declares hostPattern and platform identifying chatgpt.com", () => {
    expect(chatgptAdapter.platform).toBe("chatgpt");
    expect(chatgptAdapter.hostPattern.test("chatgpt.com/")).toBe(true);
    expect(chatgptAdapter.hostPattern.test("chatgpt.com/c/abc")).toBe(true);
    expect(chatgptAdapter.hostPattern.test("claude.ai/chat/abc")).toBe(false);
  });

  it("self-registers so selectAdapter resolves chatgpt.com URLs to it", () => {
    // Importing the adapter module triggers registerAdapter as a side
    // effect — the popup / content script bootstrap relies on this.
    // (The afterEach wipe means we re-seed manually here.)
    __setAdaptersForTesting([chatgptAdapter]);
    expect(selectAdapter("https://chatgpt.com/c/abc123")).toBe(chatgptAdapter);
    expect(selectAdapter("https://example.com/")).toBeNull();
  });
});

describe("chatgpt adapter · extractConversation", () => {
  // T07 D13 anchor — code block with a `language-*` hint must round-trip
  // through `domNodeToMarkdown` as a fenced ```python block. Drives D8
  // code-fidelity guarantee.
  it("extracts_code_block_with_language", () => {
    const doc = loadFixture("code");
    const turns = chatgptAdapter.extractConversation(doc);

    const assistant = turns.find(
      (t) => t.role === "assistant" && t.content.includes("sorted"),
    );
    expect(assistant).toBeDefined();
    expect(assistant?.content).toContain("```python\ndef sort_list(xs):\n    return sorted(xs)\n```");

    const rust = turns.find(
      (t) => t.role === "assistant" && t.content.includes("Vec::sort"),
    );
    expect(rust?.content).toMatch(/```rust\nfn main\(\) \{/);
  });

  // T07 D13 anchor — role attribute must be the source of truth so a
  // captured chat reconstructs faithfully.
  it("role_inferred_from_data_attr", () => {
    const doc = loadFixture("code");
    const turns = chatgptAdapter.extractConversation(doc);
    const roles = turns.map((t) => t.role);
    // code.html alternates user/assistant/user/assistant.
    expect(roles).toEqual(["user", "assistant", "user", "assistant"]);
  });

  it("preserves multi-turn order and folds non-standard roles into system", () => {
    const doc = loadFixture("long");
    const turns = chatgptAdapter.extractConversation(doc);
    // First turn is system, then alternating user/assistant. The trailing
    // streaming `Working on it…` turn still emits content in extraction
    // (the streaming gate is for `onNewAssistantTurn`, not for snapshot
    // capture).
    expect(turns[0]?.role).toBe("system");
    expect(turns[1]?.role).toBe("user");
    expect(turns[2]?.role).toBe("assistant");
    expect(turns.at(-1)?.role).toBe("assistant");
    expect(turns.at(-1)?.content).toContain("Working on it");
  });

  it("returns an empty list for a fresh empty chat", () => {
    const doc = loadFixture("empty");
    expect(chatgptAdapter.extractConversation(doc)).toEqual([]);
  });

  it("serializeMarkdown over extracted turns keeps fenced code blocks intact", () => {
    const doc = loadFixture("code");
    const turns = chatgptAdapter.extractConversation(doc);
    const md = serializeMarkdown(turns, {
      platform: "chatgpt",
      chatId: "abc123",
    });
    expect(md).toContain("```python\ndef sort_list(xs):\n    return sorted(xs)\n```");
    expect(md).toMatch(/```rust\nfn main\(\) \{[\s\S]+\}\n```/);
    // Role headings emitted in the order extracted.
    const userIdx = md.indexOf("### user");
    const asstIdx = md.indexOf("### assistant");
    expect(userIdx).toBeGreaterThan(-1);
    expect(asstIdx).toBeGreaterThan(userIdx);
  });
});

describe("chatgpt adapter · findInputBox", () => {
  // T07 D13 anchor — recall paste-into-chat needs a non-null element on
  // chat pages. Landing-style DOM (no textarea) should return null so
  // the popup can fall back to a clipboard copy.
  it("findInputBox_returns_textarea", () => {
    const chatDoc = loadFixture("empty");
    const found = chatgptAdapter.findInputBox(chatDoc);
    expect(found).not.toBeNull();
    expect(found?.tagName.toLowerCase()).toBe("textarea");
    expect(found?.getAttribute("data-id")).toBe("root");

    // A document without the prompt textarea (e.g. a logged-out landing
    // page) returns null.
    const empty = new JSDOM("<!doctype html><body></body>").window.document;
    expect(chatgptAdapter.findInputBox(empty)).toBeNull();
  });
});

describe("chatgpt adapter · getChatId", () => {
  it("parses the uuid out of a /c/<id> path", () => {
    const { doc, location } = loadFixtureWithLocation(
      "code",
      "https://chatgpt.com/c/abc123-uuid-xyz",
    );
    expect(chatgptAdapter.getChatId(doc, location)).toBe("abc123-uuid-xyz");
  });

  it("returns null on the landing page (no /c/ segment)", () => {
    const { doc, location } = loadFixtureWithLocation(
      "empty",
      "https://chatgpt.com/",
    );
    expect(chatgptAdapter.getChatId(doc, location)).toBeNull();
  });

  it("strips query string and hash from the chat id", () => {
    const { doc, location } = loadFixtureWithLocation(
      "code",
      "https://chatgpt.com/c/xyz-789?ref=foo#anchor",
    );
    expect(chatgptAdapter.getChatId(doc, location)).toBe("xyz-789");
  });
});

describe("chatgpt adapter · onNewAssistantTurn", () => {
  it("fires once when a streaming assistant turn loses data-stream", () => {
    const dom = new JSDOM(
      `<!doctype html><body>
        <div id="thread">
          <article
            data-message-author-role="assistant"
            data-message-id="a1"
            data-stream="true"
          ><div class="prose"><p>partial…</p></div></article>
        </div>
      </body>`,
      { url: "https://chatgpt.com/c/abc" },
    );
    const doc = dom.window.document;
    const turns: ChatTurn[] = [];
    const off = chatgptAdapter.onNewAssistantTurn!(doc, (t) => turns.push(t));

    // No emit yet — the turn is mid-stream.
    expect(turns).toEqual([]);

    // Mutate content + drop `data-stream` to simulate stream completion.
    const turn = doc.querySelector<HTMLElement>(
      '[data-message-author-role="assistant"]',
    )!;
    const prose = turn.querySelector("p")!;
    prose.textContent = "all done";
    turn.removeAttribute("data-stream");

    // MutationObserver flushes microtasks — wait one tick.
    return new Promise<void>((r) => {
      setTimeout(() => {
        expect(turns).toHaveLength(1);
        expect(turns[0]?.role).toBe("assistant");
        expect(turns[0]?.content).toContain("all done");
        off();
        r();
      }, 0);
    });
  });

  it("emits when a new already-settled assistant turn is appended", () => {
    const dom = new JSDOM(
      `<!doctype html><body><div id="thread"></div></body>`,
      { url: "https://chatgpt.com/c/abc" },
    );
    const doc = dom.window.document;
    const turns: ChatTurn[] = [];
    const off = chatgptAdapter.onNewAssistantTurn!(doc, (t) => turns.push(t));

    const thread = doc.getElementById("thread")!;
    const settled = doc.createElement("article");
    settled.setAttribute("data-message-author-role", "assistant");
    settled.innerHTML = `<div class="prose"><p>fresh reply</p></div>`;
    thread.appendChild(settled);

    return new Promise<void>((r) => {
      setTimeout(() => {
        expect(turns).toHaveLength(1);
        expect(turns[0]?.content).toContain("fresh reply");
        off();
        r();
      }, 0);
    });
  });

  it("returned disposer detaches the observer (no further emits)", () => {
    const dom = new JSDOM(
      `<!doctype html><body><div id="thread"></div></body>`,
      { url: "https://chatgpt.com/c/abc" },
    );
    const doc = dom.window.document;
    const turns: ChatTurn[] = [];
    const off = chatgptAdapter.onNewAssistantTurn!(doc, (t) => turns.push(t));
    off();

    const thread = doc.getElementById("thread")!;
    const settled = doc.createElement("article");
    settled.setAttribute("data-message-author-role", "assistant");
    settled.textContent = "should be ignored";
    thread.appendChild(settled);

    return new Promise<void>((r) => {
      setTimeout(() => {
        expect(turns).toEqual([]);
        r();
      }, 0);
    });
  });
});

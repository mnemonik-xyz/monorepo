import { JSDOM } from "jsdom";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  __setAdaptersForTesting,
  listAdapters,
  selectAdapter,
} from "../../../src/runtime/chat/registry.js";
import { geminiAdapter } from "../../../src/runtime/chat/adapters/gemini.adapter.js";
import { serializeMarkdown } from "../../../src/runtime/chat/serializer.js";
import type { ChatTurn } from "../../../src/runtime/chat/types.js";

// Snapshot the registry at module-evaluation time, before any
// `afterEach` reset has run, so the self-registration assertion below
// observes the side effect produced by `import` rather than whatever
// state the previous test left behind.
const adaptersAtImport = listAdapters().slice();

const FIXTURE_DIR = resolve(__dirname, "../../fixtures/gemini");

/**
 * JSDOM 24.1.x parses `<template shadowrootmode>` as a regular template
 * (declarative shadow DOM is gated behind a parser flag that the
 * `JSDOM` constructor doesn't expose in this minor). The fixtures use
 * declarative shadow DOM so they match the live Gemini DOM faithfully;
 * this helper applies the templates imperatively at load time so the
 * shadow-pierce code path is still exercised end-to-end.
 *
 * Refresh from a live HAR capture before merge — see decisions.md.
 */
function loadFixture(name: string): Document {
  const html = readFileSync(resolve(FIXTURE_DIR, `${name}.html`), "utf8");
  const dom = new JSDOM(html);
  const doc = dom.window.document;
  const templates = doc.querySelectorAll<HTMLTemplateElement>(
    "template[shadowrootmode]",
  );
  for (const tpl of Array.from(templates)) {
    const host = tpl.parentElement;
    if (!host) continue;
    const mode = (tpl.getAttribute("shadowrootmode") ?? "open") as
      | "open"
      | "closed";
    const root = host.attachShadow({ mode });
    root.append(...Array.from(tpl.content.childNodes));
    tpl.remove();
  }
  return doc;
}

afterEach(() => {
  __setAdaptersForTesting([]);
});

describe("gemini.adapter · contract", () => {
  it("exposes hostPattern + platform matching D11 enumerated host", () => {
    expect(geminiAdapter.platform).toBe("gemini");
    expect(geminiAdapter.hostPattern.source).toContain("gemini\\.google\\.com");
  });

  it("self-registers on import (last-line side effect)", () => {
    expect(adaptersAtImport).toContain(geminiAdapter);

    // Re-seed the (otherwise cleared) registry with the adapter under
    // test and confirm `selectAdapter` matches gemini.google.com URLs.
    __setAdaptersForTesting([geminiAdapter]);
    expect(selectAdapter("https://gemini.google.com/app/abc123")).toBe(
      geminiAdapter,
    );
    expect(selectAdapter("https://example.com")).toBeNull();
  });

  it("findInputBox returns null in read-only Phase 1", () => {
    const doc = loadFixture("empty");
    expect(geminiAdapter.findInputBox(doc)).toBeNull();
  });
});

describe("gemini.adapter · extractConversation", () => {
  // D13 TDD anchor — fixture with shadow roots → text contents present
  // in the extracted turns. Drives shadow-DOM correctness.
  it("shadow_dom_pierced_to_extract_content", () => {
    const doc = loadFixture("code");
    const turns = geminiAdapter.extractConversation(doc);
    expect(turns.length).toBeGreaterThan(0);

    const userTurn = turns.find((t) => t.role === "user");
    const asstTurn = turns.find((t) => t.role === "assistant");
    expect(userTurn?.content).toContain("How do I sort a list in Python?");
    expect(asstTurn?.content).toContain("Use the built-in `sorted`");
    // The fenced code block lives in a shadow-rooted <pre><code>; if
    // shadow piercing fails the fence disappears.
    expect(asstTurn?.content).toContain("```python");
    expect(asstTurn?.content).toContain("def sort(xs):");
  });

  // D13 TDD anchor — both roles labeled correctly.
  it("role_correct_for_assistant_and_user", () => {
    const doc = loadFixture("long");
    const turns = geminiAdapter.extractConversation(doc);
    const roles = turns.map((t) => t.role);
    expect(roles).toEqual([
      "user",
      "assistant",
      "user",
      "assistant",
      "user",
      "assistant",
    ]);
  });

  it("returns [] for an empty conversation", () => {
    const doc = loadFixture("empty");
    expect(geminiAdapter.extractConversation(doc)).toEqual([]);
  });

  it("preserves conversation order across multiple turns", () => {
    const doc = loadFixture("long");
    const turns = geminiAdapter.extractConversation(doc);
    expect(turns[0]?.content).toContain("capital of France");
    expect(turns[1]?.content).toContain("Paris");
    expect(turns[2]?.content).toContain("of Germany");
    expect(turns[3]?.content).toContain("Berlin");
    expect(turns[4]?.content).toContain("CSV");
    expect(turns[5]?.content).toContain("France,Paris");
  });
});

describe("gemini.adapter · getChatId", () => {
  function locFor(url: string): Location {
    return new URL(url) as unknown as Location;
  }

  it("parses /app/<id> from the path", () => {
    const id = geminiAdapter.getChatId(
      loadFixture("empty"),
      locFor("https://gemini.google.com/app/abc123"),
    );
    expect(id).toBe("abc123");
  });

  it("tolerates the /u/<n>/ account prefix", () => {
    const id = geminiAdapter.getChatId(
      loadFixture("empty"),
      locFor("https://gemini.google.com/u/2/app/xyz_789-id"),
    );
    expect(id).toBe("xyz_789-id");
  });

  it("ignores query string and hash", () => {
    const id = geminiAdapter.getChatId(
      loadFixture("empty"),
      locFor("https://gemini.google.com/app/foo?ref=share#section"),
    );
    expect(id).toBe("foo");
  });

  it("returns null for unscoped pages so the popup falls back to generic capture (D8)", () => {
    expect(
      geminiAdapter.getChatId(
        loadFixture("empty"),
        locFor("https://gemini.google.com/"),
      ),
    ).toBeNull();
    expect(
      geminiAdapter.getChatId(
        loadFixture("empty"),
        locFor("https://gemini.google.com/settings"),
      ),
    ).toBeNull();
  });
});

describe("gemini.adapter · markdown snapshot", () => {
  it("serializes a code-bearing turn through serializeMarkdown unmodified", () => {
    const doc = loadFixture("code");
    const turns = geminiAdapter.extractConversation(doc);
    const md = serializeMarkdown(turns, {
      platform: "gemini",
      chatId: "abc123",
      url: "https://gemini.google.com/app/abc123",
    });
    expect(md).toContain("### user");
    expect(md).toContain("### assistant");
    expect(md).toContain("How do I sort a list in Python?");
    expect(md).toContain("```python\ndef sort(xs):\n    return sorted(xs)\n```");
    expect(md).toMatch(/^---\nplatform: gemini\n/);
  });
});

describe("gemini.adapter · onNewAssistantTurn", () => {
  it("fires for assistant turns appended after the observer attaches", async () => {
    const doc = loadFixture("empty");
    const seen: ChatTurn[] = [];
    const detach = geminiAdapter.onNewAssistantTurn!(doc, (turn) =>
      seen.push(turn),
    );

    // Append a fresh <message-content> with an open shadow root +
    // assistant text. Mirrors a streamed response settling.
    const container = doc.querySelector("conversations-container")!;
    const msg = doc.createElement("message-content");
    container.appendChild(msg);
    const root = msg.attachShadow({ mode: "open" });
    const div = doc.createElement("div");
    div.textContent = "Streamed reply.";
    root.appendChild(div);

    // MutationObserver in JSDOM fires on a microtask; a deferred await
    // is enough to let it run.
    await new Promise((r) => setTimeout(r, 0));
    detach();

    expect(seen.map((t) => t.role)).toEqual(["assistant"]);
    expect(seen[0]?.content).toContain("Streamed reply.");
  });

  it("returns a detach function that stops further callbacks", async () => {
    const doc = loadFixture("empty");
    const seen: ChatTurn[] = [];
    const detach = geminiAdapter.onNewAssistantTurn!(doc, (turn) =>
      seen.push(turn),
    );
    detach();

    const container = doc.querySelector("conversations-container")!;
    const msg = doc.createElement("message-content");
    container.appendChild(msg);
    const root = msg.attachShadow({ mode: "open" });
    const div = doc.createElement("div");
    div.textContent = "Should not fire.";
    root.appendChild(div);

    await new Promise((r) => setTimeout(r, 0));
    expect(seen).toEqual([]);
  });
});

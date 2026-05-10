import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";
import {
  domNodeToMarkdown,
  serializeMarkdown,
  serializePayload,
} from "../../../src/runtime/chat/serializer.js";
import type {
  ChatMeta,
  ChatTurn,
} from "../../../src/runtime/chat/types.js";

const baseMeta: ChatMeta = {
  platform: "chatgpt",
  model: "gpt-4o",
  chatId: "abc123",
  url: "https://chatgpt.com/c/abc123",
  capturedAt: "2026-05-10T12:00:00Z",
};

describe("serializer · serializeMarkdown", () => {
  // T06 TDD anchor (D13): the serializer must round-trip code blocks
  // verbatim. Drives D8 (faithful capture of code-heavy AI chats) —
  // a chat that explained how to sort a list is useless if the code
  // is mangled.
  it("markdown_round_trip_preserves_code_blocks", () => {
    const code = "```python\ndef sort(xs):\n    return sorted(xs)\n```";
    const turns: ChatTurn[] = [
      { role: "user", content: "How do I sort a list?" },
      {
        role: "assistant",
        content: `Use \`sorted\`:\n\n${code}\n\nThat's it.`,
      },
    ];
    const md = serializeMarkdown(turns, baseMeta);
    expect(md).toContain(code);
    // Ensure we didn't accidentally HTML-escape the backticks or newlines
    // — `toContain` already checks substring identity, but assert the
    // full triple-fenced block survives unmodified.
    expect(md).toMatch(/```python\ndef sort\(xs\):\n {4}return sorted\(xs\)\n```/);
  });

  it("emits ### user / ### assistant / ### system role headings", () => {
    const turns: ChatTurn[] = [
      { role: "system", content: "You are a helpful assistant." },
      { role: "user", content: "Hi." },
      { role: "assistant", content: "Hello!" },
    ];
    const md = serializeMarkdown(turns, baseMeta);
    expect(md).toContain("### system");
    expect(md).toContain("### user");
    expect(md).toContain("### assistant");
    // Order is preserved.
    const sysIdx = md.indexOf("### system");
    const userIdx = md.indexOf("### user");
    const asstIdx = md.indexOf("### assistant");
    expect(sysIdx).toBeLessThan(userIdx);
    expect(userIdx).toBeLessThan(asstIdx);
  });

  it("emits a YAML frontmatter block with all populated meta fields", () => {
    const md = serializeMarkdown([], baseMeta);
    expect(md).toMatch(/^---\n/);
    expect(md).toContain("platform: chatgpt");
    expect(md).toContain("model: gpt-4o");
    expect(md).toContain("chat_id: abc123");
    expect(md).toContain("url: https://chatgpt.com/c/abc123");
    expect(md).toContain("captured_at: 2026-05-10T12:00:00Z");
    // Frontmatter terminator before any role heading.
    const closeIdx = md.indexOf("\n---\n");
    expect(closeIdx).toBeGreaterThan(0);
  });

  it("omits optional frontmatter fields when their meta source is missing", () => {
    const md = serializeMarkdown([], { platform: "claude" });
    expect(md).toContain("platform: claude");
    expect(md).not.toContain("model:");
    expect(md).not.toContain("chat_id:");
    expect(md).not.toContain("url:");
    expect(md).not.toContain("captured_at:");
  });

  it("includes per-turn timestamp + modelHint when adapters supply them", () => {
    const turns: ChatTurn[] = [
      {
        role: "assistant",
        content: "Done.",
        ts: "2026-05-10T12:00:01Z",
        modelHint: "gpt-4o-2026-05",
      },
    ];
    const md = serializeMarkdown(turns, baseMeta);
    expect(md).toContain("*2026-05-10T12:00:01Z*");
    expect(md).toContain("*model: gpt-4o-2026-05*");
  });
});

describe("serializer · serializePayload", () => {
  it("builds source/model/chat tags in stable order", () => {
    const { tags } = serializePayload([], baseMeta);
    expect(tags).toEqual([
      "source:chatgpt",
      "model:gpt-4o",
      "chat:abc123",
    ]);
  });

  it("drops tag entries whose source meta field is absent", () => {
    const { tags } = serializePayload([], { platform: "claude" });
    expect(tags).toEqual(["source:claude"]);
  });

  it("emits source_meta matching AttestationRow.source_meta exactly", () => {
    const { source_meta } = serializePayload([], baseMeta);
    expect(source_meta).toEqual({
      platform: "chatgpt",
      model: "gpt-4o",
      chat_id: "abc123",
      url: "https://chatgpt.com/c/abc123",
    });
  });

  it("omits absent source_meta keys instead of writing undefined (exactOptionalPropertyTypes)", () => {
    const { source_meta } = serializePayload([], { platform: "gemini" });
    expect(source_meta).toEqual({ platform: "gemini" });
    expect(Object.keys(source_meta)).toEqual(["platform"]);
  });

  it("content is the markdown body (the same string serializeMarkdown returns)", () => {
    const turns: ChatTurn[] = [{ role: "user", content: "Ping?" }];
    const payload = serializePayload(turns, baseMeta);
    expect(payload.content).toBe(serializeMarkdown(turns, baseMeta));
  });
});

describe("serializer · domNodeToMarkdown", () => {
  function parse(html: string): Element {
    const dom = new JSDOM(`<!doctype html><body><div id="r">${html}</div>`);
    const root = dom.window.document.getElementById("r");
    if (!root) throw new Error("no root");
    return root;
  }

  it("converts <pre><code class=\"language-python\"> into a fenced block with hint", () => {
    const root = parse(
      `<pre><code class="language-python">print("hi")\n</code></pre>`,
    );
    const md = domNodeToMarkdown(root);
    expect(md).toContain("```python\nprint(\"hi\")\n```");
  });

  it("emits an unhinted fence when the code element has no language-* class", () => {
    const root = parse(`<pre><code>echo ok</code></pre>`);
    const md = domNodeToMarkdown(root);
    expect(md).toMatch(/```\necho ok\n```/);
    expect(md).not.toContain("```language");
  });

  it("converts inline <code> to backticked spans", () => {
    const root = parse(`<p>Run <code>npm test</code> first.</p>`);
    const md = domNodeToMarkdown(root);
    expect(md).toContain("Run `npm test` first.");
  });

  it("strips a single trailing newline from <pre> content (highlighter quirk)", () => {
    const root = parse(
      `<pre><code class="language-rust">fn main() {}\n</code></pre>`,
    );
    const md = domNodeToMarkdown(root);
    expect(md).toContain("```rust\nfn main() {}\n```");
    expect(md).not.toContain("fn main() {}\n\n```");
  });
});

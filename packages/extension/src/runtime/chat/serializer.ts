import type { ChatMeta, ChatTurn } from "./types.js";
import type { SourceMeta } from "../store/types.js";

/**
 * Output of `serializePayload` — exactly what the popup hands to
 * `signMemory`. `content` is the human-readable markdown, `tags`
 * the AND-indexed search facets, `source_meta` the capture-context
 * metadata that lands on `AttestationRow.source_meta`.
 */
export interface ChatPayload {
  content: string;
  tags: string[];
  source_meta: SourceMeta;
}

const ROLE_HEADINGS: Record<ChatTurn["role"], string> = {
  user: "### user",
  assistant: "### assistant",
  system: "### system",
};

/**
 * Render captured turns as a markdown document with YAML frontmatter.
 * Per-turn `content` is emitted verbatim — adapters are responsible
 * for converting their platform's DOM into well-formed markdown
 * (see `domNodeToMarkdown`), so fenced code blocks they produce flow
 * straight through unchanged. Drives the D13 TDD anchor
 * `markdown_round_trip_preserves_code_blocks`.
 */
export function serializeMarkdown(turns: ChatTurn[], meta: ChatMeta): string {
  const frontmatter = buildFrontmatter(meta);
  const body = turns
    .map((turn) => {
      const tsLine = turn.ts ? `\n*${turn.ts}*` : "";
      const modelLine = turn.modelHint ? `\n*model: ${turn.modelHint}*` : "";
      return `${ROLE_HEADINGS[turn.role]}${tsLine}${modelLine}\n\n${turn.content}`;
    })
    .join("\n\n");
  return `${frontmatter}\n${body}\n`;
}

/**
 * Build the `signMemory` payload. Tags are emitted in a stable
 * order — `source:` first, then `model:`, then `chat:` — and any
 * tag whose source field is missing is dropped (vs. emitting
 * `model:undefined`). `source_meta` mirrors `SourceMeta` exactly,
 * with conditional spreads to satisfy `exactOptionalPropertyTypes`.
 */
export function serializePayload(
  turns: ChatTurn[],
  meta: ChatMeta,
): ChatPayload {
  const tags: string[] = [`source:${meta.platform}`];
  if (meta.model) tags.push(`model:${meta.model}`);
  if (meta.chatId) tags.push(`chat:${meta.chatId}`);

  const source_meta: SourceMeta = {
    platform: meta.platform,
    ...(meta.model !== undefined ? { model: meta.model } : {}),
    ...(meta.chatId !== undefined ? { chat_id: meta.chatId } : {}),
    ...(meta.url !== undefined ? { url: meta.url } : {}),
  };

  return {
    content: serializeMarkdown(turns, meta),
    tags,
    source_meta,
  };
}

/**
 * Convert a DOM subtree into markdown. Used by adapters during
 * `extractConversation` so `ChatTurn.content` arrives at the
 * serializer already-fenced.
 *
 * Code-block handling: a `<pre>` containing a `<code class="language-X">`
 * becomes ```` ```X ````-fenced; bare `<pre>` / `<code>` blocks fall back
 * to an unhinted ```` ``` ````. Inline `<code>` becomes backticked. Other
 * elements contribute their `textContent` — adapters that need richer
 * conversion (lists, tables, links) extend their own pipelines.
 */
export function domNodeToMarkdown(node: Node): string {
  if (node.nodeType === 3 /* TEXT_NODE */) {
    return node.textContent ?? "";
  }
  if (node.nodeType !== 1 /* ELEMENT_NODE */) {
    return "";
  }
  const el = node as Element;
  const tag = el.tagName.toLowerCase();

  if (tag === "pre") {
    const codeEl = el.querySelector("code");
    const lang = extractLanguageHint(codeEl ?? el);
    const text = (codeEl ?? el).textContent ?? "";
    const stripped = text.replace(/\n$/, "");
    return `\n\`\`\`${lang}\n${stripped}\n\`\`\`\n`;
  }
  if (tag === "code") {
    // Inline `<code>` — `<pre><code>` is handled above.
    const text = el.textContent ?? "";
    return `\`${text}\``;
  }

  let out = "";
  for (const child of Array.from(el.childNodes)) {
    out += domNodeToMarkdown(child);
  }
  return out;
}

function extractLanguageHint(el: Element): string {
  const cls = el.getAttribute("class") ?? "";
  const match = /(?:^|\s)language-([\w+-]+)/.exec(cls);
  return match?.[1] ?? "";
}

function buildFrontmatter(meta: ChatMeta): string {
  const lines: string[] = ["---", `platform: ${meta.platform}`];
  if (meta.model) lines.push(`model: ${meta.model}`);
  if (meta.chatId) lines.push(`chat_id: ${meta.chatId}`);
  if (meta.url) lines.push(`url: ${meta.url}`);
  if (meta.capturedAt) lines.push(`captured_at: ${meta.capturedAt}`);
  lines.push("---");
  return lines.join("\n");
}

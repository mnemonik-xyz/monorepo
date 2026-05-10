// Chat-adapter framework — platform-agnostic shapes consumed by the
// content-script and the popup recall path. T07–T09 register concrete
// adapters (ChatGPT, Claude.ai, Gemini) against this contract; the
// registry never imports a concrete adapter directly.

/** A single message in a captured chat conversation. */
export interface ChatTurn {
  role: "user" | "assistant" | "system";
  /**
   * Markdown body. Code blocks are emitted as fenced ``` regions with the
   * language hint extracted from `class="language-*"` on the source `<pre>`
   * / `<code>` element when present.
   */
  content: string;
  /** Best-effort ISO-8601 timestamp from the platform's DOM, when exposed. */
  ts?: string;
  /** Per-turn model hint when the platform surfaces it (e.g. switcher UI). */
  modelHint?: string;
}

/**
 * Capture-context metadata supplied by the adapter. Mirrors `SourceMeta`
 * in `runtime/store/types.ts`; serialised into both the markdown
 * frontmatter and the `source_meta` column on `AttestationRow`.
 */
export interface ChatMeta {
  /** `chatgpt` | `claude` | `gemini` | … (the adapter's declared platform). */
  platform: string;
  /** Best-effort model name (e.g. `gpt-4o`, `claude-3.5-sonnet`). */
  model?: string;
  /** Stable per-chat id when the platform exposes one. */
  chatId?: string;
  /** Source URL the capture came from. */
  url?: string;
  /** When the capture happened (ISO-8601). Adapters set this; serializer
   *  embeds it in the frontmatter without rewriting. */
  capturedAt?: string;
}

/**
 * Per-platform DOM extractor + selector. Implementations live in
 * `runtime/chat/adapters/*` (T07–T09); the registry treats them as
 * opaque values. Methods are sync — DOM parsing is cheap and the
 * content-script context is already on the page's main frame.
 */
export interface ChatAdapter {
  /**
   * Matched against `new URL(url).hostname + pathname` in `selectAdapter`.
   * Adapters anchor with `^` to keep matches predictable; the registry
   * does not normalise input beyond constructing a `URL`.
   */
  readonly hostPattern: RegExp;
  /** Stable platform identifier embedded in tags + `SourceMeta.platform`. */
  readonly platform: "chatgpt" | "claude" | "gemini" | string;
  /** Read the rendered conversation out of a `Document`. */
  extractConversation(doc: Document): ChatTurn[];
  /** Locate the prompt input element — used by the recall-overlay paste UI. */
  findInputBox(doc: Document): HTMLElement | null;
  /** Per-platform stable chat id. Returns `null` for unscoped pages. */
  getChatId(doc: Document, location: Location): string | null;
  /**
   * Optional auto-capture hook. Adapters that opt in subscribe to
   * "assistant turn finished streaming" events and call `cb` once per
   * settled turn; the returned function detaches the subscription.
   */
  onNewAssistantTurn?(
    doc: Document,
    cb: (turn: ChatTurn) => void,
  ): () => void;
}

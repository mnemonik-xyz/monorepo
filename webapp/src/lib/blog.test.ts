import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  deriveBlogPost,
  fetchBlogPost,
  fetchBlogPosts,
  type BlogPost,
} from "./blog";

describe("fetchBlogPosts", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns_live_posts_on_200", async () => {
    const posts = [
      {
        slug: "live",
        title: "Live post",
        summary: "s",
        body_markdown: "b",
        author: "node",
        published_at: "2026-01-01T00:00:00.000Z",
        tags: ["live"],
      },
    ];
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response(JSON.stringify({ posts }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const result = await fetchBlogPosts();
    const [p] = result.posts;
    expect(p).toBeDefined();
    // Present fields are preserved verbatim...
    expect(p!.summary).toBe("s");
    expect(p!.title).toBe("Live post");
    // ...and the omitted reading_minutes is derived (>= 1 minute).
    expect(p!.reading_minutes).toBeGreaterThanOrEqual(1);
  });

  it("throws_on_5xx_no_sample_fallback", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("oops", { status: 503 }),
    );

    await expect(fetchBlogPosts()).rejects.toThrow();
  });

  it("throws_on_network_error_no_sample_fallback", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new TypeError("network down"),
    );

    await expect(fetchBlogPosts()).rejects.toThrow();
  });
});

describe("fetchBlogPost", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns_the_live_post_on_200", async () => {
    const post = {
      slug: "live",
      title: "Live post",
      summary: "s",
      body_markdown: "b",
      author: "node",
      published_at: "2026-01-01T00:00:00.000Z",
      tags: ["live"],
    };
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response(JSON.stringify({ post }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const result = await fetchBlogPost("live");
    expect(result.post?.slug).toBe("live");
  });

  it("returns_null_post_for_a_404", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response(JSON.stringify({ post: null }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const result = await fetchBlogPost("does-not-exist");
    expect(result.post).toBeNull();
  });

  it("throws_on_network_error_no_sample_fallback", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new TypeError("network down"),
    );

    await expect(fetchBlogPost("live")).rejects.toThrow();
  });
});

describe("deriveBlogPost", () => {
  const base: BlogPost = {
    slug: "p",
    title: "T",
    summary: "",
    body_markdown:
      "## A heading\n\nFirst paragraph that should become the summary.\n\nSecond paragraph that must not appear in the summary.",
    author: "Mnemonic Protocol",
    published_at: "2026-06-01T00:00:00.000Z",
    tags: ["x"],
  };

  it("derives the summary from the first body paragraph when omitted", () => {
    const out = deriveBlogPost({ ...base, summary: "" });
    expect(out.summary).toBe("First paragraph that should become the summary.");
    // The leading markdown heading is not used as the summary.
    expect(out.summary).not.toContain("#");
    expect(out.summary).not.toContain("Second paragraph");
  });

  it("preserves a backend-provided summary", () => {
    const out = deriveBlogPost({ ...base, summary: "Editorial summary." });
    expect(out.summary).toBe("Editorial summary.");
  });

  it("caps a long derived summary to ~160 chars with an ellipsis", () => {
    const longPara = "word ".repeat(80).trim(); // ~400 chars, one paragraph
    const out = deriveBlogPost({
      ...base,
      summary: "",
      body_markdown: longPara,
    });
    expect(out.summary.length).toBeLessThanOrEqual(160);
    expect(out.summary.endsWith("…")).toBe(true);
  });

  it("derives reading_minutes at ~200 wpm, never below 1", () => {
    const short = deriveBlogPost({ ...base, body_markdown: "one two three" });
    expect(short.reading_minutes).toBe(1);
    const long = deriveBlogPost({
      ...base,
      body_markdown: "word ".repeat(450),
    });
    expect(long.reading_minutes).toBe(3); // ceil(450 / 200)
  });

  it("preserves a backend-provided reading_minutes", () => {
    const out = deriveBlogPost({ ...base, reading_minutes: 7 });
    expect(out.reading_minutes).toBe(7);
  });

  it("badges an agent author but not a human author", () => {
    expect(deriveBlogPost({ ...base, author: "research-agent-01" }).agent).toBe(
      "research-agent-01",
    );
    expect(deriveBlogPost({ ...base, author: "Mnemonic Protocol" }).agent).toBe(
      undefined,
    );
  });

  it("preserves an explicit agent field", () => {
    const out = deriveBlogPost({ ...base, author: "Human", agent: "bot-x" });
    expect(out.agent).toBe("bot-x");
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchBlogPost, fetchBlogPosts, sampleBlogPosts } from "./blog";

describe("fetchBlogPosts", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns_live_posts_with_sample_false_on_200", async () => {
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
    expect(result.sample).toBe(false);
    expect(result.posts).toEqual(posts);
  });

  it("degrades_to_sample_true_on_5xx", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("oops", { status: 503 }),
    );

    const result = await fetchBlogPosts();
    expect(result.sample).toBe(true);
    expect(result.posts).toEqual(sampleBlogPosts());
  });

  it("degrades_to_sample_true_on_network_error", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new TypeError("network down"),
    );

    const result = await fetchBlogPosts();
    expect(result.sample).toBe(true);
    expect(result.posts.length).toBeGreaterThan(0);
  });
});

describe("fetchBlogPost", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("falls_back_to_matching_sample_post_on_failure", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new TypeError("network down"),
    );

    const first = sampleBlogPosts()[0];
    expect(first).toBeDefined();
    const slug = first!.slug;
    const result = await fetchBlogPost(slug);
    expect(result.sample).toBe(true);
    expect(result.post?.slug).toBe(slug);
  });

  it("returns_null_post_for_unknown_slug_on_failure", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new TypeError("network down"),
    );

    const result = await fetchBlogPost("does-not-exist");
    expect(result.sample).toBe(true);
    expect(result.post).toBeNull();
  });
});

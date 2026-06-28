import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BlogPost } from "../lib/blog";

vi.mock("../components/SiteHeader", () => ({
  default: () => <header data-testid="site-header" />,
}));
vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));

const fetchBlogPost = vi.fn();
vi.mock("../lib/blog", () => ({
  fetchBlogPost: (slug: string) => fetchBlogPost(slug),
}));

import BlogPostPage from "./BlogPost";

const POST: BlogPost = {
  slug: "anchoring",
  title: "Anchoring memory on Solana and Arweave",
  summary: "How a participate write earns its anchors.",
  body_markdown:
    "## Two anchors\n\nA write earns **two anchors**:\n\n- Solana SPL Memo\n- Arweave bytes\n",
  author: "Mnemonic Protocol",
  published_at: "2026-06-12T09:15:00.000Z",
  tags: ["solana", "arweave"],
  reading_minutes: 4,
};

function renderAt(slug: string) {
  return render(
    <MemoryRouter initialEntries={[`/blog/${slug}`]}>
      <Routes>
        <Route path="/blog/:slug" element={<BlogPostPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("BlogPost page", () => {
  it("renders_markdown_body_as_html_elements", async () => {
    fetchBlogPost.mockResolvedValue({ post: POST });
    renderAt("anchoring");

    // Title from the post header.
    expect(
      await screen.findByRole("heading", { level: 1, name: POST.title }),
    ).toBeInTheDocument();
    // Markdown heading + list rendered as real elements.
    expect(
      screen.getByRole("heading", { level: 2, name: /two anchors/i }),
    ).toBeInTheDocument();
    expect(screen.getByText("Solana SPL Memo")).toBeInTheDocument();
    // Bold span rendered via <strong>, not literal asterisks.
    expect(screen.getByText("two anchors")).toBeInTheDocument();
  });

  it("renders_not_found_for_unknown_slug", async () => {
    fetchBlogPost.mockResolvedValue({ post: null });
    renderAt("does-not-exist");
    expect(await screen.findByTestId("post-not-found")).toBeInTheDocument();
  });

  it("emits_per_post_seo_title_and_article_jsonld", async () => {
    fetchBlogPost.mockResolvedValue({ post: POST });
    renderAt("anchoring");
    await screen.findByTestId("post-body");

    // React 19 hoists <title> into <head>.
    await vi.waitFor(() => {
      expect(document.title).toContain(POST.title);
    });

    const ld = document.querySelector('script[type="application/ld+json"]');
    expect(ld).not.toBeNull();
    const parsed = JSON.parse(ld?.textContent ?? "{}");
    expect(parsed["@type"]).toBe("BlogPosting");
    expect(parsed.headline).toBe(POST.title);
    expect(parsed.url).toContain("/blog/anchoring");

    const canonical = document.querySelector('link[rel="canonical"]');
    expect(canonical?.getAttribute("href")).toContain("/blog/anchoring");
  });

  it("does_not_inject_raw_html_from_markdown_body", async () => {
    const malicious: BlogPost = {
      ...POST,
      slug: "xss",
      body_markdown:
        'Safe **text** here\n\n<img src=x onerror="alert(1)" />\n\n<script>alert(2)</script>\n\n[evil](javascript:alert(3))\n',
    };
    fetchBlogPost.mockResolvedValue({ post: malicious });
    renderAt("xss");

    const body = await screen.findByTestId("post-body");
    // Raw HTML in the body must NOT become live elements.
    expect(body.querySelector("img")).toBeNull();
    expect(body.querySelector("script")).toBeNull();
    // Escaped, not dropped: the dangerous markup survives as inert text.
    expect(body.textContent).toContain("<script>alert(2)</script>");
    // Benign markdown around it still renders, including bold as <strong>.
    expect(body.textContent).toContain("Safe");
    expect(body.querySelector("strong")?.textContent).toBe("text");
    // Any rendered link must not carry a javascript: URL.
    for (const a of Array.from(body.querySelectorAll("a"))) {
      expect(a.getAttribute("href") ?? "").not.toMatch(/^javascript:/i);
    }
  });

  it("renders_empty_body_without_crashing", async () => {
    const empty: BlogPost = { ...POST, slug: "empty", body_markdown: "" };
    fetchBlogPost.mockResolvedValue({ post: empty });
    renderAt("empty");
    expect(
      await screen.findByRole("heading", { level: 1, name: POST.title }),
    ).toBeInTheDocument();
    const body = screen.getByTestId("post-body");
    expect(body.querySelector("p")).toBeNull();
  });

  it("renders_error_state_when_fetch_rejects", async () => {
    fetchBlogPost.mockRejectedValue(new Error("network"));
    renderAt("anchoring");
    expect(await screen.findByTestId("post-error")).toBeInTheDocument();
  });

  it("falls_back_to_raw_string_for_unparseable_date", async () => {
    const weird: BlogPost = {
      ...POST,
      slug: "weird-date",
      published_at: "not-a-real-date",
    };
    fetchBlogPost.mockResolvedValue({ post: weird });
    renderAt("weird-date");
    const time = await screen.findByText("not-a-real-date");
    expect(time.tagName).toBe("TIME");
  });

  it("renders_loading_state_before_resolution", async () => {
    let resolve: (v: { post: BlogPost | null }) => void = () => {};
    fetchBlogPost.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    renderAt("anchoring");
    expect(screen.getByTestId("post-loading")).toBeInTheDocument();
    resolve({ post: POST });
    await screen.findByTestId("post-body");
  });
});

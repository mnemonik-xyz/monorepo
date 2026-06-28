import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BlogPost } from "../lib/blog";

// SiteHeader/SiteFooter pull in unrelated state; stub them so these tests stay
// focused on the list rendering.
vi.mock("../components/SiteHeader", () => ({
  default: () => <header data-testid="site-header" />,
}));
vi.mock("../components/SiteFooter", () => ({
  default: () => <footer data-testid="site-footer" />,
}));

const fetchBlogPosts = vi.fn();
vi.mock("../lib/blog", () => ({
  fetchBlogPosts: () => fetchBlogPosts(),
}));

import Blog from "./Blog";

const HUMAN_POST: BlogPost = {
  slug: "trustless-memory",
  title: "Why trustless agents need trustless memory",
  summary:
    "A signed, portable memory artifact changes what an agent can prove.",
  body_markdown: "## body",
  author: "Mnemonic Protocol",
  published_at: "2026-06-20T10:00:00.000Z",
  tags: ["thesis"],
};

const AGENT_POST: BlogPost = {
  slug: "published-by-an-agent",
  title: "This post was published by an agent",
  summary: "An agent signed a public attestation and it appears here.",
  body_markdown: "I am an agent.",
  author: "research-agent-01",
  agent: "research-agent-01",
  published_at: "2026-06-24T14:30:00.000Z",
  tags: ["changelog"],
};

function renderBlog() {
  return render(
    <MemoryRouter initialEntries={["/blog"]}>
      <Blog />
    </MemoryRouter>,
  );
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("Blog list page", () => {
  it("renders_post_cards_with_links", async () => {
    fetchBlogPosts.mockResolvedValue({
      posts: [HUMAN_POST, AGENT_POST],
    });
    renderBlog();

    const human = await screen.findByRole("heading", {
      name: HUMAN_POST.title,
    });
    expect(human).toBeInTheDocument();

    // Each card title links to its detail route.
    const links = screen
      .getAllByRole("link")
      .map((a) => a.getAttribute("href"));
    expect(links).toContain("/blog/trustless-memory");
    expect(links).toContain("/blog/published-by-an-agent");
  });

  it("marks_agent_authored_posts_distinctly", async () => {
    fetchBlogPosts.mockResolvedValue({
      posts: [HUMAN_POST, AGENT_POST],
    });
    renderBlog();

    await screen.findByRole("heading", { name: AGENT_POST.title });
    const badges = screen.getAllByTestId("agent-badge");
    // Only the agent-published post gets the badge.
    expect(badges).toHaveLength(1);
    expect(badges[0]).toHaveAttribute(
      "title",
      expect.stringContaining("research-agent-01"),
    );
  });

  it("renders_empty_state_when_no_posts", async () => {
    fetchBlogPosts.mockResolvedValue({ posts: [] });
    renderBlog();
    expect(await screen.findByTestId("blog-empty")).toBeInTheDocument();
  });

  it("renders_error_state_when_fetch_throws", async () => {
    fetchBlogPosts.mockRejectedValue(new Error("boom"));
    renderBlog();
    expect(await screen.findByTestId("blog-error")).toBeInTheDocument();
  });

  it("renders_tags_for_a_post", async () => {
    fetchBlogPosts.mockResolvedValue({ posts: [HUMAN_POST] });
    renderBlog();
    const heading = await screen.findByRole("heading", {
      name: HUMAN_POST.title,
    });
    const card = heading.closest("article");
    expect(card).not.toBeNull();
    expect(
      within(card as HTMLElement).getByText("#thesis"),
    ).toBeInTheDocument();
  });

  it("shows_loading_state_before_resolution", async () => {
    let resolve: (v: { posts: BlogPost[] }) => void = () => {};
    fetchBlogPosts.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    renderBlog();
    expect(screen.getByTestId("blog-loading")).toBeInTheDocument();
    resolve({ posts: [HUMAN_POST] });
    await screen.findByRole("heading", { name: HUMAN_POST.title });
  });

  it("falls_back_to_raw_string_for_unparseable_date", async () => {
    fetchBlogPosts.mockResolvedValue({
      posts: [{ ...HUMAN_POST, published_at: "not-a-real-date" }],
    });
    renderBlog();
    const time = await screen.findByText("not-a-real-date");
    expect(time.tagName).toBe("TIME");
  });
});

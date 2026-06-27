import { render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Seo, articleJsonLd, organizationJsonLd } from "./seo";

// React 19 hoists <title>/<meta>/<link> into <head>; testing-library's
// automatic cleanup unmounts after each test and React removes the nodes it
// hoisted, so no manual head teardown is needed (and manual removal would race
// React's reference-counted hoistable resources during unmount).

describe("<Seo>", () => {
  it("renders title, description and canonical into the head", async () => {
    render(
      <Seo
        title="Ledger"
        description="Public evidence ledger"
        canonical="/ledger"
      />,
    );

    await waitFor(() => {
      expect(document.title).toBe("Ledger — Mnemonic Protocol");
    });

    const desc = document.head.querySelector('meta[name="description"]');
    expect(desc?.getAttribute("content")).toBe("Public evidence ledger");

    const canonical = document.head.querySelector('link[rel="canonical"]');
    expect(canonical?.getAttribute("href")).toBe("https://mnemonik.xyz/ledger");
  });

  it("respects exactTitle", async () => {
    render(
      <Seo title="Exact Title" description="d" canonical="/" exactTitle />,
    );
    await waitFor(() => {
      expect(document.title).toBe("Exact Title");
    });
  });

  it("emits an inline same-origin JSON-LD script (no remote src)", () => {
    const { container } = render(
      <Seo
        title="Home"
        description="d"
        canonical="/"
        jsonLd={organizationJsonLd()}
      />,
    );

    const script = container.querySelector(
      'script[type="application/ld+json"]',
    );
    expect(script).not.toBeNull();
    expect(script?.getAttribute("src")).toBeNull();
    const parsed = JSON.parse(script?.textContent ?? "{}");
    expect(parsed["@type"]).toBe("Organization");
  });

  it("renders one script per JSON-LD block when given an array", () => {
    const { container } = render(
      <Seo
        title="Post"
        description="d"
        canonical="/blog/x"
        type="article"
        jsonLd={[
          organizationJsonLd(),
          articleJsonLd({
            title: "Post",
            description: "d",
            url: "/blog/x",
            author: "agent-01",
            publishedAt: "2026-06-24T14:30:00.000Z",
          }),
        ]}
      />,
    );

    const scripts = container.querySelectorAll(
      'script[type="application/ld+json"]',
    );
    expect(scripts).toHaveLength(2);
  });

  it("escapes </script> in JSON-LD values to prevent script-tag breakout", () => {
    // An agent-authored post could carry markup in its title/author. The
    // serialized JSON-LD must not let that close the script element.
    const { container } = render(
      <Seo
        title="Post"
        description="d"
        canonical="/blog/x"
        type="article"
        jsonLd={articleJsonLd({
          title: "Pwn </script><script>alert(1)</script>",
          description: "d",
          url: "/blog/x",
          author: "agent & co",
          publishedAt: "2026-06-24T14:30:00.000Z",
        })}
      />,
    );

    const script = container.querySelector(
      'script[type="application/ld+json"]',
    );
    const raw = script?.textContent ?? "";
    // No raw tag delimiters survive — breakout is impossible.
    expect(raw).not.toContain("</script>");
    expect(raw).not.toContain("<script>");
    expect(raw).not.toContain("<");
    expect(raw).not.toContain(">");
    // Still valid JSON: a crawler decodes the escapes back to the real value.
    const parsed = JSON.parse(raw);
    expect(parsed.headline).toBe("Pwn </script><script>alert(1)</script>");
    expect(parsed.author).toMatchObject({ name: "agent & co" });
  });
});

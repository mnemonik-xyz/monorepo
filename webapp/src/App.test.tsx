import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// Stub every routed page to a lightweight marker so this suite tests routing
// wiring only — not page internals (which fetch on mount). Each marker is
// identifiable by test id.
vi.mock("./pages/Landing", () => ({
  default: () => <div data-testid="page-landing" />,
}));
vi.mock("./pages/Install", () => ({
  default: () => <div data-testid="page-install" />,
}));
vi.mock("./pages/Roadmap", () => ({
  default: () => <div data-testid="page-roadmap" />,
}));
vi.mock("./pages/Privacy", () => ({
  default: () => <div data-testid="page-privacy" />,
}));
vi.mock("./pages/Chat", () => ({
  default: () => <div data-testid="page-chat" />,
}));
vi.mock("./pages/Sign", () => ({
  default: () => <div data-testid="page-sign" />,
}));
vi.mock("./pages/Consent", () => ({
  default: () => <div data-testid="page-consent" />,
}));
vi.mock("./pages/Ledger", () => ({
  default: () => <div data-testid="page-ledger" />,
}));
vi.mock("./pages/Analytics", () => ({
  default: () => <div data-testid="page-analytics" />,
}));
vi.mock("./pages/Blog", () => ({
  default: () => <div data-testid="page-blog" />,
}));
// Echo the captured :slug param so the test can confirm param routing, not
// just that the BlogPost element mounted.
vi.mock("./pages/BlogPost", async () => {
  const rr =
    await vi.importActual<typeof import("react-router-dom")>(
      "react-router-dom",
    );
  return {
    default: () => {
      const { slug } = rr.useParams();
      return <div data-testid="page-blogpost">{slug}</div>;
    },
  };
});

import App from "./App";

function renderAt(path: string) {
  window.history.pushState({}, "", path);
  return render(<App />);
}

afterEach(() => {
  vi.clearAllMocks();
  window.history.pushState({}, "", "/");
});

describe("App routing", () => {
  it("routes_/ledger_to_Ledger", () => {
    renderAt("/ledger");
    expect(screen.getByTestId("page-ledger")).toBeInTheDocument();
  });

  it("routes_/analytics_to_Analytics", () => {
    renderAt("/analytics");
    expect(screen.getByTestId("page-analytics")).toBeInTheDocument();
  });

  it("routes_/blog_to_Blog", () => {
    renderAt("/blog");
    expect(screen.getByTestId("page-blog")).toBeInTheDocument();
  });

  it("routes_/blog/:slug_to_BlogPost_and_captures_slug", () => {
    renderAt("/blog/some-post");
    const page = screen.getByTestId("page-blogpost");
    expect(page).toBeInTheDocument();
    expect(page).toHaveTextContent("some-post");
  });

  it("preserves_existing_install_route", () => {
    renderAt("/install");
    expect(screen.getByTestId("page-install")).toBeInTheDocument();
  });

  // Scope guardrail (tech-spec): the OAuth/signing routes must stay unaffected
  // by the new public routes. /sign/:correlationId in particular sits above the
  // catch-all, so a routing regression there would silently redirect to "/".
  it("preserves_sign_route", () => {
    renderAt("/sign/corr-123");
    expect(screen.getByTestId("page-sign")).toBeInTheDocument();
  });

  it("preserves_oauth_consent_route", () => {
    renderAt("/oauth/consent");
    expect(screen.getByTestId("page-consent")).toBeInTheDocument();
  });

  it("preserves_chat_route", () => {
    renderAt("/chat");
    expect(screen.getByTestId("page-chat")).toBeInTheDocument();
  });

  it("preserves_catch_all_redirect_to_landing", () => {
    renderAt("/this-route-does-not-exist");
    expect(screen.getByTestId("page-landing")).toBeInTheDocument();
  });
});

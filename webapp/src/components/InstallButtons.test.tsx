import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import InstallButtons from "./InstallButtons";

describe("InstallButtons", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("deeplink_url_well_formed", () => {
    render(<InstallButtons />);

    // Cursor deeplink: scheme + base64-encoded {url:"https://mcp.mnemonik.xyz/mcp"}
    const cursor = screen.getByTestId("install-cursor") as HTMLAnchorElement;
    expect(cursor.href.startsWith("cursor://")).toBe(true);
    const cursorUrl = new URL(cursor.href);
    expect(cursorUrl.searchParams.get("name")).toBe("Mnemonic");
    const cursorConfigB64 = cursorUrl.searchParams.get("config");
    expect(cursorConfigB64).not.toBeNull();
    const decodedConfig = JSON.parse(atob(cursorConfigB64!));
    expect(decodedConfig).toEqual({
      url: "https://mcp.mnemonik.xyz/mcp",
      type: "http",
    });

    // VS Code deeplink: opaque URI (`vscode:`, NO `//`) + url-encoded JSON
    // config as a single param (NOT separate `name=&url=` query params).
    //
    // Regression guard — both shape errors below have shipped before and
    // produce the SAME symptom (VS Code opens, dialog never appears):
    //   1. `vscode://mcp/install?...` — hierarchical form. Parser sees
    //      authority="mcp", path="/install"; install handler matches
    //      path="mcp/install" so handler never fires.
    //   2. {..., headers: ...} — VS Code's install-link JSON validator
    //      is strict; only {name, type:"http", url} is accepted. Extra
    //      fields silently abort the install dialog.
    const vscode = screen.getByTestId("install-vscode") as HTMLAnchorElement;
    expect(vscode.href.startsWith("vscode:mcp/install?")).toBe(true);


    const vscodeConfig = decodeURIComponent(
      vscode.href.replace("vscode:mcp/install?", ""),


    );
    const parsedVscode = JSON.parse(vscodeConfig);
    expect(parsedVscode).toEqual({
      name: "Mnemonic",
      type: "http",
      url: "https://mcp.mnemonik.xyz/mcp",
    });
    expect(parsedVscode.headers).toBeUndefined();

    // Claude.ai is a button, not an anchor — it opens a modal that exposes
    // the paste URL — must include scheme so Claude.ai accepts it.
    const claude = screen.getByTestId("install-claude-ai");
    fireEvent.click(claude);
    const pasteUrl = screen.getByTestId("claude-paste-url");
    expect(pasteUrl.textContent).toBe("https://mcp.mnemonik.xyz/mcp");

    // WindSurf has no deeplink for arbitrary remote MCP URLs — its
    // `windsurf://windsurf-mcp-registry?serverName=` scheme only resolves
    // first-party registry entries (per docs.windsurf.com/windsurf/cascade/mcp).
    // We mirror the Claude.ai flow instead: a button opens a modal with the
    // JSON snippet that goes into `~/.codeium/windsurf/mcp_config.json`.
    const windsurf = screen.getByTestId("install-windsurf");
    expect(windsurf.tagName).toBe("BUTTON"); // not an anchor — no deeplink.
    fireEvent.click(windsurf);
    const snippetEl = screen.getByTestId("windsurf-config-snippet");
    const parsedWindsurf = JSON.parse(snippetEl.textContent ?? "{}");
    expect(parsedWindsurf).toEqual({
      mcpServers: {
        mnemonic: { serverUrl: "https://mcp.mnemonik.xyz/mcp" },
      },
    });
  });

  it("cursor_deeplink_bakes_jwt_when_logged_in", () => {
    // When the webapp has a non-expired JWT in localStorage, the Cursor
    // deeplink includes the JWT in `headers.Authorization`. Cursor's
    // deeplink handler stores it in mcp.json and sends it on every MCP
    // call — bypasses Cursor 3.2.16's broken OAuth UX (no Connect button
    // for non-directory MCP servers).
    //
    // VS Code is INTENTIONALLY excluded — its install-link JSON validator
    // rejects extra fields like `headers`. See `vscode_deeplink_never_bakes_jwt`
    // below for the regression guard.
    const futureExp = Math.floor(Date.now() / 1000) + 3600; // 1h from now
    const headerB64 = btoa(JSON.stringify({ alg: "HS256", typ: "JWT" }))
      .replace(/=+$/, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
    const payloadB64 = btoa(
      JSON.stringify({ sub: "test-pubkey", exp: futureExp })
    )
      .replace(/=+$/, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
    const jwt = `${headerB64}.${payloadB64}.fakesignature`;
    localStorage.setItem("mnemonic.jwt", jwt);

    render(<InstallButtons />);

    const cursor = screen.getByTestId("install-cursor") as HTMLAnchorElement;
    const cursorUrl = new URL(cursor.href);
    const cursorConfig = JSON.parse(
      atob(cursorUrl.searchParams.get("config")!)
    );
    expect(cursorConfig.headers).toEqual({
      Authorization: `Bearer ${jwt}`,
    });
  });

  it("vscode_deeplink_never_bakes_jwt", () => {
    // REGRESSION GUARD (2026-05-02): commit 2fad606 baked the JWT into
    // both Cursor and VS Code deeplinks. The `headers` field in VS Code's
    // install-link JSON makes VS Code's validator silently reject the
    // install — VS Code opens but the install dialog never appears.
    //
    // VS Code 1.93+ does MCP OAuth natively, so the JWT bake-in is
    // unneeded for VS Code anyway. This test asserts that VS Code's
    // deeplink contains ONLY {name, type, url} regardless of login state.
    const futureExp = Math.floor(Date.now() / 1000) + 3600;
    const headerB64 = btoa(JSON.stringify({ alg: "HS256", typ: "JWT" }))
      .replace(/=+$/, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
    const payloadB64 = btoa(
      JSON.stringify({ sub: "test-pubkey", exp: futureExp })
    )
      .replace(/=+$/, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
    const jwt = `${headerB64}.${payloadB64}.fakesignature`;
    localStorage.setItem("mnemonic.jwt", jwt);

    render(<InstallButtons />);

    const vscode = screen.getByTestId("install-vscode") as HTMLAnchorElement;
    const vscodeConfig = JSON.parse(
      decodeURIComponent(vscode.href.replace("vscode:mcp/install?", ""))
    );
    expect(vscodeConfig).toEqual({
      name: "Mnemonic",
      type: "http",
      url: "https://mcp.mnemonik.xyz/mcp",
    });
    expect(vscodeConfig.headers).toBeUndefined();
  });

  it("install_deeplinks_skip_jwt_when_expired", () => {
    // Expired JWT in localStorage MUST NOT be baked in — would cause
    // every IDE call to 401 from the moment of install.
    const pastExp = Math.floor(Date.now() / 1000) - 3600; // 1h ago
    const headerB64 = btoa(JSON.stringify({ alg: "HS256", typ: "JWT" }))
      .replace(/=+$/, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
    const payloadB64 = btoa(
      JSON.stringify({ sub: "test-pubkey", exp: pastExp })
    )
      .replace(/=+$/, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
    localStorage.setItem(
      "mnemonic.jwt",
      `${headerB64}.${payloadB64}.fakesignature`
    );

    render(<InstallButtons />);

    const cursor = screen.getByTestId("install-cursor") as HTMLAnchorElement;
    const cursorUrl = new URL(cursor.href);
    const cursorConfig = JSON.parse(
      atob(cursorUrl.searchParams.get("config")!)
    );
    expect(cursorConfig.headers).toBeUndefined();
  });
});

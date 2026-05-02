import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import InstallButtons from "./InstallButtons";

describe("InstallButtons", () => {
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

    // VS Code deeplink: scheme + url-encoded JSON config (single param, NOT
    // separate `name=&url=` query params — VS Code MCP install dialog only
    // recognizes the JSON-blob format).
    const vscode = screen.getByTestId("install-vscode") as HTMLAnchorElement;
    expect(vscode.href.startsWith("vscode://mcp/install?")).toBe(true);
    const vscodeConfig = decodeURIComponent(
      vscode.href.replace("vscode://mcp/install?", "")
    );
    const parsedVscode = JSON.parse(vscodeConfig);
    expect(parsedVscode).toEqual({
      name: "Mnemonic",
      type: "http",
      url: "https://mcp.mnemonik.xyz/mcp",
    });

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
});

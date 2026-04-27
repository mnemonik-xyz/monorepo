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
    expect(decodedConfig).toEqual({ url: "https://mcp.mnemonik.xyz/mcp" });

    // VS Code deeplink: scheme + url-encoded MCP URL.
    const vscode = screen.getByTestId("install-vscode") as HTMLAnchorElement;
    expect(vscode.href.startsWith("vscode:")).toBe(true);
    // URLSearchParams decodes percent-encoding, so we read the raw href.
    expect(vscode.href).toContain(
      encodeURIComponent("https://mcp.mnemonik.xyz/mcp")
    );
    expect(vscode.href).toContain("name=Mnemonic");

    // Claude.ai is a button, not an anchor — it opens a modal that exposes
    // the paste URL `mcp.mnemonik.xyz`.
    const claude = screen.getByTestId("install-claude-ai");
    fireEvent.click(claude);
    const pasteUrl = screen.getByTestId("claude-paste-url");
    expect(pasteUrl.textContent).toBe("mcp.mnemonik.xyz");
  });
});

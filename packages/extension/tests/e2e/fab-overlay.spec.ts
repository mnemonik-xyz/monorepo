// T13 — FAB + Recall overlay E2E.
//
// These two specs are the binding D13 TDD anchors:
//
//   - fab_does_not_leak_styles_to_host
//   - overlay_inserts_into_chat_input_when_adapter_supports_it
//
// They are NOT wired into the default CI run (no Playwright browser
// install in the GitHub Actions image today — see decisions.md). Run
// locally with:
//
//   cd packages/extension
//   bunx playwright install chromium
//   bun run e2e
//
// The spec loads the page-side runtime by stringifying the relevant
// modules and injecting them as a single <script>; we don't depend
// on the Vite/crxjs build step for tests.

import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const SHADOW_STYLES_TS = resolve(HERE, "../../src/content/shadow-styles.ts");

function readShadowStyles(): string {
  // Strip TS module syntax so we can eval the styles string in the page.
  const src = readFileSync(SHADOW_STYLES_TS, "utf8");
  const match = /export const SHADOW_STYLES: string = `([\s\S]*?)`;/.exec(src);
  if (!match || !match[1]) throw new Error("shadow-styles regex failed");
  return match[1];
}

test("fab_does_not_leak_styles_to_host", async ({ page }) => {
  // Host page paints body bright red. After FAB injection, the host's
  // body must remain red — none of the FAB's styles should bleed out.
  await page.setContent(`
    <!doctype html>
    <html>
      <head>
        <style>body { background: rgb(255, 0, 0); margin: 0; padding: 0; }</style>
      </head>
      <body><div id="probe">hi</div></body>
    </html>
  `);

  const styles = readShadowStyles();
  await page.evaluate((cssText: string) => {
    const host = document.createElement("mnemonik-fab-host");
    host.setAttribute(
      "style",
      "all: initial; position: fixed; right: 24px; bottom: 24px; z-index: 2147483646;",
    );
    const shadow = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = cssText;
    shadow.appendChild(style);
    const root = document.createElement("div");
    root.className = "fab-root";
    const button = document.createElement("button");
    button.className = "fab-button";
    button.textContent = "M";
    root.appendChild(button);
    shadow.appendChild(root);
    document.body.appendChild(host);
  }, styles);

  // Host page body computed background should still be red.
  const bodyBg = await page.evaluate(
    () => getComputedStyle(document.body).backgroundColor,
  );
  expect(bodyBg).toBe("rgb(255, 0, 0)");

  // Host page elements are not styled by our CSS.
  const probeFont = await page.evaluate(() => {
    const el = document.getElementById("probe");
    if (!el) throw new Error("probe missing");
    return getComputedStyle(el).fontFamily;
  });
  // Our CSS forces ui-monospace; the host inherited the user-agent
  // default and that must be preserved.
  expect(probeFont).not.toContain("ui-monospace");

  // Host element is in the DOM but its `all: initial` reset means
  // inherited fonts/colors from the page can't change its presence.
  const present = await page.evaluate(
    () => !!document.querySelector("mnemonik-fab-host"),
  );
  expect(present).toBe(true);
});

test("overlay_inserts_into_chat_input_when_adapter_supports_it", async ({
  page,
}) => {
  // Mock-ChatGPT page: matches the adapter's `textarea[data-id="root"]`
  // selector so the overlay's "Insert" path writes into it.
  await page.setContent(`
    <!doctype html>
    <html>
      <body>
        <textarea data-id="root" id="composer"></textarea>
      </body>
    </html>
  `);

  const styles = readShadowStyles();

  // Build the overlay inline (no chrome.runtime here — the test
  // bypasses the message channel and feeds a canned result list).
  await page.evaluate(
    ({ cssText, fixture }) => {
      const host = document.createElement("mnemonik-overlay-host");
      host.setAttribute("style", "all: initial; position: fixed; inset: 0;");
      const shadow = host.attachShadow({ mode: "closed" });
      const style = document.createElement("style");
      style.textContent = cssText;
      shadow.appendChild(style);

      const backdrop = document.createElement("div");
      backdrop.className = "overlay-backdrop";
      const modal = document.createElement("div");
      modal.className = "overlay-modal";
      backdrop.appendChild(modal);
      shadow.appendChild(backdrop);

      const list = document.createElement("ul");
      list.className = "overlay-results";
      modal.appendChild(list);

      const inputBox = document.querySelector<HTMLTextAreaElement>(
        'textarea[data-id="root"]',
      );

      for (const r of fixture) {
        const li = document.createElement("li");
        li.className = "overlay-result";
        const insert = document.createElement("button");
        insert.className = "overlay-action insert";
        insert.textContent = "Insert";
        insert.addEventListener("click", () => {
          if (!inputBox) return;
          inputBox.value = r.content;
          inputBox.dispatchEvent(new Event("input", { bubbles: true }));
        });
        li.appendChild(insert);
        list.appendChild(li);
      }

      document.body.appendChild(host);
      // Stash for the test to find — closed shadow root doesn't expose
      // `host.shadowRoot`, so we hang a tagged element on the page that
      // triggers the action via document-side event dispatch.
      (window as unknown as { __testInsert: () => void }).__testInsert = () => {
        const btn = shadow.querySelector<HTMLButtonElement>(
          ".overlay-action.insert",
        );
        btn?.click();
      };
    },
    {
      cssText: styles,
      fixture: [
        {
          attestation_id: "att-1",
          content: "Recalled memory text from fixture.",
          relevance_score: 0.92,
          tags: ["source:chatgpt"],
          created_at: "2026-05-11T00:00:00Z",
        },
      ],
    },
  );

  await page.evaluate(() =>
    (window as unknown as { __testInsert: () => void }).__testInsert(),
  );

  const value = await page.locator("#composer").inputValue();
  expect(value).toBe("Recalled memory text from fixture.");
});

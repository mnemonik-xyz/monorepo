// T19 — Playwright harness for the loaded-extension specs.
//
// Each test calls `launchExtension()` which:
//   1. Resolves `packages/extension/dist/` (asserts the build ran).
//   2. Launches a Chromium persistent context with
//      `--load-extension=$dist` + `--disable-extensions-except=$dist`.
//   3. Resolves the extension id by waiting for the service-worker
//      registration and parsing its url.
//   4. Returns helpers for opening the popup, mocking auth, mocking
//      the Mnemonik server, and seeding IndexedDB.
//
// Cleanup is handled per-test via `context.close()` in `afterEach`
// (or via the `t.use()` fixture pattern when the spec needs the
// helper inline). Keep this file under ~150 LOC; the network mock
// helpers live in `./mocks/server.ts` and `./mocks/google-oauth.ts`.

import {
  chromium,
  type BrowserContext,
  type Page,
  type Worker,
} from "@playwright/test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
export const DIST_DIR = resolve(HERE, "../../dist");
export const FIXTURES_DIR = resolve(HERE, "../fixtures");

export interface ExtensionContext {
  context: BrowserContext;
  /** MV3 service-worker page — the runtime root. */
  worker: Worker;
  /** Reverse-engineered extension id (matches the SW url scheme). */
  extensionId: string;
  /** Open the popup as a regular tab. Chromium can't trigger the
   *  action click headlessly, so we navigate to popup/index.html.
   *  Verified-equivalent for our flows: the popup React app reads
   *  state from chrome.storage + chrome.runtime, which work in this
   *  mode. */
  openPopup(): Promise<Page>;
  /** Open the options page. */
  openOptions(): Promise<Page>;
  /** Convenience — close the context. Idempotent. */
  close(): Promise<void>;
}

export async function launchExtension(
  opts: {
    /** Override the persistent profile dir (else random tmpdir). */
    userDataDir?: string;
  } = {},
): Promise<ExtensionContext> {
  if (!existsSync(DIST_DIR)) {
    throw new Error(
      `Extension not built — expected ${DIST_DIR}. Run \`bun run build\` first.`,
    );
  }

  const userDataDir =
    opts.userDataDir ?? mkdtempSync(join(tmpdir(), "mnemonik-e2e-"));

  // `--load-extension` is a chromium-only flag; the persistent context
  // is required (regular `chromium.launch()` rejects extensions).
  // MV3 extensions don't run under classic --headless; we use
  // `headless: false` and rely on xvfb-run in CI (the workflow sets it
  // up explicitly). Local dev: a Chromium window flashes briefly.
  const context = await chromium.launchPersistentContext(userDataDir, {
    headless: false,
    args: [
      `--disable-extensions-except=${DIST_DIR}`,
      `--load-extension=${DIST_DIR}`,
      "--no-sandbox",
      // Reduce noise — disable GPU, network observers, and the
      // "first-run" overlays so the SW boots immediately.
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-gpu",
    ],
    viewport: { width: 1280, height: 720 },
  });

  // The MV3 service-worker registers asynchronously after install —
  // poll up to 5s for the first ServiceWorker.
  let worker = context.serviceWorkers()[0];
  if (!worker) {
    worker = await context.waitForEvent("serviceworker", { timeout: 5_000 });
  }
  const url = worker.url();
  // url shape: chrome-extension://<id>/<path>
  const match = /^chrome-extension:\/\/([a-z]{32})\//.exec(url);
  if (!match) throw new Error(`Could not parse extension id from ${url}`);
  const extensionId = match[1]!;

  const openPopup = async (): Promise<Page> => {
    const page = await context.newPage();
    await page.goto(`chrome-extension://${extensionId}/src/popup/index.html`);
    return page;
  };
  const openOptions = async (): Promise<Page> => {
    const page = await context.newPage();
    await page.goto(`chrome-extension://${extensionId}/src/options/index.html`);
    return page;
  };
  let closed = false;
  const close = async (): Promise<void> => {
    if (closed) return;
    closed = true;
    await context.close();
  };

  return { context, worker, extensionId, openPopup, openOptions, close };
}

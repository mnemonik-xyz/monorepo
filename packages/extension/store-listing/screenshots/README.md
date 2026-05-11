# Chrome Web Store — Screenshots

The store requires at least one screenshot and accepts up to five.
We ship the full five so the listing carousel feels complete.

| Slot | Filename                       | Size (px)    | Caption                                                     |
| ---- | ------------------------------ | ------------ | ----------------------------------------------------------- |
| 1    | `popup-capture.png`            | 1280 × 800   | One-click capture from ChatGPT, Claude or Gemini.           |
| 2    | `popup-recall.png`             | 1280 × 800   | Semantic recall over your local memories.                   |
| 3    | `fab-on-chatgpt.png`           | 1280 × 800   | Floating action button on ChatGPT.                          |
| 4    | `options-storage-panel.png`    | 1280 × 800   | Local / Cloud storage modes — switch any time.              |
| 5    | `restore-flow.png`             | 1280 × 800   | Restore your identity on a second device with one passphrase. |

## Status

**Deliverables — captured AFTER T19's build fix lands on `dev`.**

Reason: the screenshots show the real popup / overlay / options
surfaces, which require the production `dist/` bundle to exist. T19
owns the build-unblock step (placeholder→real icons + vite-typecheck
fix). Until that lands, `vite build` errors out and we can't load the
extension into Chrome to capture stable visuals.

## Capture recipe

Once T19 has merged:

1. `bun install && bun run -F @mnemonik-xyz/extension build`
2. Load the unpacked `packages/extension/dist/` directory in
   `chrome://extensions` with developer mode on.
3. Seed the local store with the demo fixtures from
   `packages/extension/tests/fixtures/seed/`:

   ```js
   // Run in the popup's devtools console.
   const seed = await fetch(chrome.runtime.getURL('tests/fixtures/seed/demo.json')).then(r => r.json());
   await chrome.runtime.sendMessage({ type: 'sw:seed-local-store', payload: seed });
   ```

4. Set the browser zoom to 100 % and the window to *exactly* 1 280 ×
   800 (DevTools → Toggle device toolbar → 1 280 × 800 preset).
5. Capture each surface listed above. Use the OS screenshot tool
   (macOS: `Cmd+Shift+5`) — no browser-chrome decorations, no
   reflections, no cursors.
6. Save the PNGs to this directory with the filenames above.
7. Compress with `pngquant --quality=85-95 *.png` if the resulting
   files exceed 1 MB each (Chrome Web Store rejects oversized PNGs).

## Style guide

- Use the brand-token Light mode (the popup palette in T11). Dark
  mode is fine but be consistent across all five.
- No real user content. The fixture set lives in
  `tests/fixtures/seed/demo.json` — public, anonymised,
  intentionally interesting.
- Avoid OS-level chrome (window controls, dock, taskbar). Crop tight.
- Annotate sparingly. If a callout is essential, use the brand teal
  (`#00D4B4`) at 4 px stroke, no drop shadow.

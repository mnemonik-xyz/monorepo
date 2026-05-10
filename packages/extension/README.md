# `@mnemonik-xyz/extension`

Mnemonik browser extension (Manifest V3). Captures AI-chat context from
ChatGPT, Claude.ai, and Gemini, signs locally via WASM, optionally syncs to
the hosted MCP-`full` server.

**Build tool:** Vite + `@crxjs/vite-plugin` (D4, ratified in T01 spike).

## Status

T01 scaffold only. Buildable empty extension. Domain code lands in T02–T20
(see `work/chrome-extension/tasks/`).

## Dev workflow

```bash
# install once at the monorepo root
npm install

# dev (HMR, manifest auto-rebuild on change)
npm -w @mnemonik-xyz/extension run dev

# production build → dist/
npm -w @mnemonik-xyz/extension run build

# tests (vitest, jsdom)
npm -w @mnemonik-xyz/extension test
```

`vite build` writes `dist/` with the rewritten MV3 manifest. Load it in
Chrome via **chrome://extensions → Developer mode → Load unpacked** and
pick `packages/extension/dist/`.

For background-script HMR with Vite + crxjs: keep `npm run dev` running;
crxjs writes a watch build into `dist/` and signals the Chrome runtime to
reload. Service-worker logs show in `chrome://extensions → service worker`.

## Layout

```
manifest.json            MV3 manifest (D11: enumerated host_permissions)
public/icons/            Placeholder 1x1 PNGs — replaced in T11
src/
  background/            Service worker entry
  popup/                 React + dark theme, Mnemonik tokens
  options/               Settings page
  content/               (T06–T09) per-platform ChatAdapter scripts
  runtime/               (T03–T05) store, embed, compress, sign, sync
  auth/                  (T14–T17) OAuth, key escrow
tests/
  unit/                  vitest — scaffold, store, embedder, signing
  e2e/                   (T19) Playwright with --load-extension
```

## Conventions

- Pure ESM (`"type": "module"`).
- React 19 + JSX; no Tailwind yet (added in T11 alongside design tokens).
- All Chrome APIs accessed through wrappers in `src/runtime/` so unit tests
  can run in jsdom without `chrome.*` globals.
- TS strict + `noUncheckedIndexedAccess`; no `any` outside generated types.

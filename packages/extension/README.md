# `@mnemonik-xyz/extension`

Mnemonik Chrome extension (MV3). Captures AI-chat context (ChatGPT, Claude.ai, Gemini) and any web-page selection as verifiable memory, signs locally with Ed25519, stores in IndexedDB (Local mode) or syncs to managed Mnemonik infra (Cloud mode).

This package is the consumer-facing UI for the protocol. Domain logic (embed, compress, sign) reuses `@mnemonik-xyz/sdk` + `@mnemonic/core` WASM. See `work/chrome-extension/` for product spec and task breakdown.

## Status

T01 (this commit): scaffolding only. The extension builds, loads in Chrome, shows a placeholder popup. Real capture / recall / auth flows arrive in T02–T17 per `work/chrome-extension/tasks/`.

## Develop

```bash
# from monorepo root, after `npm install`:
npm run -w @mnemonik-xyz/extension dev          # vite dev server + HMR
npm run -w @mnemonik-xyz/extension build        # build to packages/extension/dist
npm run -w @mnemonik-xyz/extension test         # vitest unit tests
npm run -w @mnemonik-xyz/extension lint         # tsc --noEmit
npm run -w @mnemonik-xyz/extension lint:webext  # web-ext lint dist/
```

Load `packages/extension/dist/` as an unpacked extension at `chrome://extensions`.

## Layout

```
manifest.json                 # MV3 manifest (single source of truth)
src/
  background/
    service-worker.ts         # MV3 background entry. T01 stub; T10 dispatches messages.
  popup/
    index.html · main.tsx     # Browser action popup. T07 builds the real UI.
    Popup.tsx
  options/
    index.html · main.tsx     # Options page. T08 builds the real UI.
    Options.tsx
  assets/                     # Icons (T17 ships the final set; placeholders for now).
tests/
  unit/scaffold.test.ts       # T01 TDD anchor: manifest validation.
vite.config.ts                # Vite + @crxjs/vite-plugin (D4).
vitest.config.ts              # vitest, environment: "node".
tsconfig.json                 # strict TS, matches packages/sdk conventions.
```

## Decisions

See `work/chrome-extension/decisions.md`. The most relevant for this package:

- **D1** — Local mode is fully self-contained; embedding/signing/storage all run in the browser.
- **D4 (ratified, T01)** — Vite 6 + `@crxjs/vite-plugin`. Matches webapp toolchain.
- **D11** — Enumerated `host_permissions` (no `<all_urls>`). T07–T09 add specific platforms.
- **D13** — A task is not `done` until its TDD-anchor tests + `verify:` commands pass in CI.

## Test plan (per T01)

- `tests/unit/scaffold.test.ts::dist_has_valid_manifest` — asserts the manifest is MV3, declares the expected permission set, registers popup + options + service-worker entries, registers commands.

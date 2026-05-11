# Extension icons

Real branded Mnemonik marks at the four sizes required by Chrome MV3:

| File           | Size      | Usage                                              |
| -------------- | --------- | -------------------------------------------------- |
| `icon-16.png`  | 16 × 16   | `action.default_icon` (toolbar)                    |
| `icon-32.png`  | 32 × 32   | `action.default_icon` (HiDPI toolbar)              |
| `icon-48.png`  | 48 × 48   | Extension management page (`chrome://extensions`)  |
| `icon-128.png` | 128 × 128 | Chrome Web Store listing + install dialog          |

**Status:** real branded icons (not placeholders). These overwrite any
placeholders shipped by sibling tasks (e.g. T19) — see the merge note in T20's
PR body. The mark is a stylised `M` on the brand background:

- background: `#0A0F1E` (deep navy, rounded square)
- glyph: `#00D4B4` (mnemonik teal)

**Source of truth:** `scripts/gen-icons.mjs` at the repo root. Re-run to
regenerate after any brand-token change:

```bash
node scripts/gen-icons.mjs
```

The script uses only Node stdlib (`zlib`, `Buffer`) — no `sharp` / `jimp` /
`canvas` install required. If a designer ships portfolio-grade artwork later,
replace these PNGs (and either retire or update the generator).

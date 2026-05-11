# Chrome Web Store — Promotional images

The Chrome Web Store accepts three promo image sizes. All are
*optional*, but the marquee is required for placement in featured
collections, so we ship all three.

| File                       | Size (px)    | Where it appears                           |
| -------------------------- | ------------ | ------------------------------------------ |
| `small-440x280.png`        | 440 × 280    | Search results, category browse tiles      |
| `large-920x680.png`        | 920 × 680    | Featured row on the Chrome Web Store home  |
| `marquee-1400x560.png`     | 1400 × 560   | Editor-picked marquee on category pages    |

## Status

**Deliverables — pending visual designer.**

The PNGs are not committed to the repo yet. They are produced from a
working extension build (Local mode popup against a seeded dataset)
and require a designer-touched composition that goes beyond what a
generated script should ship. T20 ships the spec; the designer ships
the bytes.

## Spec (hand-off to the designer)

Background: solid `#0A0F1E` (matches the icon background and the
webapp landing-page hero).

Accent (foreground glyph + key text): `#00D4B4`.

Body text on dark: `#E6EAF2` at 90 % opacity (matches webapp
`--color-text-primary`).

Glyph: the same `M` mark used in `src/assets/icon-128.png` — the
designer should source from the Figma file rather than re-tracing
the bitmap. Keep the rounded-square outer shape; the marquee can
allow the glyph to extend partially off-frame for movement.

Headline copy:

- Small (440 × 280): glyph + product name only, no headline.
- Large (920 × 680): "Verifiable AI memory" — two lines.
- Marquee (1400 × 560): "Verifiable AI memory across every chat" —
  one line, glyph anchored left.

Sub-line on the marquee: "ChatGPT · Claude · Gemini · Local or
Cloud — your choice."

Safe area: at least 64 px margin on every edge.

Export: PNG-24, sRGB, no transparency (Chrome Web Store rejects
transparent promo backgrounds). File size < 2 MB each.

## Capture recipe (for reference)

If the designer asks "what does the current product actually look
like?", point them at:

- `packages/extension/src/popup/App.tsx` — popup layout
- `packages/extension/src/content/recall-overlay.ts` — page-injected
  recall surface
- `webapp/src/components/IdentityPanel.tsx` — webapp brand reference
- `webapp/public/logo-512.png` — current 512 × 512 master logo

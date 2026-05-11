# Mnemonik — Chrome Web Store release checklist (v1.0.0)

End-to-end submission steps for the first public release. The build
itself is reproducible from `packages/extension` — this document is
the *manual* surface that surrounds it: developer-dashboard clicks,
OAuth verification with Google, and the bits the Chrome Web Store
asks for that don't live in the repo.

Run through this top-to-bottom on submission day. Tick each box in
the PR description before tagging `v1.0.0`.

---

## 0. Pre-flight (machine-side)

```bash
# Clean re-install.
bun install
bun run -F @mnemonik-xyz/extension build

# Static lint of the produced zip.
bunx web-ext lint --source-dir packages/extension/dist

# Package for the store. Produces mnemonik-1.0.0.zip in web-store/.
bunx web-ext build \
  --source-dir packages/extension/dist \
  --artifacts-dir packages/extension/web-store \
  --filename "mnemonik-{version}.zip"

# Dry-run upload (optional — needs a service account; skip in CI).
bunx chrome-webstore-upload-cli upload \
  --source packages/extension/web-store/mnemonik-1.0.0.zip \
  --extension-id "$CWS_EXTENSION_ID" \
  --client-id "$CWS_CLIENT_ID" \
  --client-secret "$CWS_CLIENT_SECRET" \
  --refresh-token "$CWS_REFRESH_TOKEN" \
  --auto-publish=false
```

> The `web-ext build` + `web-ext lint` steps depend on T19's
> build-unblock landing on `dev` (placeholder→real icons swap +
> vite-typecheck fix). They are gated on T19's PR being merged.

Also produce a **source-code zip** for the Chrome Web Store source
review — required when any JS/WASM in the bundle is minified:

```bash
git archive --format=zip --output packages/extension/web-store/source-1.0.0.zip HEAD -- \
  packages/extension webapp work/chrome-extension scripts/gen-icons.mjs \
  README.md LICENSE
```

The zip must contain enough source to rebuild the submitted bundle.
We ship the entire workspace so the reviewer can `bun install &&
bun run -F @mnemonik-xyz/extension build` and verify the artifact
hashes.

---

## 1. Developer dashboard — first-time setup

- [ ] Pay the one-time **$5 registration fee** (USD) at
      `https://chrome.google.com/webstore/devconsole/registration`.
      Use a corporate card if available so the receipt lands in
      finance.
- [ ] Set the **publisher name** to "Mnemonik".
- [ ] Configure **two-factor auth** on the Google account that owns
      the listing. Without it, the dashboard refuses uploads.
- [ ] Add a backup owner (a second teammate's Google account) under
      *Account → Group publishers*. Losing access to a single Google
      account otherwise locks the listing.

## 2. OAuth verification with Google

The extension's `identity.launchWebAuthFlow` flow requires a verified
OAuth consent screen on the **same Google Cloud project** that issues
the `id_token`. The current project id is `mnemonik-xyz`.

- [ ] In Google Cloud → APIs & Services → OAuth consent screen, set
      *Publishing status: In production*.
- [ ] Brand: `Mnemonik` (icon: `packages/extension/src/assets/icon-128.png`).
- [ ] Authorized domains: `mnemonik.xyz`.
- [ ] Scopes requested: `openid`, `email`, `profile`. Justify each:
        - `openid`: receive a Google-signed `id_token` we can verify
          locally to bind a `sub` to the user's account.
        - `email`: display the signed-in user's email in the popup so
          they can confirm they signed in with the right account.
        - `profile`: display the signed-in user's name + avatar in the
          popup header for the same reason.
- [ ] Submit for **verification**. Reviewer turn-around is typically
      4–6 weeks; start this before the Chrome Web Store submission.
- [ ] Verification needs the privacy-policy URL (`https://mnemonik.xyz/extension/privacy`)
      and a YouTube video of the consent screen UX.

## 3. Listing fields (paste from `store-listing/`)

- [ ] Title: `Mnemonik — Verifiable AI Memory`
- [ ] Short description (132 char max): from
      `store-listing/description.md` → "Short description" section.
- [ ] Detailed description (16 000 char max): from
      `store-listing/description.md` → "Detailed description" section.
- [ ] Category: **Productivity** (single).
- [ ] Language: `English (United States)`. Translations are post-MVP.
- [ ] Permissions justification: paste each row from
      `store-listing/permissions-justification.md` into the matching
      box in *Privacy practices → Single purpose / Permission justifications*.
- [ ] Privacy policy URL: `https://mnemonik.xyz/extension/privacy`.
- [ ] Homepage URL: `https://mnemonik.xyz`.
- [ ] Support URL: `https://github.com/mnemonik-xyz/monorepo/issues`.

## 4. Visual assets

Upload from `store-listing/`. Required dimensions:

- [ ] Icon: `src/assets/icon-128.png` (128 × 128, embedded in the ZIP).
- [ ] **Small promo tile**: `store-listing/promo/small-440x280.png`.
- [ ] **Large promo tile**: `store-listing/promo/large-920x680.png`.
- [ ] **Marquee promo**: `store-listing/promo/marquee-1400x560.png`.
- [ ] **Screenshots** (1280 × 800, 5 total) from
      `store-listing/screenshots/`:
        1. popup-capture
        2. popup-recall
        3. fab-on-chatgpt
        4. options-storage-panel
        5. restore-flow

> Promo images and screenshots are designer deliverables produced
> from a working extension build, i.e. *after* T19's build fix lands
> and CI emits a `dist/` we can drive Playwright against. See
> `store-listing/promo/README.md` and
> `store-listing/screenshots/README.md` for capture specs.

## 5. Pricing & distribution

- [ ] Visibility: **Public**.
- [ ] Distribution: **All regions** except those blocked by Google's
      default sanctions list.
- [ ] Pricing: **Free** (Local mode). Cloud mode is billed *outside*
      the Chrome Web Store via x402 / Stripe and does not affect this
      flag.

## 6. Privacy practices block

- [ ] *Single purpose*: "Capture conversations from ChatGPT, Claude
      and Gemini, plus any web selection, as locally-signed verifiable
      memory; optionally sync to managed Mnemonik infrastructure."
- [ ] Permissions justifications: copied from §3.
- [ ] *Data usage* — tick the boxes for the categories we actually
      collect (Authentication info, Personally identifiable information,
      User-generated content). Untick everything else.
- [ ] Declare: "Data is encrypted in transit" — Y.
- [ ] Declare: "I do not sell or transfer user data to third
      parties..." — Y.
- [ ] Declare: "I do not use or transfer user data for purposes
      unrelated to my item's single purpose" — Y.
- [ ] Declare: "I do not use or transfer user data to determine
      creditworthiness or for lending purposes" — Y.

## 7. Submit

- [ ] Click **Submit for review**. Typical Chrome Web Store turn-around
      is 1–5 business days.
- [ ] Tag the release: `git tag -s v1.0.0 -m "v1.0.0 — first public
      Chrome Web Store release" && git push --tags`.
- [ ] Open a GitHub release with the source-zip + the changelog
      snippet for v1.0.0.
- [ ] Once approved, paste the listing URL into
      `webapp/src/components/LandingPage.tsx` (the "Install Chrome
      extension" CTA) and ship a webapp deploy.

## 8. Post-launch monitoring (first week)

- [ ] Watch the developer dashboard for the daily install/uninstall
      counter.
- [ ] Subscribe the team channel to `support@mnemonik.xyz` for first
      week of bug reports.
- [ ] If a critical bug surfaces, hotfix on `dev`, bump
      `manifest.json` to `1.0.1`, repeat §0–§7. Chrome Web Store will
      auto-update users within ~24 h.

---

**Owners:** the engineer cutting the release tag is on point for
§0–§5 and §7; the OAuth verification (§2) is owned by whoever holds
the `mnemonik-xyz` Google Cloud project IAM admin role.

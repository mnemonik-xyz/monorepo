// T19 — Stub. Tracked as follow-up after T19 partial-ship lands.
// Profile starts in Local with 3 IDB attestations, no identity. The
// user toggles Storage tier to Cloud → Onboarding fires → mocked
// Google sign-in (via mocks/google-oauth.ts + mocks/server.ts) → set
// passphrase → backfill drain progress bar reaches 100% → assert all
// 3 IDB rows acquired non-`local:` tx ids.

import { test } from "@playwright/test";

test.skip("local_attestations_backfill_to_cloud_after_mode_switch", () => {
  // TODO(T19-follow-up): wire the OAuth + lookup mocks so the
  // Onboarding state machine reaches `set_passphrase`, then assert
  // the progress UI and post-drain row state.
});

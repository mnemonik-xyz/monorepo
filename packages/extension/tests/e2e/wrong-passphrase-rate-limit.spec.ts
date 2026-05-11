// T19 — Stub. Tracked as follow-up after T19 partial-ship lands.
// Different shape from the `wrong_passphrase_5_attempts_blocks_input`
// test in restore-on-second-profile.spec.ts: here the SERVER rate-
// limits escrow GETs after 5 calls and returns Retry-After. The popup
// must surface the countdown without consuming the local 5-attempt
// budget. Wire the mocks/server.ts `escrowGetRateLimitAfter` knob.

import { test } from "@playwright/test";

test.skip("server_429_after_5_gets_renders_retry_after_countdown", () => {
  // TODO(T19-follow-up): seed mockServer with
  // `escrowGetRateLimitAfter: 5` and assert the Restore UI surfaces
  // the rate-limit countdown rather than the wrong-passphrase one.
});

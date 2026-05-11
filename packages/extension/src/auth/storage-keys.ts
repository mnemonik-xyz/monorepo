// Single source of truth for chrome.storage.local identity keys.
//
// Pre-T25-round-2 these literals were duplicated across three sites:
//   - `options/runtime-impl.ts` exported `IDENTITY_KEY` / `IDENTITY_SECRET_KEY`.
//   - `popup/onboarding/Restore.tsx` exported `IDENTITY_STORAGE_KEY` /
//     `IDENTITY_SECRET_STORAGE_KEY`.
//   - `popup/Onboarding.tsx::mintAndPersistKeypair` used bare string
//     literals `"identity"` / `"identity_secret"`.
//
// All three sites happened to agree on the literal value `"identity"` /
// `"identity_secret"`, but a future rename would only touch one site
// and silently break recovery (the popup writes one key, the options
// reader looks at another). Code-review finding PR136-C-01 + audit
// finding SA-T25-003.
//
// This module lives under `src/auth/` because both the options realm
// and the popup realm already depend on `src/auth/`, so importing from
// here introduces no new cross-realm coupling. The constants are
// re-exported from their previous homes for callers that still rely on
// the old names — those re-exports are mechanical and can be removed
// once every import site is migrated.

/** chrome.storage.local key for the public identity metadata
 *  (`{pubkey_base58, created_at?}`). Read by both the popup runtime
 *  and the options identity facade. */
export const IDENTITY_KEY = "identity";

/** chrome.storage.local key for the 64-byte Solana keypair secret
 *  (seed||pubkey) stored as a structured-clone-safe `number[]`.
 *  Never logged. */
export const IDENTITY_SECRET_KEY = "identity_secret";

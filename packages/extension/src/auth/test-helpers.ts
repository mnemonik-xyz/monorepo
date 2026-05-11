// E2E TEST HELPER — Re-exports the `key-escrow` symbols the Playwright
// harness needs to dynamic-import from a compiled chunk. Production
// code paths route through the popup runtime facade (`runtime.ts`),
// which only ever accesses `keyEscrow.<method>` on the facade object.
// As a result, rollup tree-shakes the bare named exports of
// `key-escrow.ts` (`wrapSecret`, `unwrapSecret`, `uploadEscrow`) from
// the production chunk. This file is wired as a separate Vite input so
// it gets its own hashed chunk that retains the full export surface;
// `restore-on-second-profile.spec.ts` dynamic-imports it by URL inside
// `popup.evaluate(...)`.
//
// IMPORTANT: This file is for tests only. Do NOT import it from
// production source — doing so would still tree-shake the bare exports
// out of the production chunk because rollup analyses graph reachability
// per export, not per module.

export {
  deleteEscrow,
  EscrowNotFoundError,
  EscrowPubkeyMismatchError,
  EscrowRateLimitError,
  fetchEscrow,
  rotatePassphrase,
  unwrapSecret,
  uploadEscrow,
  wrapSecret,
  WrongPassphraseError,
} from "./key-escrow.js";
export type { EscrowBlob, EscrowHttpOptions } from "./key-escrow.js";

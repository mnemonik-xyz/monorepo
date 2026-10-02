# Clean candidate build and installed recovery

Run this from the repository root to build SDK/CLI tarballs from the current
committed source and exercise their installed contents:

```sh
node scripts/check-built-candidate.mjs
```

Required tools are Node 20+, npm, Rust/Cargo with the WASM target, wasm-pack and
the repository's native MCP test dependencies. Optional `wasm-opt` changes the
WASM output; its presence/version is recorded. The build needs temporary disk
space and may download dependencies. This is a source-candidate check, not npm
publication or permission to deploy.

## What runs

The command archives `HEAD` into a fresh temporary source directory. It never
copies ignored build outputs or workspace `node_modules`. It installs the root
npm lockfile, builds the SDK (including WASM) and then CLI with a separate Cargo
target directory. Source lockfiles must remain unchanged.

Both tarballs are installed in another disposable directory outside the source
checkout. Every installed package file is compared with the unpacked tarball.
The CLI must resolve the installed candidate SDK. Node resolution/loader and
Rust compiler/toolchain override environment variables are cleared for the
probe; declared repository toolchain configuration still applies.

A fresh standalone CLI identity saves sealed local content and opens it in a
new process with HTTP fetch rejected. This fetch guard is not a system network
sandbox. Next, the real SDK/WASM research handoff runs using the installed SDK:
backup restore, exact-byte storage migration, source/operator shutdown, fresh
reviewer continuation through O2 and the local failure controls. The combined
runner also executes the separate seeded-payment fault tests.

The copied SDK test helpers and synthetic fixture are harness additions beside
the installed package, not shipped package contents. Both Node phases use those
helpers' relative imports into the installed `dist`. Rust operators remain the
local source test harness. Shipped package file hashes are checked again after
the drill; test helpers cannot silently modify the candidate being measured.

## Evidence and failure handling

`target/built-candidate/report.json` records:

- Committed source/tree and archive hashes, source lockfile hashes and tool versions.
- Tarball hashes/integrity and hashes of installed package files, including WASM.
- Installed dependency versions/integrities and the installation lockfile hash.
- Harness file hashes and the combined recovery report hash.
- Observed checks and an explicit `running`, `failed` or `passed` status.

Build and test logs are in the same output directory. The combined viewer is
`target/built-candidate/recovery/report.html`. Check the top-level status before
using any nested report: a failed build can leave an earlier nested report as
historical evidence. Each new attempt invalidates the top-level success record.
Temporary build, installation and identity directories are removed afterward.
Long-running commands have a 15-minute timeout and terminate their process group
on interruption on POSIX systems. An abrupt host shutdown still needs ordinary
temporary-directory cleanup.

Runtime source and HEAD are checked before building, before the Rust drill and
afterward. Test inputs are hashed separately and must remain unchanged during
the drill. One clean build with recorded tools is provenance evidence; it is not
a hermetic or independently reproduced build claim.

## Remaining release boundary

These are unpublished source candidates. The test uses local synthetic storage,
indexes and operators; it does not establish live-provider visibility, retention,
paid settlement, production configuration, published MCP compatibility or rollback.
A-17 still requires pinned released artifacts and its operational acceptance.
The [pilot runbook](../work/protocol-product/pilot-runbook.md) describes those
remaining steps. Use the [read-only live probe](live-observation-probe.md) only
with independently pinned real fixture identifiers and submission observations.
